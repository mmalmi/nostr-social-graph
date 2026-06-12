use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sentiment {
    Positive,
    Negative,
    Neutral,
}

/// NIP-56 report types for negative ratings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportType {
    Nudity,
    Malware,
    Profanity,
    Illegal,
    Spam,
    Impersonation,
}

/// A subjective evaluation of an entity or npub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rating {
    pub id: String,
    pub rater: String,
    /// The npub or entity ID being rated.
    pub subject: String,
    pub sentiment: Sentiment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Free-form domain tags for filtering (e.g. "relay-op", "rust", "moderation").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_type: Option<ReportType>,
    pub created_at: DateTime<Utc>,
}

impl Rating {
    pub fn new(rater: impl Into<String>, subject: impl Into<String>, sentiment: Sentiment) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            rater: rater.into(),
            subject: subject.into(),
            sentiment,
            context: None,
            tags: Vec::new(),
            report_type: None,
            created_at: Utc::now(),
        }
    }

    pub fn is_positive(&self) -> bool {
        self.sentiment == Sentiment::Positive
    }

    pub fn is_negative(&self) -> bool {
        self.sentiment == Sentiment::Negative
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_positive() {
        let r = Rating::new("npub1rater", "npub1target", Sentiment::Positive);
        assert!(!r.id.is_empty());
        assert_eq!(r.rater, "npub1rater");
        assert_eq!(r.subject, "npub1target");
        assert!(r.is_positive());
        assert!(!r.is_negative());
        assert!(r.context.is_none());
        assert!(r.tags.is_empty());
        assert!(r.report_type.is_none());
    }

    #[test]
    fn construction_negative_with_report() {
        let mut r = Rating::new("npub1rater", "npub1spammer", Sentiment::Negative);
        r.context = Some("sent unsolicited DMs".into());
        r.report_type = Some(ReportType::Spam);

        assert!(r.is_negative());
        assert!(!r.is_positive());
        assert_eq!(r.report_type, Some(ReportType::Spam));
    }

    #[test]
    fn construction_neutral() {
        let r = Rating::new("npub1rater", "npub1target", Sentiment::Neutral);
        assert!(!r.is_positive());
        assert!(!r.is_negative());
    }

    #[test]
    fn with_tags() {
        let mut r = Rating::new("rater", "target", Sentiment::Positive);
        r.context = Some("great relay operator".into());
        r.tags = vec!["relay-op".into(), "infrastructure".into()];

        assert_eq!(r.tags, vec!["relay-op", "infrastructure"]);
    }

    #[test]
    fn toml_roundtrip_positive() {
        let mut r = Rating::new("npub1rater", "npub1target", Sentiment::Positive);
        r.context = Some("helpful in nostr dev".into());
        r.tags = vec!["dev".into(), "nostr".into()];

        let serialized = toml::to_string_pretty(&r).unwrap();
        let deserialized: Rating = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.id, r.id);
        assert_eq!(deserialized.rater, "npub1rater");
        assert_eq!(deserialized.subject, "npub1target");
        assert_eq!(deserialized.sentiment, Sentiment::Positive);
        assert_eq!(deserialized.context, Some("helpful in nostr dev".into()));
        assert_eq!(deserialized.tags, vec!["dev", "nostr"]);
        assert!(deserialized.report_type.is_none());
    }

    #[test]
    fn toml_roundtrip_negative_with_report() {
        let mut r = Rating::new("npub1rater", "npub1bad", Sentiment::Negative);
        r.context = Some("impersonating someone".into());
        r.report_type = Some(ReportType::Impersonation);

        let serialized = toml::to_string_pretty(&r).unwrap();
        let deserialized: Rating = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.sentiment, Sentiment::Negative);
        assert_eq!(deserialized.report_type, Some(ReportType::Impersonation));
    }

    #[test]
    fn toml_roundtrip_neutral() {
        let r = Rating::new("rater", "target", Sentiment::Neutral);
        let serialized = toml::to_string_pretty(&r).unwrap();
        let deserialized: Rating = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.sentiment, Sentiment::Neutral);
    }

    #[test]
    fn toml_omits_none_and_empty_fields() {
        let r = Rating::new("rater", "target", Sentiment::Positive);
        let serialized = toml::to_string_pretty(&r).unwrap();
        assert!(!serialized.contains("context"));
        assert!(!serialized.contains("tags"));
        assert!(!serialized.contains("report_type"));
    }

    #[test]
    fn all_report_types_roundtrip() {
        let types = vec![
            ReportType::Nudity,
            ReportType::Malware,
            ReportType::Profanity,
            ReportType::Illegal,
            ReportType::Spam,
            ReportType::Impersonation,
        ];
        for rt in types {
            let mut r = Rating::new("rater", "target", Sentiment::Negative);
            r.report_type = Some(rt.clone());
            let serialized = toml::to_string_pretty(&r).unwrap();
            let deserialized: Rating = toml::from_str(&serialized).unwrap();
            assert_eq!(deserialized.report_type, Some(rt));
        }
    }
}
