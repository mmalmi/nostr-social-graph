use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A factual claim that a set of identifiers belong together.
///
/// An attestation says: "I claim these identifiers (npubs, entity UUIDs, etc.)
/// all refer to the same entity." Time-bounded — ended_at marks when the
/// relationship ended (key revoked, left org, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    pub id: String,
    pub attester: String,
    /// Identifiers claimed to belong together.
    pub attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
}

impl Attestation {
    pub fn new(attester: impl Into<String>, attributes: Vec<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            attester: attester.into(),
            attributes,
            context: None,
            created_at: Utc::now(),
            ended_at: None,
        }
    }

    /// Whether this attestation is still active (no end date).
    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction() {
        let a = Attestation::new("npub1attester", vec!["npub1a".into(), "npub1b".into()]);
        assert!(!a.id.is_empty());
        assert_eq!(a.attester, "npub1attester");
        assert_eq!(a.attributes, vec!["npub1a", "npub1b"]);
        assert!(a.context.is_none());
        assert!(a.ended_at.is_none());
        assert!(a.is_active());
    }

    #[test]
    fn with_context() {
        let mut a = Attestation::new("attester", vec!["npub1a".into()]);
        a.context = Some("same person, confirmed at meetup".into());
        assert_eq!(
            a.context.as_deref(),
            Some("same person, confirmed at meetup")
        );
    }

    #[test]
    fn ended_attestation() {
        let mut a = Attestation::new("attester", vec!["npub1a".into(), "org-uuid".into()]);
        a.ended_at = Some(Utc::now());
        assert!(!a.is_active());
    }

    #[test]
    fn toml_roundtrip() {
        let mut a = Attestation::new(
            "npub1attester",
            vec!["npub1a".into(), "npub1b".into(), "entity-uuid-123".into()],
        );
        a.context = Some("verified at conference".into());

        let serialized = toml::to_string_pretty(&a).unwrap();
        let deserialized: Attestation = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, a.id);
        assert_eq!(deserialized.attester, a.attester);
        assert_eq!(deserialized.attributes, a.attributes);
        assert_eq!(deserialized.context, a.context);
        assert!(deserialized.is_active());
    }

    #[test]
    fn toml_roundtrip_with_end_date() {
        let mut a = Attestation::new("attester", vec!["npub1old".into()]);
        a.context = Some("key compromised".into());
        a.ended_at = Some(Utc::now());

        let serialized = toml::to_string_pretty(&a).unwrap();
        let deserialized: Attestation = toml::from_str(&serialized).unwrap();

        assert!(!deserialized.is_active());
        assert!(deserialized.ended_at.is_some());
    }

    #[test]
    fn toml_omits_none_fields() {
        let a = Attestation::new("attester", vec!["npub1a".into()]);
        let serialized = toml::to_string_pretty(&a).unwrap();
        assert!(!serialized.contains("context"));
        assert!(!serialized.contains("ended_at"));
    }

    #[test]
    fn empty_attributes() {
        let a = Attestation::new("attester", vec![]);
        assert!(a.attributes.is_empty());

        let serialized = toml::to_string_pretty(&a).unwrap();
        let deserialized: Attestation = toml::from_str(&serialized).unwrap();
        assert!(deserialized.attributes.is_empty());
    }
}
