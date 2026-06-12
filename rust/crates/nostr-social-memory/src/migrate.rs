use crate::store::EntityStore;
use crate::types::Entity;
use anyhow::Result;

/// Migrate a plain-text contacts file (ndr format) into entity directories.
///
/// Format: one contact per line, `npub1... petname`
/// Lines starting with # are comments, blank lines are ignored.
///
/// Returns the number of entities created.
pub fn migrate_contacts(contacts_content: &str, store: &EntityStore) -> Result<Vec<Entity>> {
    let mut created = Vec::new();
    for line in contacts_content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let npub = match parts.next() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let name = match parts.next() {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => continue,
        };

        // Skip if this npub is already in the store
        if store.find_by_npub(npub)?.is_some() {
            continue;
        }

        // Skip if name already exists — don't overwrite
        if store.find_by_name(name)?.is_some() {
            continue;
        }

        let mut entity = Entity::new(name);
        entity.add_npub(npub, Some("main".into()));
        store.create(&entity)?;
        created.push(entity);
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, EntityStore) {
        let tmp = TempDir::new().unwrap();
        let store = EntityStore::new(tmp.path().join("identities")).unwrap();
        (tmp, store)
    }

    #[test]
    fn migrate_basic_contacts() {
        let (_tmp, store) = setup();
        let content = "\
npub1alice00000000000000000000000000000000000000000000000000000 alice
npub1bob0000000000000000000000000000000000000000000000000000000 bob
";
        let created = migrate_contacts(content, &store).unwrap();
        assert_eq!(created.len(), 2);

        let alice = store.find_by_name("alice").unwrap().unwrap();
        assert_eq!(
            alice.npubs[0].npub,
            "npub1alice00000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(alice.npubs[0].label.as_deref(), Some("main"));

        let bob = store.find_by_name("bob").unwrap().unwrap();
        assert_eq!(bob.npubs.len(), 1);
    }

    #[test]
    fn migrate_skips_comments_and_blanks() {
        let (_tmp, store) = setup();
        let content = "\
# This is a comment
npub1alice00000000000000000000000000000000000000000000000000000 alice

# Another comment

npub1bob0000000000000000000000000000000000000000000000000000000 bob
";
        let created = migrate_contacts(content, &store).unwrap();
        assert_eq!(created.len(), 2);
    }

    #[test]
    fn migrate_skips_malformed_lines() {
        let (_tmp, store) = setup();
        let content = "\
npub1alice00000000000000000000000000000000000000000000000000000 alice
justaname
npub_only_no_name
npub1bob0000000000000000000000000000000000000000000000000000000 bob
";
        let created = migrate_contacts(content, &store).unwrap();
        assert_eq!(created.len(), 2);
    }

    #[test]
    fn migrate_skips_duplicate_npub() {
        let (_tmp, store) = setup();
        let content = "npub1alice00000000000000000000000000000000000000000000000000000 alice";
        migrate_contacts(content, &store).unwrap();

        // Migrate again with same npub under different name
        let content2 = "npub1alice00000000000000000000000000000000000000000000000000000 alice2";
        let created = migrate_contacts(content2, &store).unwrap();
        assert_eq!(created.len(), 0);
    }

    #[test]
    fn migrate_skips_duplicate_name() {
        let (_tmp, store) = setup();
        let content = "npub1alice00000000000000000000000000000000000000000000000000000 alice";
        migrate_contacts(content, &store).unwrap();

        // Migrate again with same name but different npub
        let content2 = "npub1different000000000000000000000000000000000000000000000000000 alice";
        let created = migrate_contacts(content2, &store).unwrap();
        assert_eq!(created.len(), 0);
    }

    #[test]
    fn migrate_empty_content() {
        let (_tmp, store) = setup();
        let created = migrate_contacts("", &store).unwrap();
        assert_eq!(created.len(), 0);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn migrate_idempotent() {
        let (_tmp, store) = setup();
        let content = "\
npub1alice00000000000000000000000000000000000000000000000000000 alice
npub1bob0000000000000000000000000000000000000000000000000000000 bob
";
        let first = migrate_contacts(content, &store).unwrap();
        assert_eq!(first.len(), 2);

        let second = migrate_contacts(content, &store).unwrap();
        assert_eq!(second.len(), 0);
        assert_eq!(store.list().unwrap().len(), 2);
    }
}
