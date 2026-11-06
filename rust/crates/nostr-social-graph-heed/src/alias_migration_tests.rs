use super::*;
use tempfile::TempDir;

fn key(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}
fn revision(n: u8, at: u64, complete: bool) -> ListRevision {
    ListRevision {
        id: key(n),
        created_at: at,
        complete,
    }
}
fn event(author: &str, kind: u32, at: u64, targets: &[String]) -> NostrEvent {
    NostrEvent {
        pubkey: author.into(),
        kind,
        created_at: at,
        id: key(at as u8),
        tags: targets
            .iter()
            .map(|p| vec!["p".into(), p.clone()])
            .collect(),
        sig: String::new(),
        content: String::new(),
    }
}
fn aliases() -> SocialGraphState {
    let mut s = SocialGraph::new(&key(10)).export_state();
    s.root = key(10).to_uppercase();
    s.unique_ids.extend([
        (key(11), 1),
        (key(11).to_uppercase(), 8),
        (key(12), 2),
        (key(12).to_uppercase(), 9),
        (key(13), 3),
    ]);
    s.followed_by_user = vec![(0, vec![8, 9]), (1, vec![3]), (8, vec![9])];
    s.follow_list_created_at = vec![(0, 1), (1, 10), (8, 10)];
    s.muted_by_user = vec![(1, vec![3]), (8, vec![9])];
    s.mute_list_created_at = vec![(1, 20), (8, 20)];
    s.list_revisions = vec![
        (1, 3, revision(15, 10, true)),
        (8, 3, revision(14, 10, false)),
        (1, 10_000, revision(13, 20, false)),
        (8, 10_000, revision(13, 20, true)),
    ];
    s
}

// Write the pre-migration table representation directly, including duplicate
// aliases and absent empty-list rows. This must not call replace_state.
fn legacy(path: &Path, s: &SocialGraphState) {
    let store = HeedSocialGraph::open(path, &key(10)).unwrap();
    let mut tx = store.env.write_txn().unwrap();
    persist_state(
        &mut tx,
        &store.metadata,
        &store.str_to_unique_id,
        &store.unique_id_to_str,
        &store.follow_distance_by_user,
        &store.followed_by_user,
        &store.followers_by_user,
        &store.follow_list_created_at,
        &store.muted_by_user,
        &store.user_muted_by,
        &store.mute_list_created_at,
        &store.users_by_follow_distance,
        s,
    )
    .unwrap();
    store.metadata.delete(&mut tx, "identity_schema").unwrap();
    tx.commit().unwrap();
    close(store);
}
fn close(store: HeedSocialGraph) {
    let env = store.env.clone();
    drop(store);
    env.prepare_for_closing().wait();
}
fn raw_env(path: &Path) -> Env {
    unsafe {
        EnvOpenOptions::new()
            .map_size(DEFAULT_MAP_SIZE)
            .max_dbs(MAX_DBS)
            .open(path)
            .unwrap()
    }
}
fn marker(path: &Path) -> Option<Vec<u8>> {
    let env = raw_env(path);
    let tx = env.read_txn().unwrap();
    let db: Database<Str, Bytes> = env.open_database(&tx, Some(METADATA_DB)).unwrap().unwrap();
    let result = db
        .get(&tx, "identity_schema")
        .unwrap()
        .map(ToOwned::to_owned);
    drop(tx);
    env.prepare_for_closing().wait();
    result
}

#[test]
fn mixed_case_public_api_uses_one_identity_and_preserves_nonhex_case() {
    let tmp = TempDir::new().unwrap();
    let mut graph = HeedSocialGraph::open(tmp.path(), &key(10).to_uppercase()).unwrap();
    assert_eq!(graph.get_root().unwrap(), key(10));
    graph
        .handle_event(
            &event(
                &key(10).to_uppercase(),
                3,
                1,
                &[key(11), key(11).to_uppercase(), key(10)],
            ),
            false,
            1.0,
        )
        .unwrap();
    graph
        .handle_event(
            &event(
                &key(11).to_uppercase(),
                10_000,
                2,
                &[key(12).to_uppercase()],
            ),
            false,
            1.0,
        )
        .unwrap();
    assert!(
        graph
            .is_following(&key(10).to_uppercase(), &key(11).to_uppercase())
            .unwrap()
    );
    assert_eq!(
        graph.get_followed_by_user(&key(10).to_uppercase()).unwrap(),
        vec![key(11)]
    );
    assert_eq!(
        graph
            .get_followers_by_user(&key(11).to_uppercase())
            .unwrap(),
        vec![key(10)]
    );
    assert_eq!(
        graph.get_follow_distance(&key(11).to_uppercase()).unwrap(),
        1
    );
    assert_eq!(
        graph
            .get_follow_list_created_at(&key(10).to_uppercase())
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        graph
            .get_mute_list_created_at(&key(11).to_uppercase())
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        graph.get_muted_by_user(&key(11).to_uppercase()).unwrap(),
        vec![key(12)]
    );
    assert_eq!(
        graph.get_user_muted_by(&key(12).to_uppercase()).unwrap(),
        vec![key(11)]
    );
    assert!(graph.is_overmuted(&key(12).to_uppercase(), 1.0).unwrap());
    assert_eq!(
        graph
            .get_list_revision(&key(11).to_uppercase(), 10_000)
            .unwrap()
            .unwrap()
            .id,
        key(2)
    );
    graph.set_root(&key(11).to_uppercase()).unwrap();
    assert_eq!(graph.get_root().unwrap(), key(11));
    graph.set_root("Alice").unwrap();
    graph.set_root("alice").unwrap();
    let ids = graph.export_state().unwrap().unique_ids;
    assert_eq!(ids.len(), 5);
    assert_ne!(
        ids.iter().find(|(p, _)| p == "Alice").unwrap().1,
        ids.iter().find(|(p, _)| p == "alice").unwrap().1
    );
}

#[test]
fn migration_repairs_indexes_edges_and_revision_winners_once() {
    let tmp = TempDir::new().unwrap();
    legacy(tmp.path(), &aliases());
    let graph = HeedSocialGraph::open(tmp.path(), "other-root").unwrap();
    assert_eq!(graph.get_root().unwrap(), key(10));
    assert_eq!(graph.export_state().unwrap().unique_ids.len(), 4);
    assert_eq!(graph.get_followed_by_user(&key(11)).unwrap(), vec![key(12)]);
    assert_eq!(
        graph.get_followers_by_user(&key(12)).unwrap(),
        vec![key(10), key(11)]
    );
    assert_eq!(graph.get_follow_distance(&key(12)).unwrap(), 1);
    assert_eq!(
        graph.get_list_revision(&key(11), 3).unwrap(),
        Some(revision(14, 10, false))
    );
    assert_eq!(
        graph.get_list_revision(&key(11), 10_000).unwrap(),
        Some(revision(13, 20, true))
    );
    assert_eq!(graph.get_user_muted_by(&key(12)).unwrap(), vec![key(11)]);
    let state = graph.export_state().unwrap();
    close(graph);
    assert_eq!(marker(tmp.path()), Some(vec![1]));
    let bytes = fs::read(tmp.path().join("data.mdb")).unwrap();
    let reopened = HeedSocialGraph::open(tmp.path(), "different-root").unwrap();
    assert_eq!(reopened.export_state().unwrap(), state);
    close(reopened);
    assert_eq!(fs::read(tmp.path().join("data.mdb")).unwrap(), bytes);
}

#[test]
fn timestamped_empty_lists_win_over_older_alias_rows() {
    let tmp = TempDir::new().unwrap();
    let mut s = aliases();
    s.followed_by_user.retain(|(id, _)| *id != 8);
    s.muted_by_user.retain(|(id, _)| *id != 8);
    s.follow_list_created_at
        .iter_mut()
        .find(|(id, _)| *id == 8)
        .unwrap()
        .1 = 30;
    s.mute_list_created_at
        .iter_mut()
        .find(|(id, _)| *id == 8)
        .unwrap()
        .1 = 30;
    s.list_revisions.retain(|(id, _, _)| *id != 8);
    s.list_revisions.extend([
        (8, 3, revision(1, 30, true)),
        (8, 10_000, revision(2, 30, true)),
    ]);
    legacy(tmp.path(), &s);
    let graph = HeedSocialGraph::open(tmp.path(), &key(10)).unwrap();
    assert!(graph.get_followed_by_user(&key(11)).unwrap().is_empty());
    assert!(graph.get_muted_by_user(&key(11)).unwrap().is_empty());
    assert!(graph.get_followers_by_user(&key(13)).unwrap().is_empty());
    assert_eq!(
        graph.get_list_revision(&key(11), 3).unwrap(),
        Some(revision(1, 30, true))
    );
    assert_eq!(
        graph.get_list_revision(&key(11), 10_000).unwrap(),
        Some(revision(2, 30, true))
    );
}

#[test]
fn equal_time_unknown_mutes_union_but_follow_conflicts_abort_unchanged() {
    let tmp = TempDir::new().unwrap();
    let mut s = aliases();
    s.followed_by_user.retain(|(id, _)| *id == 0);
    s.follow_list_created_at.retain(|(id, _)| *id == 0);
    s.list_revisions.clear();
    legacy(tmp.path(), &s);
    let graph = HeedSocialGraph::open(tmp.path(), &key(10)).unwrap();
    let muted = graph.get_muted_by_user(&key(11)).unwrap();
    assert_eq!(muted.len(), 2);
    assert!(muted.contains(&key(12)) && muted.contains(&key(13)));
    assert!(graph.get_list_revision(&key(11), 10_000).unwrap().is_none());
    close(graph);
    let mut conflict = aliases();
    conflict.list_revisions.clear();
    legacy(tmp.path(), &conflict);
    let bytes = fs::read(tmp.path().join("data.mdb")).unwrap();
    assert!(HeedSocialGraph::open(tmp.path(), &key(10)).is_err());
    assert_eq!(fs::read(tmp.path().join("data.mdb")).unwrap(), bytes);
    assert_eq!(marker(tmp.path()), None);
}

#[test]
fn migration_checks_forward_index_before_reverse_index_information_is_lost() {
    let tmp = TempDir::new().unwrap();
    let mut s = aliases();
    s.unique_ids.push((key(14), 1)); // Reverse row overwrites key(11), but forward row survives.
    legacy(tmp.path(), &s);
    let bytes = fs::read(tmp.path().join("data.mdb")).unwrap();
    assert!(HeedSocialGraph::open(tmp.path(), &key(10)).is_err());
    assert_eq!(fs::read(tmp.path().join("data.mdb")).unwrap(), bytes);
    assert_eq!(marker(tmp.path()), None);
}

#[test]
fn missing_reverse_entry_is_recovered_from_unambiguous_forward_index() {
    let tmp = TempDir::new().unwrap();
    legacy(tmp.path(), &aliases());
    let env = raw_env(tmp.path());
    let mut tx = env.write_txn().unwrap();
    let db: Database<U32<BigEndian>, Str> = env
        .open_database(&tx, Some(UNIQUE_ID_TO_STR_DB))
        .unwrap()
        .unwrap();
    db.delete(&mut tx, &9).unwrap();
    tx.commit().unwrap();
    env.prepare_for_closing().wait();
    let graph = HeedSocialGraph::open(tmp.path(), &key(10)).unwrap();
    assert!(graph.is_following(&key(10), &key(12)).unwrap());
    assert_eq!(graph.export_state().unwrap().unique_ids.len(), 4);
}

#[test]
fn replace_state_normalizes_before_persistence_and_conflicts_leave_previous_state() {
    let tmp = TempDir::new().unwrap();
    let mut graph = HeedSocialGraph::open(tmp.path(), &key(10)).unwrap();
    graph.replace_state(&aliases()).unwrap();
    assert_eq!(graph.export_state().unwrap().unique_ids.len(), 4);
    let good = graph.export_state().unwrap();
    let mut bad = aliases();
    bad.list_revisions.clear();
    assert!(graph.replace_state(&bad).is_err());
    assert_eq!(graph.export_state().unwrap(), good);
    bad = aliases();
    bad.unique_ids.push((key(14), 1));
    assert!(graph.replace_state(&bad).is_err());
    assert_eq!(graph.export_state().unwrap(), good);
}

#[test]
fn exhausted_ids_cannot_overwrite_an_existing_identity() {
    let tmp = TempDir::new().unwrap();
    let mut s = SocialGraph::new(&key(10)).export_state();
    s.unique_ids.push((key(11), u32::MAX));
    legacy(tmp.path(), &s);
    let mut graph = HeedSocialGraph::open(tmp.path(), &key(10)).unwrap();
    let before = graph.export_state().unwrap();
    assert!(
        graph
            .handle_event(&event(&key(10), 3, 1, &[key(12)]), true, 1.0)
            .is_err()
    );
    assert_eq!(graph.export_state().unwrap(), before);
}

#[test]
fn stale_counter_is_repaired_before_new_identity_allocation() {
    let tmp = TempDir::new().unwrap();
    legacy(tmp.path(), &aliases());
    let env = raw_env(tmp.path());
    let mut tx = env.write_txn().unwrap();
    let db: Database<Str, Bytes> = env.open_database(&tx, Some(METADATA_DB)).unwrap().unwrap();
    db.put(&mut tx, NEXT_UNIQUE_ID_KEY, &0u32.to_be_bytes())
        .unwrap();
    tx.commit().unwrap();
    env.prepare_for_closing().wait();
    let mut graph = HeedSocialGraph::open(tmp.path(), &key(10)).unwrap();
    graph
        .handle_event(&event(&key(10), 3, 31, &[key(14)]), true, 1.0)
        .unwrap();
    assert_eq!(graph.get_root().unwrap(), key(10));
    assert!(graph.is_following(&key(10), &key(14)).unwrap());
    assert_eq!(graph.export_state().unwrap().unique_ids.len(), 5);
}

#[test]
fn last_available_id_is_allocated_once_and_missing_root_cannot_reuse_it() {
    let tmp = TempDir::new().unwrap();
    let mut s = SocialGraph::new(&key(10)).export_state();
    s.unique_ids.push((key(11), u32::MAX - 1));
    legacy(tmp.path(), &s);
    let mut graph = HeedSocialGraph::open(tmp.path(), &key(10)).unwrap();
    graph.set_root(&key(12)).unwrap();
    assert!(
        graph
            .export_state()
            .unwrap()
            .unique_ids
            .contains(&(key(12), u32::MAX))
    );
    let before = graph.export_state().unwrap();
    assert!(graph.set_root(&key(13)).is_err());
    assert_eq!(graph.export_state().unwrap(), before);
    let mut bad = before.clone();
    bad.root = key(14);
    assert!(graph.replace_state(&bad).is_err());
    assert_eq!(graph.export_state().unwrap(), before);
}

#[test]
fn unknown_schema_and_unidentified_edges_leave_stored_bytes_unchanged() {
    let tmp = TempDir::new().unwrap();
    legacy(tmp.path(), &aliases());
    let env = raw_env(tmp.path());
    let mut tx = env.write_txn().unwrap();
    let db: Database<Str, Bytes> = env.open_database(&tx, Some(METADATA_DB)).unwrap().unwrap();
    db.put(&mut tx, "identity_schema", &[2]).unwrap();
    db.put(
        &mut tx,
        SNAPSHOT_KEY,
        &SocialGraph::new(&key(10)).to_binary().unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    env.prepare_for_closing().wait();
    let bytes = fs::read(tmp.path().join("data.mdb")).unwrap();
    assert!(matches!(
        HeedSocialGraph::open(tmp.path(), &key(10)),
        Err(HeedSocialGraphError::UnsupportedIdentitySchema)
    ));
    assert_eq!(fs::read(tmp.path().join("data.mdb")).unwrap(), bytes);
    let other = TempDir::new().unwrap();
    let mut s = aliases();
    s.followed_by_user.push((3, vec![1234]));
    legacy(other.path(), &s);
    let bytes = fs::read(other.path().join("data.mdb")).unwrap();
    assert!(HeedSocialGraph::open(other.path(), &key(10)).is_err());
    assert_eq!(fs::read(other.path().join("data.mdb")).unwrap(), bytes);
}

#[test]
fn map_full_during_replace_rolls_back_every_table_and_revision() {
    let tmp = TempDir::new().unwrap();
    let mut graph = unsafe {
        HeedSocialGraph::open_with_env_flags_and_map_size(
            tmp.path(),
            &key(10),
            EnvFlags::empty(),
            1024 * 1024,
        )
        .unwrap()
    };
    graph
        .handle_event(&event(&key(10), 3, 1, &[key(11)]), true, 1.0)
        .unwrap();
    graph
        .handle_event(&event(&key(10), 10_000, 2, &[key(12)]), true, 1.0)
        .unwrap();
    let before = graph.export_state().unwrap();
    let mut large = before.clone();
    large
        .unique_ids
        .extend((10..20_000).map(|id| (format!("{id:064x}"), id)));
    assert!(matches!(
        graph.replace_state(&large),
        Err(HeedSocialGraphError::Heed(heed::Error::Mdb(
            heed::MdbError::MapFull
        )))
    ));
    assert_eq!(graph.export_state().unwrap(), before);
    close(graph);
    let reopened = unsafe {
        HeedSocialGraph::open_with_env_flags_and_map_size(
            tmp.path(),
            &key(10),
            EnvFlags::empty(),
            1024 * 1024,
        )
        .unwrap()
    };
    assert_eq!(reopened.export_state().unwrap(), before);
}

#[test]
fn migration_map_full_aborts_then_recovers_after_capacity_is_restored() {
    let tmp = TempDir::new().unwrap();
    let store = unsafe {
        HeedSocialGraph::open_with_env_flags_and_map_size(
            tmp.path(),
            &key(10),
            EnvFlags::empty(),
            1024 * 1024,
        )
        .unwrap()
    };
    let env = store.env.clone();
    let mut tx = env.write_txn().unwrap();
    persist_state(
        &mut tx,
        &store.metadata,
        &store.str_to_unique_id,
        &store.unique_id_to_str,
        &store.follow_distance_by_user,
        &store.followed_by_user,
        &store.followers_by_user,
        &store.follow_list_created_at,
        &store.muted_by_user,
        &store.user_muted_by,
        &store.mute_list_created_at,
        &store.users_by_follow_distance,
        &aliases(),
    )
    .unwrap();
    store.metadata.delete(&mut tx, "identity_schema").unwrap();
    let pressure: Database<Str, Bytes> = env
        .create_database(&mut tx, Some("migration_pressure"))
        .unwrap();
    tx.commit().unwrap();
    drop(store);
    // Fill this synthetic environment only, committing each bounded block.
    let mut full = false;
    for n in 0..512 {
        let mut tx = env.write_txn().unwrap();
        let result = pressure
            .put(&mut tx, &format!("{n:04}"), &[0; 4096])
            .and_then(|()| tx.commit());
        match result {
            Ok(()) => {}
            Err(heed::Error::Mdb(heed::MdbError::MapFull)) => {
                full = true;
                break;
            }
            Err(error) => panic!("unexpected fixture failure: {error}"),
        }
    }
    assert!(full);
    let bytes = fs::read(tmp.path().join("data.mdb")).unwrap();
    assert!(matches!(
        HeedSocialGraph::open_env(env.clone(), &key(10)),
        Err(HeedSocialGraphError::Heed(heed::Error::Mdb(
            heed::MdbError::MapFull
        )))
    ));
    assert_eq!(fs::read(tmp.path().join("data.mdb")).unwrap(), bytes);
    // No live transaction or other process uses this task-owned environment.
    unsafe {
        env.resize(2 * 1024 * 1024).unwrap();
    }
    let graph = HeedSocialGraph::open_env(env.clone(), &key(10)).unwrap();
    assert_eq!(graph.export_state().unwrap().unique_ids.len(), 4);
    assert_eq!(
        graph.get_list_revision(&key(11), 3).unwrap(),
        Some(revision(14, 10, false))
    );
    assert_eq!(
        graph.get_list_revision(&key(11), 10_000).unwrap(),
        Some(revision(13, 20, true))
    );
    let tx = env.read_txn().unwrap();
    assert_eq!(
        graph.metadata.get(&tx, "identity_schema").unwrap(),
        Some([1].as_slice())
    );
}
