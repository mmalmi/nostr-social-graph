//! Private contact memory. Storage and transport are left to the application.
//!
//! Keep one record per viewing account and contact, separate from public profile
//! metadata. A favorite is a private preference and conveys no social trust.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ContactMemory {
    pub first_seen_name: Option<String>,
    pub accepted_name: Option<String>,
    pub favorite: bool,
    pub name_changes: Vec<AcceptedNameChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedNameChange {
    pub previous_name: String,
    pub accepted_name: String,
    pub accepted_at_secs: u64,
}

impl ContactMemory {
    /// Call after an interaction, not for every profile fetched by the app.
    /// The first nonblank name is retained; later observations cannot rename it.
    pub fn observe_name(&mut self, name: &str) {
        if name.trim().is_empty() {
            return;
        }
        self.first_seen_name.get_or_insert_with(|| name.to_string());
        self.accepted_name.get_or_insert_with(|| name.to_string());
    }

    /// The current public name is transient and remains outside this record.
    pub fn pending_name(&self, current_name: Option<String>) -> Option<String> {
        current_name.filter(|name| {
            !name.trim().is_empty()
                && self
                    .accepted_name
                    .as_ref()
                    .is_some_and(|saved| saved != name)
        })
    }

    /// Approve exactly the name shown to the user, only if it still matches the
    /// latest public profile. A stale or redundant approval leaves no history.
    pub fn approve_name(
        &mut self,
        expected: &str,
        current: &str,
        now_secs: u64,
    ) -> Option<AcceptedNameChange> {
        if expected != current || expected.trim().is_empty() {
            return None;
        }
        let previous = self
            .accepted_name
            .as_ref()
            .filter(|name| *name != expected)?
            .clone();
        let change = AcceptedNameChange {
            previous_name: previous,
            accepted_name: expected.to_string(),
            accepted_at_secs: now_secs,
        };
        self.accepted_name = Some(change.accepted_name.clone());
        self.name_changes.push(change.clone());
        Some(change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_first_real_name_and_keeps_profile_updates_pending() {
        let mut memory = ContactMemory::default();
        memory.observe_name(" ");
        assert_eq!(memory, ContactMemory::default());
        assert_eq!(memory.pending_name(Some("Alice".into())), None);
        memory.observe_name("Alice");
        memory.observe_name("Bob");
        assert_eq!(memory.first_seen_name.as_deref(), Some("Alice"));
        assert_eq!(memory.accepted_name.as_deref(), Some("Alice"));
        assert_eq!(memory.pending_name(Some("Bob".into())), Some("Bob".into()));
        assert_eq!(memory.pending_name(Some("Alice".into())), None);
        assert_eq!(memory.pending_name(Some(" ".into())), None);
        assert_eq!(memory.pending_name(None), None);
    }

    #[test]
    fn requires_current_explicit_approval_and_retains_all_accepted_names() {
        let mut memory = ContactMemory::default();
        assert_eq!(memory.approve_name("Bob", "Bob", 100), None);
        memory.observe_name("Alice");
        assert_eq!(memory.approve_name("Bob", "Carol", 100), None);
        assert_eq!(memory.approve_name(" ", " ", 100), None);
        assert_eq!(memory.approve_name("Alice", "Alice", 100), None);
        let first = memory.approve_name("Bob", "Bob", 100).unwrap();
        let second = memory.approve_name("Alice", "Alice", 101).unwrap();
        assert_eq!(memory.first_seen_name.as_deref(), Some("Alice"));
        assert_eq!(memory.accepted_name.as_deref(), Some("Alice"));
        assert_eq!(memory.name_changes, vec![first, second]);
        let portable: ContactMemory =
            serde_json::from_str(include_str!("../../../../fixtures/contact-memory.json")).unwrap();
        assert_eq!(memory, portable);
        let json = serde_json::to_string(&memory).unwrap();
        assert_eq!(
            serde_json::from_str::<ContactMemory>(&json).unwrap(),
            memory
        );
    }

    #[test]
    fn favorites_are_independent_and_missing_fields_default_for_old_storage() {
        let mut memory: ContactMemory = serde_json::from_str(r#"{"favorite":true}"#).unwrap();
        memory.observe_name("Alice");
        assert!(memory.favorite);
        memory.favorite = false;
        assert_eq!(memory.first_seen_name.as_deref(), Some("Alice"));
        assert_eq!(memory.accepted_name.as_deref(), Some("Alice"));
        assert!(memory.name_changes.is_empty());
    }
}
