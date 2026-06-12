use crate::rating::Rating;
use crate::store::EntityStore;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

const RATINGS_DIR: &str = "ratings";
const BY_NPUB_DIR: &str = "_by_npub";

impl EntityStore {
    /// Add a rating to an entity's ratings directory.
    pub fn add_rating(&self, entity_id: &str, rating: &Rating) -> Result<()> {
        let dir = self.entity_dir(entity_id).join(RATINGS_DIR);
        if !self.entity_dir(entity_id).exists() {
            anyhow::bail!("Entity {} does not exist", entity_id);
        }
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.toml", rating.id));
        let content = toml::to_string_pretty(rating).context("Failed to serialize rating")?;
        fs::write(&path, content)?;
        Ok(())
    }

    /// List all ratings for an entity.
    pub fn list_ratings(&self, entity_id: &str) -> Result<Vec<Rating>> {
        let dir = self.entity_dir(entity_id).join(RATINGS_DIR);
        read_rating_files(&dir)
    }

    /// Remove a rating by ID from an entity.
    pub fn remove_rating(&self, entity_id: &str, rating_id: &str) -> Result<bool> {
        let path = self
            .entity_dir(entity_id)
            .join(RATINGS_DIR)
            .join(format!("{}.toml", rating_id));
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)?;
        Ok(true)
    }

    /// Add a rating for an npub. If the npub belongs to a known entity,
    /// store it there. Otherwise, store under `_by_npub/{npub}/ratings/`.
    pub fn add_rating_for_npub(&self, npub: &str, rating: &Rating) -> Result<()> {
        if let Some(entity) = self.find_by_npub(npub)? {
            return self.add_rating(&entity.id, rating);
        }
        let dir = self.npub_ratings_dir(npub);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.toml", rating.id));
        let content = toml::to_string_pretty(rating).context("Failed to serialize rating")?;
        fs::write(&path, content)?;
        Ok(())
    }

    /// List ratings for an npub. Checks entity first, falls back to orphan storage.
    pub fn list_ratings_for_npub(&self, npub: &str) -> Result<Vec<Rating>> {
        if let Some(entity) = self.find_by_npub(npub)? {
            return self.list_ratings(&entity.id);
        }
        let dir = self.npub_ratings_dir(npub);
        read_rating_files(&dir)
    }

    /// Move orphan ratings from `_by_npub/{npub}/` into an entity's directory.
    pub fn adopt_npub_ratings(&self, entity_id: &str, npub: &str) -> Result<u32> {
        if !self.entity_dir(entity_id).exists() {
            anyhow::bail!("Entity {} does not exist", entity_id);
        }
        let orphan_dir = self.npub_ratings_dir(npub);
        if !orphan_dir.exists() {
            return Ok(0);
        }
        let entity_rat_dir = self.entity_dir(entity_id).join(RATINGS_DIR);
        fs::create_dir_all(&entity_rat_dir)?;
        let mut count = 0u32;
        for entry in fs::read_dir(&orphan_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let dest = entity_rat_dir.join(entry.file_name());
                fs::rename(entry.path(), dest)?;
                count += 1;
            }
        }
        let npub_dir = self.base_dir().join(BY_NPUB_DIR).join(npub);
        let _ = fs::remove_dir(&orphan_dir);
        let _ = fs::remove_dir(&npub_dir);
        Ok(count)
    }

    fn npub_ratings_dir(&self, npub: &str) -> PathBuf {
        self.base_dir()
            .join(BY_NPUB_DIR)
            .join(npub)
            .join(RATINGS_DIR)
    }
}

fn read_rating_files(dir: &std::path::Path) -> Result<Vec<Rating>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut ratings = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            let content = fs::read_to_string(&path)?;
            let r: Rating = toml::from_str(&content)
                .with_context(|| format!("Failed to parse rating {}", path.display()))?;
            ratings.push(r);
        }
    }
    ratings.sort_by_key(|rating| rating.created_at);
    Ok(ratings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rating::*;
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

    fn make_positive(rater: &str) -> Rating {
        Rating::new(rater, "npub1target", Sentiment::Positive)
    }

    fn make_negative(rater: &str) -> Rating {
        let mut r = Rating::new(rater, "npub1target", Sentiment::Negative);
        r.context = Some("spam".into());
        r.report_type = Some(ReportType::Spam);
        r
    }

    #[test]
    fn add_and_list() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        store
            .add_rating(&alice.id, &make_positive("npub1bob"))
            .unwrap();
        store
            .add_rating(&alice.id, &make_negative("npub1charlie"))
            .unwrap();

        let list = store.list_ratings(&alice.id).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn add_nonexistent_entity_fails() {
        let (_tmp, store) = setup();
        assert!(
            store
                .add_rating("nonexistent", &make_positive("r"))
                .is_err()
        );
    }

    #[test]
    fn remove() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let r = make_positive("npub1bob");
        let rid = r.id.clone();
        store.add_rating(&alice.id, &r).unwrap();

        assert!(store.remove_rating(&alice.id, &rid).unwrap());
        assert!(!store.remove_rating(&alice.id, &rid).unwrap());
        assert!(store.list_ratings(&alice.id).unwrap().is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();
        assert!(!store.remove_rating(&alice.id, "no-such-id").unwrap());
    }

    #[test]
    fn list_empty() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();
        assert!(store.list_ratings(&alice.id).unwrap().is_empty());
    }

    #[test]
    fn list_nonexistent_entity_returns_empty() {
        let (_tmp, store) = setup();
        assert!(store.list_ratings("nonexistent").unwrap().is_empty());
    }

    #[test]
    fn for_known_npub_goes_to_entity() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let r = make_positive("npub1bob");
        store.add_rating_for_npub("npub1alice", &r).unwrap();

        let list = store.list_ratings(&alice.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, r.id);
    }

    #[test]
    fn for_unknown_npub_goes_to_orphan() {
        let (_tmp, store) = setup();

        let r = make_positive("npub1bob");
        store.add_rating_for_npub("npub1unknown", &r).unwrap();

        assert!(store.list().unwrap().is_empty());
        let list = store.list_ratings_for_npub("npub1unknown").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, r.id);
    }

    #[test]
    fn list_for_known_npub() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        store
            .add_rating(&alice.id, &make_positive("npub1bob"))
            .unwrap();

        let list = store.list_ratings_for_npub("npub1alice").unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn list_for_npub_no_data() {
        let (_tmp, store) = setup();
        assert!(
            store
                .list_ratings_for_npub("npub1nobody")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn adopt_moves_orphans() {
        let (_tmp, store) = setup();

        let r1 = make_positive("npub1bob");
        let r2 = make_negative("npub1charlie");
        store.add_rating_for_npub("npub1alice", &r1).unwrap();
        store.add_rating_for_npub("npub1alice", &r2).unwrap();

        assert_eq!(store.list_ratings_for_npub("npub1alice").unwrap().len(), 2);

        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let moved = store.adopt_npub_ratings(&alice.id, "npub1alice").unwrap();
        assert_eq!(moved, 2);
        assert_eq!(store.list_ratings(&alice.id).unwrap().len(), 2);

        let orphan_dir = store.base_dir().join(BY_NPUB_DIR).join("npub1alice");
        assert!(!orphan_dir.exists());
    }

    #[test]
    fn adopt_with_no_orphans() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();
        assert_eq!(
            store.adopt_npub_ratings(&alice.id, "npub1alice").unwrap(),
            0
        );
    }

    #[test]
    fn adopt_nonexistent_entity_fails() {
        let (_tmp, store) = setup();
        assert!(store.adopt_npub_ratings("nonexistent", "npub1x").is_err());
    }

    #[test]
    fn persists_with_tags_and_report_type() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let mut r = Rating::new("npub1bob", "npub1alice", Sentiment::Positive);
        r.context = Some("great relay op".into());
        r.tags = vec!["relay-op".into(), "infra".into()];
        let rid = r.id.clone();
        store.add_rating(&alice.id, &r).unwrap();

        let list = store.list_ratings(&alice.id).unwrap();
        assert_eq!(list.len(), 1);
        let loaded = &list[0];
        assert_eq!(loaded.id, rid);
        assert_eq!(loaded.rater, "npub1bob");
        assert!(loaded.is_positive());
        assert_eq!(loaded.tags, vec!["relay-op", "infra"]);
        assert_eq!(loaded.context, Some("great relay op".into()));
    }

    #[test]
    fn neutral_rating() {
        let (_tmp, store) = setup();
        let alice = make_entity("alice", "npub1alice");
        store.create(&alice).unwrap();

        let r = Rating::new("npub1bob", "npub1alice", Sentiment::Neutral);
        store.add_rating(&alice.id, &r).unwrap();

        let list = store.list_ratings(&alice.id).unwrap();
        assert_eq!(list.len(), 1);
        assert!(!list[0].is_positive());
        assert!(!list[0].is_negative());
    }
}
