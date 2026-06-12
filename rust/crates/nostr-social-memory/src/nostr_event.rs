use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, TimeZone, Utc};
use nostr::prelude::*;
use serde_json::json;

use crate::attestation::Attestation;
use crate::counter_attestation::CounterAttestation;
use crate::rating::{Rating, ReportType, Sentiment};

pub const ATTESTATION_KIND: u16 = 31985;
pub const RATING_KIND: u16 = 31986;
pub const COUNTER_ATTESTATION_KIND: u16 = 31987;

/// Verify a nostr event's id and signature.
pub fn verify_event(event: &Event) -> Result<()> {
    event
        .verify()
        .map_err(|e| anyhow!("event verification failed: {e}"))
}

impl Attestation {
    /// Sign this attestation as a nostr event.
    pub fn to_event(&self, keys: &Keys) -> Result<Event> {
        let mut content = serde_json::Map::new();
        content.insert("attributes".into(), json!(self.attributes));
        if let Some(ctx) = &self.context {
            content.insert("context".into(), json!(ctx));
        }
        if let Some(ended) = &self.ended_at {
            content.insert("ended_at".into(), json!(ended.to_rfc3339()));
        }

        let created_at = Timestamp::from(self.created_at.timestamp() as u64);

        let mut builder = EventBuilder::new(
            Kind::from(ATTESTATION_KIND),
            serde_json::to_string(&content)?,
        )
        .custom_created_at(created_at)
        .tag(Tag::identifier(&self.id));

        for attr in &self.attributes {
            builder = builder.tag(Tag::custom(
                TagKind::Custom("attr".into()),
                vec![attr.clone()],
            ));
        }

        builder
            .sign_with_keys(keys)
            .context("failed to sign attestation event")
    }

    /// Parse an attestation from a nostr event.
    pub fn from_event(event: &Event) -> Result<Attestation> {
        if event.kind != Kind::from(ATTESTATION_KIND) {
            bail!(
                "wrong event kind: expected {}, got {:?}",
                ATTESTATION_KIND,
                event.kind
            );
        }

        let id = event
            .tags
            .identifier()
            .ok_or_else(|| anyhow!("missing d tag"))?
            .to_string();

        let content: serde_json::Value =
            serde_json::from_str(&event.content).context("invalid content JSON")?;

        let attributes: Vec<String> = content
            .get("attributes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let context = content
            .get("context")
            .and_then(|v| v.as_str())
            .map(String::from);

        let ended_at = content
            .get("ended_at")
            .and_then(|v| v.as_str())
            .map(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .context("invalid ended_at timestamp")
            })
            .transpose()?;

        let created_at = Utc
            .timestamp_opt(event.created_at.as_u64() as i64, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid created_at timestamp"))?;

        Ok(Attestation {
            id,
            attester: event.pubkey.to_hex(),
            attributes,
            context,
            created_at,
            ended_at,
        })
    }
}

impl CounterAttestation {
    /// Sign this counter-attestation as a nostr event.
    pub fn to_event(&self, keys: &Keys) -> Result<Event> {
        let mut content = serde_json::Map::new();
        content.insert("attributes".into(), json!(self.attributes));
        if let Some(ctx) = &self.context {
            content.insert("context".into(), json!(ctx));
        }

        let created_at = Timestamp::from(self.created_at.timestamp() as u64);

        let mut builder = EventBuilder::new(
            Kind::from(COUNTER_ATTESTATION_KIND),
            serde_json::to_string(&content)?,
        )
        .custom_created_at(created_at)
        .tag(Tag::identifier(&self.id));

        if let Some(ref event_id) = self.disputed_event_id {
            builder = builder.tag(Tag::event(
                EventId::from_hex(event_id).context("invalid disputed_event_id hex")?,
            ));
        }

        for attr in &self.attributes {
            builder = builder.tag(Tag::custom(
                TagKind::Custom("attr".into()),
                vec![attr.clone()],
            ));
        }

        builder
            .sign_with_keys(keys)
            .context("failed to sign counter-attestation event")
    }

    /// Parse a counter-attestation from a nostr event.
    pub fn from_event(event: &Event) -> Result<CounterAttestation> {
        if event.kind != Kind::from(COUNTER_ATTESTATION_KIND) {
            bail!(
                "wrong event kind: expected {}, got {:?}",
                COUNTER_ATTESTATION_KIND,
                event.kind
            );
        }

        let id = event
            .tags
            .identifier()
            .ok_or_else(|| anyhow!("missing d tag"))?
            .to_string();

        let content: serde_json::Value =
            serde_json::from_str(&event.content).context("invalid content JSON")?;

        let attributes: Vec<String> = content
            .get("attributes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let context = content
            .get("context")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Extract the "e" tag for disputed_event_id
        let disputed_event_id = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::E)))
            .and_then(|t| t.content())
            .map(|s| s.to_string());

        let created_at = Utc
            .timestamp_opt(event.created_at.as_u64() as i64, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid created_at timestamp"))?;

        Ok(CounterAttestation {
            id,
            attester: event.pubkey.to_hex(),
            attributes,
            disputed_event_id,
            context,
            created_at,
        })
    }
}

impl Rating {
    /// Sign this rating as a nostr event.
    pub fn to_event(&self, keys: &Keys) -> Result<Event> {
        let sentiment_str = match self.sentiment {
            Sentiment::Positive => "positive",
            Sentiment::Negative => "negative",
            Sentiment::Neutral => "neutral",
        };

        let mut content = serde_json::Map::new();
        content.insert("subject".into(), json!(self.subject));
        content.insert("sentiment".into(), json!(sentiment_str));
        if let Some(ctx) = &self.context {
            content.insert("context".into(), json!(ctx));
        }
        if !self.tags.is_empty() {
            content.insert("tags".into(), json!(self.tags));
        }
        if let Some(rt) = &self.report_type {
            let rt_str = match rt {
                ReportType::Nudity => "nudity",
                ReportType::Malware => "malware",
                ReportType::Profanity => "profanity",
                ReportType::Illegal => "illegal",
                ReportType::Spam => "spam",
                ReportType::Impersonation => "impersonation",
            };
            content.insert("report_type".into(), json!(rt_str));
        }

        let created_at = Timestamp::from(self.created_at.timestamp() as u64);

        let mut builder =
            EventBuilder::new(Kind::from(RATING_KIND), serde_json::to_string(&content)?)
                .custom_created_at(created_at)
                .tag(Tag::identifier(&self.id))
                .tag(Tag::public_key(
                    PublicKey::from_hex(&self.subject).unwrap_or_else(|_| {
                        // subject might not be a valid pubkey hex; store as custom p tag
                        Keys::generate().public_key()
                    }),
                ))
                .tag(Tag::custom(
                    TagKind::Custom("sentiment".into()),
                    vec![sentiment_str.to_string()],
                ));

        for tag in &self.tags {
            builder = builder.tag(Tag::hashtag(tag));
        }

        if let Some(rt) = &self.report_type {
            let rt_str = match rt {
                ReportType::Nudity => "nudity",
                ReportType::Malware => "malware",
                ReportType::Profanity => "profanity",
                ReportType::Illegal => "illegal",
                ReportType::Spam => "spam",
                ReportType::Impersonation => "impersonation",
            };
            builder = builder.tag(Tag::custom(
                TagKind::Custom("report".into()),
                vec![rt_str.to_string()],
            ));
        }

        builder
            .sign_with_keys(keys)
            .context("failed to sign rating event")
    }

    /// Parse a rating from a nostr event.
    pub fn from_event(event: &Event) -> Result<Rating> {
        if event.kind != Kind::from(RATING_KIND) {
            bail!(
                "wrong event kind: expected {}, got {:?}",
                RATING_KIND,
                event.kind
            );
        }

        let id = event
            .tags
            .identifier()
            .ok_or_else(|| anyhow!("missing d tag"))?
            .to_string();

        let content: serde_json::Value =
            serde_json::from_str(&event.content).context("invalid content JSON")?;

        let subject = content
            .get("subject")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing subject in content"))?
            .to_string();

        let sentiment_str = content
            .get("sentiment")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing sentiment in content"))?;
        let sentiment = match sentiment_str {
            "positive" => Sentiment::Positive,
            "negative" => Sentiment::Negative,
            "neutral" => Sentiment::Neutral,
            other => bail!("unknown sentiment: {other}"),
        };

        let context = content
            .get("context")
            .and_then(|v| v.as_str())
            .map(String::from);

        let tags: Vec<String> = content
            .get("tags")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let report_type = content
            .get("report_type")
            .and_then(|v| v.as_str())
            .map(|s| match s {
                "nudity" => Ok(ReportType::Nudity),
                "malware" => Ok(ReportType::Malware),
                "profanity" => Ok(ReportType::Profanity),
                "illegal" => Ok(ReportType::Illegal),
                "spam" => Ok(ReportType::Spam),
                "impersonation" => Ok(ReportType::Impersonation),
                other => Err(anyhow!("unknown report type: {other}")),
            })
            .transpose()?;

        let created_at = Utc
            .timestamp_opt(event.created_at.as_u64() as i64, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid created_at timestamp"))?;

        Ok(Rating {
            id,
            rater: event.pubkey.to_hex(),
            subject,
            sentiment,
            context,
            tags,
            report_type,
            created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attestation_roundtrip() {
        let keys = Keys::generate();
        let a = Attestation::new(
            keys.public_key().to_hex(),
            vec!["npub1a".into(), "npub1b".into()],
        );

        let event = a.to_event(&keys).unwrap();
        assert_eq!(event.kind, Kind::from(ATTESTATION_KIND));
        verify_event(&event).unwrap();

        let parsed = Attestation::from_event(&event).unwrap();
        assert_eq!(parsed.id, a.id);
        assert_eq!(parsed.attester, keys.public_key().to_hex());
        assert_eq!(parsed.attributes, a.attributes);
        assert!(parsed.context.is_none());
        assert!(parsed.ended_at.is_none());
        assert_eq!(parsed.created_at.timestamp(), a.created_at.timestamp());
    }

    #[test]
    fn attestation_roundtrip_all_fields() {
        let keys = Keys::generate();
        let mut a = Attestation::new(
            keys.public_key().to_hex(),
            vec!["npub1x".into(), "entity-uuid".into(), "npub1y".into()],
        );
        a.context = Some("verified at conference".into());
        a.ended_at = Some(Utc::now());

        let event = a.to_event(&keys).unwrap();
        let parsed = Attestation::from_event(&event).unwrap();

        assert_eq!(parsed.attributes, a.attributes);
        assert_eq!(parsed.context, a.context);
        assert!(parsed.ended_at.is_some());
        assert_eq!(
            parsed.ended_at.unwrap().timestamp(),
            a.ended_at.unwrap().timestamp()
        );
    }

    #[test]
    fn attestation_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&keys)
            .unwrap();

        let err = Attestation::from_event(&event).unwrap_err();
        assert!(err.to_string().contains("wrong event kind"));
    }

    #[test]
    fn rating_roundtrip_positive() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let mut r = Rating::new(
            keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            Sentiment::Positive,
        );
        r.context = Some("helpful dev".into());
        r.tags = vec!["rust".into(), "nostr".into()];

        let event = r.to_event(&keys).unwrap();
        assert_eq!(event.kind, Kind::from(RATING_KIND));
        verify_event(&event).unwrap();

        let parsed = Rating::from_event(&event).unwrap();
        assert_eq!(parsed.id, r.id);
        assert_eq!(parsed.rater, keys.public_key().to_hex());
        assert_eq!(parsed.subject, subject_keys.public_key().to_hex());
        assert_eq!(parsed.sentiment, Sentiment::Positive);
        assert_eq!(parsed.context, Some("helpful dev".into()));
        assert_eq!(parsed.tags, vec!["rust", "nostr"]);
        assert!(parsed.report_type.is_none());
        assert_eq!(parsed.created_at.timestamp(), r.created_at.timestamp());
    }

    #[test]
    fn rating_roundtrip_negative_with_report() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let mut r = Rating::new(
            keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            Sentiment::Negative,
        );
        r.report_type = Some(ReportType::Spam);
        r.context = Some("unsolicited DMs".into());

        let event = r.to_event(&keys).unwrap();
        let parsed = Rating::from_event(&event).unwrap();

        assert_eq!(parsed.sentiment, Sentiment::Negative);
        assert_eq!(parsed.report_type, Some(ReportType::Spam));
        assert_eq!(parsed.context, Some("unsolicited DMs".into()));
    }

    #[test]
    fn rating_roundtrip_neutral() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let r = Rating::new(
            keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            Sentiment::Neutral,
        );

        let event = r.to_event(&keys).unwrap();
        let parsed = Rating::from_event(&event).unwrap();
        assert_eq!(parsed.sentiment, Sentiment::Neutral);
    }

    #[test]
    fn rating_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&keys)
            .unwrap();

        let err = Rating::from_event(&event).unwrap_err();
        assert!(err.to_string().contains("wrong event kind"));
    }

    #[test]
    fn all_report_types_roundtrip_via_event() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let types = vec![
            ReportType::Nudity,
            ReportType::Malware,
            ReportType::Profanity,
            ReportType::Illegal,
            ReportType::Spam,
            ReportType::Impersonation,
        ];
        for rt in types {
            let mut r = Rating::new(
                keys.public_key().to_hex(),
                subject_keys.public_key().to_hex(),
                Sentiment::Negative,
            );
            r.report_type = Some(rt.clone());

            let event = r.to_event(&keys).unwrap();
            let parsed = Rating::from_event(&event).unwrap();
            assert_eq!(parsed.report_type, Some(rt));
        }
    }

    #[test]
    fn event_verification_passes() {
        let keys = Keys::generate();
        let a = Attestation::new(keys.public_key().to_hex(), vec!["npub1a".into()]);
        let event = a.to_event(&keys).unwrap();
        assert!(verify_event(&event).is_ok());
    }

    #[test]
    fn attestation_empty_attributes() {
        let keys = Keys::generate();
        let a = Attestation::new(keys.public_key().to_hex(), vec![]);

        let event = a.to_event(&keys).unwrap();
        let parsed = Attestation::from_event(&event).unwrap();
        assert!(parsed.attributes.is_empty());
    }

    #[test]
    fn rating_no_optional_fields() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let r = Rating::new(
            keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            Sentiment::Positive,
        );

        let event = r.to_event(&keys).unwrap();
        let parsed = Rating::from_event(&event).unwrap();
        assert!(parsed.context.is_none());
        assert!(parsed.tags.is_empty());
        assert!(parsed.report_type.is_none());
    }

    #[test]
    fn counter_attestation_roundtrip() {
        let keys = Keys::generate();
        let ca = CounterAttestation::new(
            keys.public_key().to_hex(),
            vec!["npub1a".into(), "npub1b".into()],
        );

        let event = ca.to_event(&keys).unwrap();
        assert_eq!(event.kind, Kind::from(COUNTER_ATTESTATION_KIND));
        verify_event(&event).unwrap();

        let parsed = CounterAttestation::from_event(&event).unwrap();
        assert_eq!(parsed.id, ca.id);
        assert_eq!(parsed.attester, keys.public_key().to_hex());
        assert_eq!(parsed.attributes, ca.attributes);
        assert!(parsed.context.is_none());
        assert!(parsed.disputed_event_id.is_none());
        assert_eq!(parsed.created_at.timestamp(), ca.created_at.timestamp());
    }

    #[test]
    fn counter_attestation_with_disputed_event_id() {
        let keys = Keys::generate();

        // Create a real attestation event to dispute
        let attestation = Attestation::new(
            keys.public_key().to_hex(),
            vec!["npub1a".into(), "npub1b".into()],
        );
        let att_event = attestation.to_event(&keys).unwrap();
        let att_event_id = att_event.id.to_hex();

        let mut ca = CounterAttestation::new(
            keys.public_key().to_hex(),
            vec!["npub1a".into(), "npub1b".into()],
        );
        ca.disputed_event_id = Some(att_event_id.clone());
        ca.context = Some("these are different people".into());

        let event = ca.to_event(&keys).unwrap();
        let parsed = CounterAttestation::from_event(&event).unwrap();

        assert_eq!(parsed.disputed_event_id, Some(att_event_id));
        assert_eq!(parsed.context, Some("these are different people".into()));
        assert_eq!(parsed.attributes, ca.attributes);
    }

    #[test]
    fn counter_attestation_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&keys)
            .unwrap();

        let err = CounterAttestation::from_event(&event).unwrap_err();
        assert!(err.to_string().contains("wrong event kind"));
    }

    #[test]
    fn counter_attestation_all_fields_survive() {
        let keys = Keys::generate();

        // Generate a valid event id for disputed_event_id
        let dummy_event = EventBuilder::new(Kind::TextNote, "dummy")
            .sign_with_keys(&keys)
            .unwrap();

        let mut ca = CounterAttestation::new(
            keys.public_key().to_hex(),
            vec!["npub1x".into(), "entity-uuid".into(), "npub1y".into()],
        );
        ca.context = Some("impersonation attempt".into());
        ca.disputed_event_id = Some(dummy_event.id.to_hex());

        let event = ca.to_event(&keys).unwrap();
        let parsed = CounterAttestation::from_event(&event).unwrap();

        assert_eq!(parsed.id, ca.id);
        assert_eq!(parsed.attester, keys.public_key().to_hex());
        assert_eq!(parsed.attributes, ca.attributes);
        assert_eq!(parsed.context, ca.context);
        assert_eq!(parsed.disputed_event_id, ca.disputed_event_id);
        assert_eq!(parsed.created_at.timestamp(), ca.created_at.timestamp());
    }

    #[test]
    fn counter_attestation_empty_attributes() {
        let keys = Keys::generate();
        let ca = CounterAttestation::new(keys.public_key().to_hex(), vec![]);

        let event = ca.to_event(&keys).unwrap();
        let parsed = CounterAttestation::from_event(&event).unwrap();
        assert!(parsed.attributes.is_empty());
    }
}
