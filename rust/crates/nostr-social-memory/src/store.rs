use crate::types::Entity;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const ENTITY_FILE: &str = "entity.toml";
const NOTES_FILE: &str = "notes.md";
const HISTORY_DIR: &str = "history";

/// Directory-based entity store.
///
/// Layout:
/// ```text
/// {base_dir}/
/// ├── {uuid}/
/// │   ├── entity.toml
/// │   ├── notes.md          (optional)
/// │   └── history/          (optional)
/// │       └── 2024-01.md
/// ```
pub struct EntityStore {
    base_dir: PathBuf,
}

impl EntityStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Result<Self> {
        let base_dir = base_dir.into();
        fs::create_dir_all(&base_dir)
            .with_context(|| format!("Failed to create identities dir: {}", base_dir.display()))?;
        Ok(Self { base_dir })
    }

    /// Create a new entity and persist it.
    pub fn create(&self, entity: &Entity) -> Result<()> {
        let dir = self.entity_dir(&entity.id);
        if dir.exists() {
            anyhow::bail!("Entity {} already exists", entity.id);
        }
        fs::create_dir_all(&dir)?;
        self.write_entity(entity)
    }

    /// Load an entity by UUID.
    pub fn get(&self, id: &str) -> Result<Option<Entity>> {
        let path = self.entity_dir(id).join(ENTITY_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let entity: Entity = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(Some(entity))
    }

    /// Update an existing entity.
    pub fn update(&self, entity: &Entity) -> Result<()> {
        let dir = self.entity_dir(&entity.id);
        if !dir.exists() {
            anyhow::bail!("Entity {} does not exist", entity.id);
        }
        self.write_entity(entity)
    }

    /// Delete an entity and all its files.
    pub fn delete(&self, id: &str) -> Result<bool> {
        let dir = self.entity_dir(id);
        if !dir.exists() {
            return Ok(false);
        }
        fs::remove_dir_all(&dir)?;
        Ok(true)
    }

    /// List all entities.
    pub fn list(&self) -> Result<Vec<Entity>> {
        let mut entities = Vec::new();
        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let toml_path = entry.path().join(ENTITY_FILE);
            if toml_path.exists() {
                let content = fs::read_to_string(&toml_path)?;
                if let Ok(id) = toml::from_str(&content) {
                    entities.push(id);
                }
            }
        }
        entities.sort_by(|a: &Entity, b: &Entity| a.name.cmp(&b.name));
        Ok(entities)
    }

    /// Find entity by petname or alias (case-insensitive).
    pub fn find_by_name(&self, name: &str) -> Result<Option<Entity>> {
        for entity in self.list()? {
            if entity.matches_name(name) {
                return Ok(Some(entity));
            }
        }
        Ok(None)
    }

    /// Find entity that owns a given npub.
    pub fn find_by_npub(&self, npub: &str) -> Result<Option<Entity>> {
        for entity in self.list()? {
            if entity.has_npub(npub) {
                return Ok(Some(entity));
            }
        }
        Ok(None)
    }

    /// Build an in-memory index for fast lookups.
    /// Returns (name→uuid, npub→uuid) maps.
    pub fn build_index(&self) -> Result<Index> {
        let mut by_name: HashMap<String, String> = HashMap::new();
        let mut by_npub: HashMap<String, String> = HashMap::new();
        for entity in self.list()? {
            by_name.insert(entity.name.to_lowercase(), entity.id.clone());
            for alias in &entity.aliases {
                by_name.insert(alias.to_lowercase(), entity.id.clone());
            }
            for entry in &entity.npubs {
                by_npub.insert(entry.npub.clone(), entity.id.clone());
            }
        }
        Ok(Index { by_name, by_npub })
    }

    // --- Notes & history ---

    /// Read freeform notes for an entity.
    pub fn get_notes(&self, id: &str) -> Result<Option<String>> {
        let path = self.entity_dir(id).join(NOTES_FILE);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(&path)?))
    }

    /// Write freeform notes for an entity.
    pub fn set_notes(&self, id: &str, content: &str) -> Result<()> {
        let dir = self.entity_dir(id);
        if !dir.exists() {
            anyhow::bail!("Entity {} does not exist", id);
        }
        fs::write(dir.join(NOTES_FILE), content)?;
        Ok(())
    }

    /// Append a history entry (e.g. monthly summary).
    pub fn append_history(&self, id: &str, filename: &str, content: &str) -> Result<()> {
        let dir = self.entity_dir(id).join(HISTORY_DIR);
        fs::create_dir_all(&dir)?;
        let path = dir.join(filename);
        let mut existing = if path.exists() {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(content);
        fs::write(&path, existing)?;
        Ok(())
    }

    /// List history files for an entity.
    pub fn list_history(&self, id: &str) -> Result<Vec<String>> {
        let dir = self.entity_dir(id).join(HISTORY_DIR);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && let Some(name) = entry.file_name().to_str()
            {
                files.push(name.to_string());
            }
        }
        files.sort();
        Ok(files)
    }

    /// Get the directory path for an entity.
    pub fn entity_dir(&self, id: &str) -> PathBuf {
        self.base_dir.join(id)
    }

    /// Get the base directory.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn write_entity(&self, entity: &Entity) -> Result<()> {
        let content = toml::to_string_pretty(entity).context("Failed to serialize entity")?;
        let path = self.entity_dir(&entity.id).join(ENTITY_FILE);
        fs::write(&path, &content)?;
        Ok(())
    }
}

/// In-memory lookup index built from scanning all entities.
pub struct Index {
    pub by_name: HashMap<String, String>,
    pub by_npub: HashMap<String, String>,
}

impl Index {
    /// Look up UUID by petname/alias (case-insensitive).
    pub fn resolve_name(&self, name: &str) -> Option<&str> {
        self.by_name.get(&name.to_lowercase()).map(|s| s.as_str())
    }

    /// Look up UUID by npub.
    pub fn resolve_npub(&self, npub: &str) -> Option<&str> {
        self.by_npub.get(npub).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, EntityStore) {
        let tmp = TempDir::new().unwrap();
        let store = EntityStore::new(tmp.path().join("identities")).unwrap();
        (tmp, store)
    }

    fn make_alice() -> Entity {
        let mut alice = Entity::new("alice");
        alice.add_npub(
            "npub1alicefake000000000000000000000000000000000000000000000000",
            Some("main".into()),
        );
        alice
    }

    #[test]
    fn create_and_get_entity() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        let id = alice.id.clone();

        store.create(&alice).unwrap();

        let loaded = store.get(&id).unwrap().unwrap();
        assert_eq!(loaded.name, "alice");
        assert_eq!(loaded.npubs.len(), 1);
        assert_eq!(loaded.npubs[0].npub, alice.npubs[0].npub);
        assert!(loaded.npubs[0].active);
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let (_tmp, store) = setup();
        assert!(store.get("nonexistent-uuid").unwrap().is_none());
    }

    #[test]
    fn create_duplicate_fails() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        store.create(&alice).unwrap();
        assert!(store.create(&alice).is_err());
    }

    #[test]
    fn update_entity() {
        let (_tmp, store) = setup();
        let mut alice = make_alice();
        store.create(&alice).unwrap();

        alice.aliases.push("a".into());
        alice.relationship = Some(Relationship {
            kind: RelationshipKind::Friend,
            trust: Some(0.9),
        });
        store.update(&alice).unwrap();

        let loaded = store.get(&alice.id).unwrap().unwrap();
        assert_eq!(loaded.aliases, vec!["a"]);
        assert_eq!(loaded.relationship.unwrap().kind, RelationshipKind::Friend);
    }

    #[test]
    fn update_nonexistent_fails() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        assert!(store.update(&alice).is_err());
    }

    #[test]
    fn delete_entity() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        store.create(&alice).unwrap();

        assert!(store.delete(&alice.id).unwrap());
        assert!(store.get(&alice.id).unwrap().is_none());
    }

    #[test]
    fn delete_nonexistent_returns_false() {
        let (_tmp, store) = setup();
        assert!(!store.delete("nope").unwrap());
    }

    #[test]
    fn list_entities_sorted_by_name() {
        let (_tmp, store) = setup();

        let mut bob = Entity::new("bob");
        bob.add_npub(
            "npub1bobfake0000000000000000000000000000000000000000000000000000",
            None,
        );
        let alice = make_alice();

        store.create(&bob).unwrap();
        store.create(&alice).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "alice");
        assert_eq!(list[1].name, "bob");
    }

    #[test]
    fn find_by_name_case_insensitive() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        store.create(&alice).unwrap();

        assert!(store.find_by_name("Alice").unwrap().is_some());
        assert!(store.find_by_name("ALICE").unwrap().is_some());
        assert!(store.find_by_name("nobody").unwrap().is_none());
    }

    #[test]
    fn find_by_alias() {
        let (_tmp, store) = setup();
        let mut alice = make_alice();
        alice.aliases.push("ally".into());
        store.create(&alice).unwrap();

        let found = store.find_by_name("ally").unwrap().unwrap();
        assert_eq!(found.id, alice.id);
    }

    #[test]
    fn find_by_npub() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        let npub = alice.npubs[0].npub.clone();
        store.create(&alice).unwrap();

        let found = store.find_by_npub(&npub).unwrap().unwrap();
        assert_eq!(found.id, alice.id);
        assert!(store.find_by_npub("npub1unknown").unwrap().is_none());
    }

    #[test]
    fn multiple_npubs_same_entity() {
        let (_tmp, store) = setup();
        let mut alice = make_alice();
        alice.add_npub(
            "npub1alicealt0000000000000000000000000000000000000000000000000",
            Some("alt".into()),
        );
        store.create(&alice).unwrap();

        // Both npubs resolve to same entity
        let found1 = store.find_by_npub(&alice.npubs[0].npub).unwrap().unwrap();
        let found2 = store.find_by_npub(&alice.npubs[1].npub).unwrap().unwrap();
        assert_eq!(found1.id, found2.id);
    }

    #[test]
    fn index_resolves_names_and_npubs() {
        let (_tmp, store) = setup();
        let mut alice = make_alice();
        alice.aliases.push("ally".into());
        store.create(&alice).unwrap();

        let index = store.build_index().unwrap();
        assert_eq!(index.resolve_name("alice").unwrap(), alice.id);
        assert_eq!(index.resolve_name("ALLY").unwrap(), alice.id);
        assert_eq!(index.resolve_npub(&alice.npubs[0].npub).unwrap(), alice.id);
        assert!(index.resolve_name("nobody").is_none());
    }

    #[test]
    fn notes_crud() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        store.create(&alice).unwrap();

        assert!(store.get_notes(&alice.id).unwrap().is_none());

        store
            .set_notes(&alice.id, "Met at conference. Works on nostr.")
            .unwrap();
        let notes = store.get_notes(&alice.id).unwrap().unwrap();
        assert!(notes.contains("conference"));

        store.set_notes(&alice.id, "Updated notes.").unwrap();
        let notes = store.get_notes(&alice.id).unwrap().unwrap();
        assert_eq!(notes, "Updated notes.");
    }

    #[test]
    fn notes_on_nonexistent_fails() {
        let (_tmp, store) = setup();
        assert!(store.set_notes("nope", "text").is_err());
    }

    #[test]
    fn history_append_and_list() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        store.create(&alice).unwrap();

        store
            .append_history(&alice.id, "2024-01.md", "January summary.")
            .unwrap();
        store
            .append_history(&alice.id, "2024-02.md", "February summary.")
            .unwrap();
        store
            .append_history(&alice.id, "2024-01.md", "\nMore January notes.")
            .unwrap();

        let files = store.list_history(&alice.id).unwrap();
        assert_eq!(files, vec!["2024-01.md", "2024-02.md"]);

        // Check append worked
        let dir = store.entity_dir(&alice.id).join("history");
        let jan = std::fs::read_to_string(dir.join("2024-01.md")).unwrap();
        assert!(jan.contains("January summary."));
        assert!(jan.contains("More January notes."));
    }

    #[test]
    fn history_empty_when_none() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        store.create(&alice).unwrap();
        assert!(store.list_history(&alice.id).unwrap().is_empty());
    }

    #[test]
    fn organization_with_members() {
        let (_tmp, store) = setup();

        let alice = make_alice();
        let mut bob = Entity::new("bob");
        bob.add_npub(
            "npub1bobfake0000000000000000000000000000000000000000000000000000",
            None,
        );

        let mut org = Entity::new("nostr-dev-team");
        org.relationship = Some(Relationship {
            kind: RelationshipKind::Organization,
            trust: None,
        });
        org.member_ids = vec![alice.id.clone(), bob.id.clone()];

        store.create(&alice).unwrap();
        store.create(&bob).unwrap();
        store.create(&org).unwrap();

        let loaded = store.get(&org.id).unwrap().unwrap();
        assert_eq!(loaded.member_ids.len(), 2);
        assert!(loaded.member_ids.contains(&alice.id));
        assert!(loaded.member_ids.contains(&bob.id));
    }

    #[test]
    fn tags_roundtrip() {
        let (_tmp, store) = setup();
        let mut alice = make_alice();
        alice.tags.insert(
            "met_at".into(),
            toml::Value::String("conference 2024".into()),
        );
        alice.tags.insert(
            "topics".into(),
            toml::Value::Array(vec![
                toml::Value::String("rust".into()),
                toml::Value::String("nostr".into()),
            ]),
        );
        store.create(&alice).unwrap();

        let loaded = store.get(&alice.id).unwrap().unwrap();
        assert_eq!(
            loaded.tags.get("met_at").unwrap().as_str().unwrap(),
            "conference 2024"
        );
        let topics = loaded.tags.get("topics").unwrap().as_array().unwrap();
        assert_eq!(topics.len(), 2);
    }

    #[test]
    fn toml_serialization_format() {
        let mut alice = make_alice();
        alice.relationship = Some(Relationship {
            kind: RelationshipKind::Friend,
            trust: Some(0.8),
        });
        let toml_str = toml::to_string_pretty(&alice).unwrap();
        // Should be human-readable TOML
        assert!(toml_str.contains("name = \"alice\""));
        assert!(toml_str.contains("[[npubs]]"));
        assert!(toml_str.contains("[relationship]"));
    }

    #[test]
    fn entity_dir_is_accessible() {
        let (_tmp, store) = setup();
        let alice = make_alice();
        store.create(&alice).unwrap();

        let dir = store.entity_dir(&alice.id);
        assert!(dir.exists());
        assert!(dir.join(ENTITY_FILE).exists());

        // Agents can write arbitrary files here
        std::fs::write(dir.join("custom-agent-data.json"), "{}").unwrap();
        assert!(dir.join("custom-agent-data.json").exists());
    }
}
