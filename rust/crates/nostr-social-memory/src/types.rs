use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A unique entity — a person, organization, or bot.
/// Keyed by UUID. May have multiple npubs attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npubs: Vec<NpubEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Relationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub member_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, toml::Value>,
}

/// An npub associated with an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpubEntry {
    pub npub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub added_at: DateTime<Utc>,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Relationship metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    #[serde(rename = "type")]
    pub kind: RelationshipKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationshipKind {
    Friend,
    Acquaintance,
    Adversary,
    Organization,
    Unknown,
}

impl Entity {
    /// Create a new entity with a generated UUID.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            aliases: Vec::new(),
            created_at: Utc::now(),
            npubs: Vec::new(),
            relationship: None,
            member_ids: Vec::new(),
            tags: BTreeMap::new(),
        }
    }

    /// Get the first active npub, if any.
    pub fn active_npub(&self) -> Option<&str> {
        self.npubs
            .iter()
            .find(|e| e.active)
            .map(|e| e.npub.as_str())
    }

    /// Get all active npubs.
    pub fn active_npubs(&self) -> Vec<&str> {
        self.npubs
            .iter()
            .filter(|e| e.active)
            .map(|e| e.npub.as_str())
            .collect()
    }

    /// Add an npub to this entity.
    pub fn add_npub(&mut self, npub: impl Into<String>, label: Option<String>) {
        self.npubs.push(NpubEntry {
            npub: npub.into(),
            label,
            added_at: Utc::now(),
            active: true,
            notes: None,
        });
    }

    /// Check if name or any alias matches (case-insensitive).
    pub fn matches_name(&self, query: &str) -> bool {
        self.name.eq_ignore_ascii_case(query)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(query))
    }

    /// Check if any npub matches.
    pub fn has_npub(&self, npub: &str) -> bool {
        self.npubs.iter().any(|e| e.npub == npub)
    }
}
