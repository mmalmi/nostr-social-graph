use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, TimeZone, Utc};
use nostr_identity::{
    build_fact_op_event_with_links_and_identifiers, parse_fact_op_event, Fact, FactOp, FactOpLinks,
    FACT_OP_KIND,
};
use nostr_sdk::prelude::{Event, Keys};
use uuid::Uuid;

use crate::attestation::Attestation;
use crate::counter_attestation::CounterAttestation;
use crate::rating::{Rating, ReportType, Sentiment};

/// Social-memory records are encoded as shared fact operation events.
pub const ATTESTATION_KIND: u16 = FACT_OP_KIND;
pub const RATING_KIND: u16 = FACT_OP_KIND;
pub const COUNTER_ATTESTATION_KIND: u16 = FACT_OP_KIND;

const SCHEMA_VERSION: &str = "1";
const TYPE_ATTESTATION: &str = "social_memory_attestation";
const TYPE_RATING: &str = "social_memory_rating";
const TYPE_COUNTER_ATTESTATION: &str = "social_memory_counter_attestation";

/// Verify a nostr event's id and signature.
pub fn verify_event(event: &Event) -> Result<()> {
    event
        .verify()
        .map_err(|error| anyhow!("event verification failed: {error}"))
}

impl Attestation {
    /// Sign this attestation as a fact event.
    ///
    /// The event signer is the publisher/crawler. The attestation author is the
    /// `attester` fact, so imported data does not have to be self-authored.
    pub fn to_event(&self, keys: &Keys) -> Result<Event> {
        let mut facts = base_facts(TYPE_ATTESTATION, self.created_at)?;
        facts.push(Fact::new("attester", [self.attester.clone()]));
        facts.extend(
            self.attributes
                .iter()
                .cloned()
                .map(|attribute| Fact::new("attribute", [attribute])),
        );
        if let Some(context) = &self.context {
            facts.push(Fact::new("context", [context.clone()]));
        }
        if let Some(ended_at) = &self.ended_at {
            facts.push(Fact::new("ended_at", [ended_at.to_rfc3339()]));
        }

        build_fact_record_event(
            keys,
            &self.id,
            facts,
            FactOpLinks::default(),
            safe_identifiers(
                [&self.attester]
                    .into_iter()
                    .chain(self.attributes.iter())
                    .map(String::as_str),
            ),
        )
    }

    /// Parse an attestation from a fact event.
    pub fn from_event(event: &Event) -> Result<Attestation> {
        let op = parse_social_memory_fact(event, TYPE_ATTESTATION)?;
        Ok(Attestation {
            id: op.subject.to_string(),
            attester: required_scalar(&op, "attester")?,
            attributes: scalar_values(&op, "attribute")?,
            context: optional_scalar(&op, "context")?,
            created_at: required_datetime(&op, "created_at")?,
            ended_at: optional_datetime(&op, "ended_at")?,
        })
    }
}

impl CounterAttestation {
    /// Sign this counter-attestation as a fact event.
    ///
    /// The event signer is the publisher/crawler. The counter-attestation author
    /// is the `attester` fact.
    pub fn to_event(&self, keys: &Keys) -> Result<Event> {
        let mut facts = base_facts(TYPE_COUNTER_ATTESTATION, self.created_at)?;
        facts.push(Fact::new("attester", [self.attester.clone()]));
        facts.extend(
            self.attributes
                .iter()
                .cloned()
                .map(|attribute| Fact::new("attribute", [attribute])),
        );
        if let Some(disputed_event_id) = &self.disputed_event_id {
            facts.push(Fact::new("disputed_event_id", [disputed_event_id.clone()]));
        }
        if let Some(context) = &self.context {
            facts.push(Fact::new("context", [context.clone()]));
        }

        let links = FactOpLinks {
            dispute: self
                .disputed_event_id
                .iter()
                .map(|event_id| event_id.to_lowercase())
                .collect(),
            ..FactOpLinks::default()
        };

        build_fact_record_event(
            keys,
            &self.id,
            facts,
            links,
            safe_identifiers(
                [&self.attester]
                    .into_iter()
                    .chain(self.attributes.iter())
                    .map(String::as_str),
            ),
        )
    }

    /// Parse a counter-attestation from a fact event.
    pub fn from_event(event: &Event) -> Result<CounterAttestation> {
        let op = parse_social_memory_fact(event, TYPE_COUNTER_ATTESTATION)?;
        let disputed_event_id =
            optional_scalar(&op, "disputed_event_id")?.or_else(|| op.dispute.first().cloned());
        Ok(CounterAttestation {
            id: op.subject.to_string(),
            attester: required_scalar(&op, "attester")?,
            attributes: scalar_values(&op, "attribute")?,
            disputed_event_id,
            context: optional_scalar(&op, "context")?,
            created_at: required_datetime(&op, "created_at")?,
        })
    }
}

impl Rating {
    /// Sign this rating as a fact event.
    ///
    /// The event signer is the publisher/crawler. The rating author is the
    /// `rater` fact, so crawled reviews can preserve their external author.
    pub fn to_event(&self, keys: &Keys) -> Result<Event> {
        let mut facts = base_facts(TYPE_RATING, self.created_at)?;
        facts.push(Fact::new("rater", [self.rater.clone()]));
        facts.push(Fact::new("rating_subject", [self.subject.clone()]));
        facts.push(Fact::new("sentiment", [sentiment_str(&self.sentiment)]));
        if let Some(context) = &self.context {
            facts.push(Fact::new("context", [context.clone()]));
        }
        facts.extend(self.tags.iter().cloned().map(|tag| Fact::new("tag", [tag])));
        if let Some(report_type) = &self.report_type {
            facts.push(Fact::new("report_type", [report_type_str(report_type)]));
        }

        build_fact_record_event(
            keys,
            &self.id,
            facts,
            FactOpLinks::default(),
            safe_identifiers(
                [&self.rater, &self.subject]
                    .into_iter()
                    .chain(self.tags.iter())
                    .map(String::as_str),
            ),
        )
    }

    /// Parse a rating from a fact event.
    pub fn from_event(event: &Event) -> Result<Rating> {
        let op = parse_social_memory_fact(event, TYPE_RATING)?;
        Ok(Rating {
            id: op.subject.to_string(),
            rater: required_scalar(&op, "rater")?,
            subject: required_scalar(&op, "rating_subject")?,
            sentiment: parse_sentiment(&required_scalar(&op, "sentiment")?)?,
            context: optional_scalar(&op, "context")?,
            tags: scalar_values(&op, "tag")?,
            report_type: optional_scalar(&op, "report_type")?
                .map(|value| parse_report_type(&value))
                .transpose()?,
            created_at: required_datetime(&op, "created_at")?,
        })
    }
}

fn base_facts(record_type: &'static str, created_at: DateTime<Utc>) -> Result<Vec<Fact>> {
    Ok(vec![
        Fact::new("type", [record_type.to_owned()]),
        Fact::new("schema", [SCHEMA_VERSION.to_owned()]),
        Fact::new("created_at", [datetime_unix(created_at)?.to_string()]),
    ])
}

fn build_fact_record_event(
    keys: &Keys,
    id: &str,
    facts: Vec<Fact>,
    links: FactOpLinks,
    external_identifiers: Vec<String>,
) -> Result<Event> {
    let subject = record_subject(id)?;
    build_fact_op_event_with_links_and_identifiers(
        keys,
        subject,
        facts,
        links,
        external_identifiers,
        now_unix()?,
    )
}

fn parse_social_memory_fact(event: &Event, expected_type: &str) -> Result<FactOp> {
    let op = parse_fact_op_event(event)?;
    let record_type = required_scalar(&op, "type")?;
    if record_type != expected_type {
        bail!("unexpected social-memory fact event type {record_type}");
    }
    let schema = required_scalar(&op, "schema")?;
    if schema != SCHEMA_VERSION {
        bail!("unsupported social-memory fact schema {schema}");
    }
    Ok(op)
}

fn record_subject(id: &str) -> Result<Uuid> {
    Uuid::parse_str(id)
        .map_err(|error| anyhow!("social-memory fact record id must be a UUID: {id}: {error}"))
}

fn now_unix() -> Result<u64> {
    datetime_unix(Utc::now())
}

fn datetime_unix(value: DateTime<Utc>) -> Result<u64> {
    value
        .timestamp()
        .try_into()
        .map_err(|_| anyhow!("timestamp before Unix epoch is not supported: {value}"))
}

fn required_datetime(op: &FactOp, predicate: &str) -> Result<DateTime<Utc>> {
    parse_datetime(&required_scalar(op, predicate)?)
}

fn optional_datetime(op: &FactOp, predicate: &str) -> Result<Option<DateTime<Utc>>> {
    optional_scalar(op, predicate)?
        .map(|value| parse_datetime(&value))
        .transpose()
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>> {
    if let Ok(seconds) = value.parse::<i64>() {
        return Utc
            .timestamp_opt(seconds, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid Unix timestamp: {value}"));
    }
    DateTime::parse_from_rfc3339(value)
        .map(|datetime| datetime.with_timezone(&Utc))
        .map_err(|error| anyhow!("invalid timestamp {value}: {error}"))
}

fn required_scalar(op: &FactOp, predicate: &str) -> Result<String> {
    let values = scalar_values(op, predicate)?;
    match values.as_slice() {
        [value] => Ok(value.clone()),
        [] => bail!("missing social-memory fact {predicate}"),
        _ => bail!("social-memory fact {predicate} must appear once"),
    }
}

fn optional_scalar(op: &FactOp, predicate: &str) -> Result<Option<String>> {
    let values = scalar_values(op, predicate)?;
    match values.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(value.clone())),
        _ => bail!("social-memory fact {predicate} must appear once"),
    }
}

fn scalar_values(op: &FactOp, predicate: &str) -> Result<Vec<String>> {
    op.facts
        .iter()
        .filter(|fact| fact.predicate == predicate)
        .map(|fact| match fact.values.as_slice() {
            [value] => Ok(value.clone()),
            _ => bail!("social-memory fact {predicate} must be a scalar"),
        })
        .collect()
}

fn safe_identifiers<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    values
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_whitespace))
        .map(str::to_owned)
        .collect()
}

fn sentiment_str(sentiment: &Sentiment) -> String {
    match sentiment {
        Sentiment::Positive => "positive",
        Sentiment::Negative => "negative",
        Sentiment::Neutral => "neutral",
    }
    .to_owned()
}

fn parse_sentiment(value: &str) -> Result<Sentiment> {
    match value {
        "positive" => Ok(Sentiment::Positive),
        "negative" => Ok(Sentiment::Negative),
        "neutral" => Ok(Sentiment::Neutral),
        other => bail!("unknown sentiment: {other}"),
    }
}

fn report_type_str(report_type: &ReportType) -> String {
    match report_type {
        ReportType::Nudity => "nudity",
        ReportType::Malware => "malware",
        ReportType::Profanity => "profanity",
        ReportType::Illegal => "illegal",
        ReportType::Spam => "spam",
        ReportType::Impersonation => "impersonation",
    }
    .to_owned()
}

fn parse_report_type(value: &str) -> Result<ReportType> {
    match value {
        "nudity" => Ok(ReportType::Nudity),
        "malware" => Ok(ReportType::Malware),
        "profanity" => Ok(ReportType::Profanity),
        "illegal" => Ok(ReportType::Illegal),
        "spam" => Ok(ReportType::Spam),
        "impersonation" => Ok(ReportType::Impersonation),
        other => bail!("unknown report type: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::prelude::{EventBuilder, Kind};

    fn sorted(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }

    #[test]
    fn attestation_roundtrip() {
        let keys = Keys::generate();
        let a = Attestation::new(
            keys.public_key().to_hex(),
            vec!["npub1a".into(), "npub1b".into()],
        );

        let event = a.to_event(&keys).unwrap();
        assert_eq!(event.kind, Kind::from(ATTESTATION_KIND));
        assert_eq!(event.content, "");
        verify_event(&event).unwrap();

        let parsed = Attestation::from_event(&event).unwrap();
        assert_eq!(parsed.id, a.id);
        assert_eq!(parsed.attester, keys.public_key().to_hex());
        assert_eq!(parsed.attributes, sorted(a.attributes.clone()));
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

        assert_eq!(parsed.attributes, sorted(a.attributes.clone()));
        assert_eq!(parsed.context, a.context);
        assert!(parsed.ended_at.is_some());
        assert_eq!(
            parsed.ended_at.unwrap().timestamp(),
            a.ended_at.unwrap().timestamp()
        );
    }

    #[test]
    fn attestation_signer_can_differ_from_attester() {
        let crawler_keys = Keys::generate();
        let attester_keys = Keys::generate();
        let a = Attestation::new(
            attester_keys.public_key().to_hex(),
            vec!["external:reviewer:alice".into(), "npub1alice".into()],
        );

        let event = a.to_event(&crawler_keys).unwrap();
        assert_eq!(event.pubkey.to_hex(), crawler_keys.public_key().to_hex());

        let parsed = Attestation::from_event(&event).unwrap();
        assert_eq!(parsed.attester, attester_keys.public_key().to_hex());
    }

    #[test]
    fn attestation_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&keys)
            .unwrap();

        let err = Attestation::from_event(&event).unwrap_err();
        assert!(err.to_string().contains("wrong fact op kind"));
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
        assert_eq!(event.content, "");
        verify_event(&event).unwrap();

        let parsed = Rating::from_event(&event).unwrap();
        assert_eq!(parsed.id, r.id);
        assert_eq!(parsed.rater, keys.public_key().to_hex());
        assert_eq!(parsed.subject, subject_keys.public_key().to_hex());
        assert_eq!(parsed.sentiment, Sentiment::Positive);
        assert_eq!(parsed.context, Some("helpful dev".into()));
        assert_eq!(parsed.tags, sorted(r.tags.clone()));
        assert!(parsed.report_type.is_none());
        assert_eq!(parsed.created_at.timestamp(), r.created_at.timestamp());
    }

    #[test]
    fn rating_signer_can_differ_from_rater() {
        let crawler_keys = Keys::generate();
        let rater_keys = Keys::generate();
        let subject_keys = Keys::generate();
        let r = Rating::new(
            rater_keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            Sentiment::Positive,
        );

        let event = r.to_event(&crawler_keys).unwrap();
        assert_eq!(event.pubkey.to_hex(), crawler_keys.public_key().to_hex());

        let parsed = Rating::from_event(&event).unwrap();
        assert_eq!(parsed.rater, rater_keys.public_key().to_hex());
        assert_eq!(parsed.subject, subject_keys.public_key().to_hex());
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
        assert!(err.to_string().contains("wrong fact op kind"));
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
        assert_eq!(event.content, "");
        verify_event(&event).unwrap();

        let parsed = CounterAttestation::from_event(&event).unwrap();
        assert_eq!(parsed.id, ca.id);
        assert_eq!(parsed.attester, keys.public_key().to_hex());
        assert_eq!(parsed.attributes, sorted(ca.attributes.clone()));
        assert!(parsed.context.is_none());
        assert!(parsed.disputed_event_id.is_none());
        assert_eq!(parsed.created_at.timestamp(), ca.created_at.timestamp());
    }

    #[test]
    fn counter_attestation_with_disputed_event_id() {
        let keys = Keys::generate();

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
        assert_eq!(parsed.attributes, sorted(ca.attributes.clone()));
    }

    #[test]
    fn counter_attestation_signer_can_differ_from_attester() {
        let crawler_keys = Keys::generate();
        let attester_keys = Keys::generate();
        let ca = CounterAttestation::new(
            attester_keys.public_key().to_hex(),
            vec!["external:alice".into(), "external:bob".into()],
        );

        let event = ca.to_event(&crawler_keys).unwrap();
        assert_eq!(event.pubkey.to_hex(), crawler_keys.public_key().to_hex());

        let parsed = CounterAttestation::from_event(&event).unwrap();
        assert_eq!(parsed.attester, attester_keys.public_key().to_hex());
    }

    #[test]
    fn counter_attestation_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&keys)
            .unwrap();

        let err = CounterAttestation::from_event(&event).unwrap_err();
        assert!(err.to_string().contains("wrong fact op kind"));
    }

    #[test]
    fn counter_attestation_all_fields_survive() {
        let keys = Keys::generate();

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
        assert_eq!(parsed.attributes, sorted(ca.attributes.clone()));
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
