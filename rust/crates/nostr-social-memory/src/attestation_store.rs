use crate::attestation::Attestation;
use crate::store::EntityStore;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

const ATTESTATIONS_DIR: &str = "attestations";
const BY_NPUB_DIR: &str = "_by_npub";

impl EntityStore {
    /// Add an attestation to an entity's attestations directory.
    pub fn add_attestation(&self, entity_id: &str, attestation: &Attestation) -> Result<()> {
        let dir = self.entity_dir(entity_id).join(ATTESTATIONS_DIR);
        if !self.entity_dir(entity_id).exists() {
            anyhow::bail!("Entity {} does not exist", entity_id);
        }
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.toml", attestation.id));
        let content =
            toml::to_string_pretty(attestation).context("Failed to serialize attestation")?;
        fs::write(&path, content)?;
        Ok(())
    }

    /// List all attestations for an entity.
    pub fn list_attestations(&self, entity_id: &str) -> Result<Vec<Attestation>> {
        let dir = self.entity_dir(entity_id).join(ATTESTATIONS_DIR);
        read_toml_files(&dir)
    }

    /// Remove an attestation by ID from an entity.
    pub fn remove_attestation(&self, entity_id: &str, attestation_id: &str) -> Result<bool> {
        let path = self
            .entity_dir(entity_id)
            .join(ATTESTATIONS_DIR)
            .join(format!("{}.toml", attestation_id));
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)?;
        Ok(true)
    }

    /// Add an attestation for an npub. If the npub belongs to a known entity,
    /// store it there. Otherwise, store under `_by_npub/{npub}/attestations/`.
    pub fn add_attestation_for_npub(&self, npub: &str, attestation: &Attestation) -> Result<()> {
        if let Some(entity) = self.find_by_npub(npub)? {
            return self.add_attestation(&entity.id, attestation);
        }
        let dir = self.npub_attestations_dir(npub);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.toml", attestation.id));
        let content =
            toml::to_string_pretty(attestation).context("Failed to serialize attestation")?;
        fs::write(&path, content)?;
        Ok(())
    }

    /// List attestations for an npub. Checks entity first, falls back to orphan storage.
    pub fn list_attestations_for_npub(&self, npub: &str) -> Result<Vec<Attestation>> {
        if let Some(entity) = self.find_by_npub(npub)? {
            return self.list_attestations(&entity.id);
        }
        let dir = self.npub_attestations_dir(npub);
        read_toml_files(&dir)
    }

    /// Move orphan attestations from `_by_npub/{npub}/` into an entity's directory.
    pub fn adopt_npub_attestations(&self, entity_id: &str, npub: &str) -> Result<u32> {
        if !self.entity_dir(entity_id).exists() {
            anyhow::bail!("Entity {} does not exist", entity_id);
        }
        let orphan_dir = self.npub_attestations_dir(npub);
        if !orphan_dir.exists() {
            return Ok(0);
        }
        let entity_att_dir = self.entity_dir(entity_id).join(ATTESTATIONS_DIR);
        fs::create_dir_all(&entity_att_dir)?;
        let mut count = 0u32;
        for entry in fs::read_dir(&orphan_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let dest = entity_att_dir.join(entry.file_name());
                fs::rename(entry.path(), dest)?;
                count += 1;
            }
        }
        let npub_dir = self.base_dir().join(BY_NPUB_DIR).join(npub);
        let _ = fs::remove_dir(&orphan_dir);
        let _ = fs::remove_dir(&npub_dir);
        Ok(count)
    }

    fn npub_attestations_dir(&self, npub: &str) -> PathBuf {
        self.base_dir()
            .join(BY_NPUB_DIR)
            .join(npub)
            .join(ATTESTATIONS_DIR)
    }
}

fn read_toml_files<T: serde::de::DeserializeOwned>(dir: &std::path::Path) -> Result<Vec<T>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            let content = fs::read_to_string(&path)?;
            let item: T = toml::from_str(&content)
                .with_context(|| format!("Failed to parse {}", path.display()))?;
            items.push(item);
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Entity;
    use tempfile::TempDir;

    fn setup() -> (TempDir, EntityStore) {
        let tmp = TempDir::new().unwrap();
        let store = EntityStore::new(tmp.path().join("identities")).unwrap();
        (tmp, store)
    }

    fn make_entity(name: &str, npub: &str) -> Entity {
        let mut e = Entity::new(name);
        e.add_npub(npub, None);
        e
    }

    fn make_attestation(attester: &str) -> Attestation {
        Attestation::new(attester, vec!["npub1a".into(), "npub1b".into()])
    }

    #[test]
    fn add_and_list() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let a1 = make_attestation("npub1bob");
        let a2 = make_attestation("npub1charlie");
        store.add_attestation(&alice.id, &a1).unwrap();
        store.add_attestation(&alice.id, &a2).unwrap();

        let list = store.list_attestations(&alice.id).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn add_nonexistent_entity_fails() {
        let (_tmp, store) = setup();
        let a = make_attestation("npub1bob");
        assert!(store.add_attestation("nonexistent", &a).is_err());
    }

    #[test]
    fn remove() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let a = make_attestation("npub1bob");
        let aid = a.id.clone();
        store.add_attestation(&alice.id, &a).unwrap();

        assert!(store.remove_attestation(&alice.id, &aid).unwrap());
        assert!(!store.remove_attestation(&alice.id, &aid).unwrap());
        assert!(store.list_attestations(&alice.id).unwrap().is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();
        assert!(!store.remove_attestation(&alice.id, "no-such-id").unwrap());
    }

    #[test]
    fn list_empty() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();
        assert!(store.list_attestations(&alice.id).unwrap().is_empty());
    }

    #[test]
    fn list_nonexistent_entity_returns_empty() {
        let (_tmp, store) = setup();
        assert!(store.list_attestations("nonexistent").unwrap().is_empty());
    }

    #[test]
    fn for_known_npub_goes_to_entity() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let a = make_attestation("npub1bob");
        store.add_attestation_for_npub("npub1alice", &a).unwrap();

        let list = store.list_attestations(&alice.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, a.id);
    }

    #[test]
    fn for_unknown_npub_goes_to_orphan() {
        let (_tmp, store) = setup();

        let a = make_attestation("npub1bob");
        store.add_attestation_for_npub("npub1unknown", &a).unwrap();

        assert!(store.list().unwrap().is_empty());
        let list = store.list_attestations_for_npub("npub1unknown").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, a.id);
    }

    #[test]
    fn list_for_known_npub() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let a = make_attestation("npub1bob");
        store.add_attestation(&alice.id, &a).unwrap();

        let list = store.list_attestations_for_npub("npub1alice").unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn list_for_npub_no_data() {
        let (_tmp, store) = setup();
        assert!(
            store
                .list_attestations_for_npub("npub1nobody")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn adopt_moves_orphans() {
        let (_tmp, store) = setup();

        let a1 = make_attestation("npub1bob");
        let a2 = make_attestation("npub1charlie");
        store.add_attestation_for_npub("npub1alice", &a1).unwrap();
        store.add_attestation_for_npub("npub1alice", &a2).unwrap();

        assert_eq!(
            store
                .list_attestations_for_npub("npub1alice")
                .unwrap()
                .len(),
            2
        );

        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let moved = store
            .adopt_npub_attestations(&alice.id, "npub1alice")
            .unwrap();
        assert_eq!(moved, 2);
        assert_eq!(store.list_attestations(&alice.id).unwrap().len(), 2);

        let orphan_dir = store.base_dir().join(BY_NPUB_DIR).join("npub1alice");
        assert!(!orphan_dir.exists());
    }

    #[test]
    fn adopt_with_no_orphans() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();
        assert_eq!(
            store
                .adopt_npub_attestations(&alice.id, "npub1alice")
                .unwrap(),
            0
        );
    }

    #[test]
    fn adopt_nonexistent_entity_fails() {
        let (_tmp, store) = setup();
        assert!(
            store
                .adopt_npub_attestations("nonexistent", "npub1x")
                .is_err()
        );
    }

    #[test]
    fn persists_across_reads() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let mut a = Attestation::new("npub1bob", vec!["npub1x".into(), "npub1y".into()]);
        a.context = Some("confirmed at meetup".into());
        let aid = a.id.clone();
        store.add_attestation(&alice.id, &a).unwrap();

        let list = store.list_attestations(&alice.id).unwrap();
        assert_eq!(list.len(), 1);
        let loaded = &list[0];
        assert_eq!(loaded.id, aid);
        assert_eq!(loaded.attester, "npub1bob");
        assert_eq!(loaded.attributes, vec!["npub1x", "npub1y"]);
        assert_eq!(loaded.context, Some("confirmed at meetup".into()));
        assert!(loaded.is_active());
    }
}
