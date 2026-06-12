use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A signed claim that a set of identifiers do NOT belong together.
///
/// The inverse of an Attestation: "I claim these identifiers do NOT refer to
/// the same entity." Optionally references the specific attestation being
/// disputed via `disputed_event_id`.
///
/// No `ended_at` — you don't "end" a counter-claim, you delete it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterAttestation {
    pub id: String,
    pub attester: String,
    /// Identifiers claimed to NOT belong together.
    pub attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disputed_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl CounterAttestation {
    pub fn new(attester: impl Into<String>, attributes: Vec<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            attester: attester.into(),
            attributes,
            disputed_event_id: None,
            context: None,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction() {
        let ca = CounterAttestation::new("npub1attester", vec!["npub1a".into(), "npub1b".into()]);
        assert!(!ca.id.is_empty());
        assert_eq!(ca.attester, "npub1attester");
        assert_eq!(ca.attributes, vec!["npub1a", "npub1b"]);
        assert!(ca.disputed_event_id.is_none());
        assert!(ca.context.is_none());
    }

    #[test]
    fn with_context_and_disputed() {
        let mut ca = CounterAttestation::new("attester", vec!["npub1a".into()]);
        ca.context = Some("these are different people".into());
        ca.disputed_event_id = Some("abc123eventid".into());
        assert_eq!(ca.context.as_deref(), Some("these are different people"));
        assert_eq!(ca.disputed_event_id.as_deref(), Some("abc123eventid"));
    }

    #[test]
    fn toml_roundtrip() {
        let mut ca = CounterAttestation::new(
            "npub1attester",
            vec!["npub1a".into(), "npub1b".into(), "entity-uuid-123".into()],
        );
        ca.context = Some("mistaken identity".into());
        ca.disputed_event_id = Some("event123".into());

        let serialized = toml::to_string_pretty(&ca).unwrap();
        let deserialized: CounterAttestation = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, ca.id);
        assert_eq!(deserialized.attester, ca.attester);
        assert_eq!(deserialized.attributes, ca.attributes);
        assert_eq!(deserialized.context, ca.context);
        assert_eq!(deserialized.disputed_event_id, ca.disputed_event_id);
    }

    #[test]
    fn toml_omits_none_fields() {
        let ca = CounterAttestation::new("attester", vec!["npub1a".into()]);
        let serialized = toml::to_string_pretty(&ca).unwrap();
        assert!(!serialized.contains("context"));
        assert!(!serialized.contains("disputed_event_id"));
    }

    #[test]
    fn empty_attributes() {
        let ca = CounterAttestation::new("attester", vec![]);
        assert!(ca.attributes.is_empty());

        let serialized = toml::to_string_pretty(&ca).unwrap();
        let deserialized: CounterAttestation = toml::from_str(&serialized).unwrap();
        assert!(deserialized.attributes.is_empty());
    }
}
