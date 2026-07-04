use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, TimeZone, Utc};
use nostr_identity::{
    FACT_OP_KIND, Fact, FactOp, FactOpLinks, build_fact_op_event_with_links_and_identifiers,
    parse_fact_op_event,
};
use nostr_sdk::prelude::{Event, Keys};
use uuid::Uuid;

use crate::attestation::Attestation;
use crate::counter_attestation::CounterAttestation;
use crate::rating::Rating;

/// Social-memory records are encoded as shared fact operation events.
pub const ATTESTATION_KIND: u16 = FACT_OP_KIND;
pub const RATING_KIND: u16 = FACT_OP_KIND;
pub const COUNTER_ATTESTATION_KIND: u16 = FACT_OP_KIND;

const SCHEMA_VERSION: &str = "1";
const TYPE_ATTESTATION: &str = "social_memory_attestation";
const TYPE_RATING: &str = "rating";
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
        if let Some(scope) = &self.scope {
            facts.push(Fact::new("scope", [scope.clone()]));
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
            scope: optional_scalar(&op, "scope")?,
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
        if let Some(scope) = &self.scope {
            facts.push(Fact::new("scope", [scope.clone()]));
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
            scope: optional_scalar(&op, "scope")?,
            created_at: required_datetime(&op, "created_at")?,
        })
    }
}

pub trait RatingEventExt {
    fn to_event(&self, keys: &Keys) -> Result<Event>;
}

impl RatingEventExt for Rating {
    /// Sign this rating as a fact event.
    ///
    /// The event signer is the publisher/crawler. The rating author is the
    /// `rater` fact, so crawled reviews can preserve their external author.
    fn to_event(&self, keys: &Keys) -> Result<Event> {
        rating_to_event(self, keys)
    }
}

pub fn rating_to_event(rating: &Rating, keys: &Keys) -> Result<Event> {
    rating.validate()?;
    let mut facts = base_facts_unix(TYPE_RATING, rating.created_at);
    facts.push(Fact::new("rater", [rating.rater.clone()]));
    facts.push(Fact::new("subject", [rating.subject.clone()]));
    facts.push(Fact::new("rating", [rating.rating.to_string()]));
    facts.push(Fact::new("min_rating", [rating.min_rating.to_string()]));
    facts.push(Fact::new("max_rating", [rating.max_rating.to_string()]));
    if let Some(scope) = &rating.scope {
        facts.push(Fact::new("scope", [scope.clone()]));
    }
    if let Some(sample_count) = rating.sample_count {
        facts.push(Fact::new("sample_count", [sample_count.to_string()]));
    }
    if let Some(window_start) = rating.window_start {
        facts.push(Fact::new("window_start", [window_start.to_string()]));
    }
    if let Some(window_end) = rating.window_end {
        facts.push(Fact::new("window_end", [window_end.to_string()]));
    }
    facts.extend(
        rating
            .evidence
            .iter()
            .cloned()
            .map(|evidence| Fact::new("evidence", [evidence])),
    );
    if let Some(reason) = &rating.reason {
        facts.push(Fact::new("reason", [reason.clone()]));
    }
    facts.extend(
        rating
            .tags
            .iter()
            .cloned()
            .map(|tag| Fact::new("tag", [tag])),
    );

    build_fact_record_event(
        keys,
        &rating.id,
        facts,
        FactOpLinks::default(),
        safe_identifiers(
            [&rating.rater, &rating.subject]
                .into_iter()
                .chain(rating.scope.iter())
                .chain(rating.evidence.iter())
                .chain(rating.tags.iter())
                .map(String::as_str),
        ),
    )
}

/// Parse a rating from a fact event.
pub fn rating_from_event(event: &Event) -> Result<Rating> {
    let op = parse_social_memory_fact(event, TYPE_RATING)?;
    let scope = optional_scalar(&op, "scope")?;
    let rating = Rating {
        id: op.subject.to_string(),
        rater: required_scalar(&op, "rater")?,
        subject: required_scalar(&op, "subject")?,
        scope,
        rating: required_i64(&op, "rating")?,
        min_rating: required_i64(&op, "min_rating")?,
        max_rating: required_i64(&op, "max_rating")?,
        sample_count: optional_u64(&op, "sample_count")?,
        window_start: optional_u64(&op, "window_start")?,
        window_end: optional_u64(&op, "window_end")?,
        evidence: scalar_values(&op, "evidence")?,
        reason: optional_scalar(&op, "reason")?,
        tags: scalar_values(&op, "tag")?,
        created_at: required_u64(&op, "created_at")?,
    };
    rating.validate()?;
    Ok(rating)
}

fn base_facts(record_type: &'static str, created_at: DateTime<Utc>) -> Result<Vec<Fact>> {
    Ok(base_facts_unix(record_type, datetime_unix(created_at)?))
}

fn base_facts_unix(record_type: &'static str, created_at: u64) -> Vec<Fact> {
    vec![
        Fact::new("type", [record_type.to_owned()]),
        Fact::new("schema", [SCHEMA_VERSION.to_owned()]),
        Fact::new("created_at", [created_at.to_string()]),
    ]
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

fn required_i64(op: &FactOp, predicate: &str) -> Result<i64> {
    required_scalar(op, predicate)?
        .parse()
        .map_err(|error| anyhow!("invalid i64 social-memory fact {predicate}: {error}"))
}

fn required_u64(op: &FactOp, predicate: &str) -> Result<u64> {
    required_scalar(op, predicate)?
        .parse()
        .map_err(|error| anyhow!("invalid u64 social-memory fact {predicate}: {error}"))
}

fn optional_u64(op: &FactOp, predicate: &str) -> Result<Option<u64>> {
    optional_scalar(op, predicate)?
        .map(|value| {
            value
                .parse()
                .map_err(|error| anyhow!("invalid u64 social-memory fact {predicate}: {error}"))
        })
        .transpose()
}

fn safe_identifiers<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    values
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_whitespace))
        .map(str::to_owned)
        .collect()
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
        assert!(parsed.scope.is_none());
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
        a.scope = Some("verified at conference".into());
        a.ended_at = Some(Utc::now());

        let event = a.to_event(&keys).unwrap();
        let parsed = Attestation::from_event(&event).unwrap();

        assert_eq!(parsed.attributes, sorted(a.attributes.clone()));
        assert_eq!(parsed.scope, a.scope);
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
    fn rating_roundtrip_with_integer_range() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let mut r = Rating::new(
            keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            80,
            0,
            100,
        );
        r.scope = Some("helpful dev".into());
        r.sample_count = Some(7);
        r.window_start = Some(1_000);
        r.window_end = Some(2_000);
        r.evidence = vec!["https://example.test/review/1".into()];
        r.reason = Some("consistently useful".into());
        r.tags = vec!["rust".into(), "nostr".into()];
        r.created_at = 3_000;

        let event = r.to_event(&keys).unwrap();
        assert_eq!(event.kind, Kind::from(RATING_KIND));
        assert_eq!(event.content, "");
        verify_event(&event).unwrap();

        let parsed = rating_from_event(&event).unwrap();
        assert_eq!(parsed.id, r.id);
        assert_eq!(parsed.rater, keys.public_key().to_hex());
        assert_eq!(parsed.subject, subject_keys.public_key().to_hex());
        assert_eq!(parsed.rating, 80);
        assert_eq!(parsed.min_rating, 0);
        assert_eq!(parsed.max_rating, 100);
        assert_eq!(parsed.scope, Some("helpful dev".into()));
        assert_eq!(parsed.sample_count, Some(7));
        assert_eq!(parsed.window_start, Some(1_000));
        assert_eq!(parsed.window_end, Some(2_000));
        assert_eq!(parsed.evidence, vec!["https://example.test/review/1"]);
        assert_eq!(parsed.reason, Some("consistently useful".into()));
        assert_eq!(parsed.tags, sorted(r.tags.clone()));
        assert_eq!(parsed.created_at, 3_000);
    }

    #[test]
    fn rating_context_fact_is_not_scope() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let r = Rating::new(
            keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            80,
            0,
            100,
        );
        let facts = vec![
            Fact::new("type", [TYPE_RATING.to_owned()]),
            Fact::new("schema", [SCHEMA_VERSION.to_owned()]),
            Fact::new("created_at", [3_000_u64.to_string()]),
            Fact::new("rater", [r.rater.clone()]),
            Fact::new("subject", [r.subject.clone()]),
            Fact::new("rating", [r.rating.to_string()]),
            Fact::new("min_rating", [r.min_rating.to_string()]),
            Fact::new("max_rating", [r.max_rating.to_string()]),
            Fact::new("note", ["helpful dev".to_string()]),
        ];
        let event =
            build_fact_record_event(&keys, &r.id, facts, FactOpLinks::default(), Vec::new())
                .unwrap();

        let parsed = rating_from_event(&event).unwrap();
        assert_eq!(parsed.scope, None);
    }

    #[test]
    fn rating_signer_can_differ_from_rater() {
        let crawler_keys = Keys::generate();
        let rater_keys = Keys::generate();
        let subject_keys = Keys::generate();
        let r = Rating::new(
            rater_keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            4,
            1,
            5,
        );

        let event = r.to_event(&crawler_keys).unwrap();
        assert_eq!(event.pubkey.to_hex(), crawler_keys.public_key().to_hex());

        let parsed = rating_from_event(&event).unwrap();
        assert_eq!(parsed.rater, rater_keys.public_key().to_hex());
        assert_eq!(parsed.subject, subject_keys.public_key().to_hex());
        assert_eq!(parsed.normalized_score().unwrap(), 50);
    }

    #[test]
    fn rating_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&keys)
            .unwrap();

        let err = rating_from_event(&event).unwrap_err();
        assert!(err.to_string().contains("wrong fact op kind"));
    }

    #[test]
    fn rating_no_optional_fields() {
        let keys = Keys::generate();
        let subject_keys = Keys::generate();
        let r = Rating::new(
            keys.public_key().to_hex(),
            subject_keys.public_key().to_hex(),
            1,
            -1,
            1,
        );

        let event = r.to_event(&keys).unwrap();
        let parsed = rating_from_event(&event).unwrap();
        assert!(parsed.scope.is_none());
        assert!(parsed.sample_count.is_none());
        assert!(parsed.window_start.is_none());
        assert!(parsed.window_end.is_none());
        assert!(parsed.evidence.is_empty());
        assert!(parsed.reason.is_none());
        assert!(parsed.tags.is_empty());
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
        assert!(parsed.scope.is_none());
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
        ca.scope = Some("these are different people".into());

        let event = ca.to_event(&keys).unwrap();
        let parsed = CounterAttestation::from_event(&event).unwrap();

        assert_eq!(parsed.disputed_event_id, Some(att_event_id));
        assert_eq!(parsed.scope, Some("these are different people".into()));
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
        ca.scope = Some("impersonation attempt".into());
        ca.disputed_event_id = Some(dummy_event.id.to_hex());

        let event = ca.to_event(&keys).unwrap();
        let parsed = CounterAttestation::from_event(&event).unwrap();

        assert_eq!(parsed.id, ca.id);
        assert_eq!(parsed.attester, keys.public_key().to_hex());
        assert_eq!(parsed.attributes, sorted(ca.attributes.clone()));
        assert_eq!(parsed.scope, ca.scope);
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
