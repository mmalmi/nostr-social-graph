use super::*;

fn key(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}
fn binary(
    version: u64,
    ids: &[(u32, String)],
    follows: &[(u32, u64, Vec<u32>)],
    mutes: &[(u32, u64, Vec<u32>)],
) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(&mut out, version);
    write_varint(&mut out, ids.len() as u64);
    for (id, key) in ids {
        if version == 2 {
            out.extend(hex::decode(key).unwrap());
            write_varint(&mut out, *id as u64);
        } else {
            write_varint(&mut out, *id as u64);
            write_binary_node_id(&mut out, key);
        }
    }
    for lists in [follows, mutes] {
        write_varint(&mut out, lists.len() as u64);
        for (owner, at, targets) in lists {
            write_varint(&mut out, *owner as u64);
            write_varint(&mut out, *at);
            if version == 4 {
                out.push(0);
            }
            write_varint(&mut out, targets.len() as u64);
            for target in targets {
                write_varint(&mut out, *target as u64);
            }
        }
    }
    out
}

#[test]
fn legacy_aliases_preserve_all_incoming_edges_and_deduplicate_counts() {
    for version in [2, 3, 4] {
        let ids = [(0, key(10)), (1, key(11)), (2, key(12)), (7, key(12))];
        let bytes = binary(
            version,
            &ids,
            &[(0, 5, vec![7, 1]), (1, 6, vec![2, 7])],
            &[(0, 7, vec![2, 7])],
        );
        let graph = SocialGraph::from_binary(&key(10), &bytes).unwrap();
        assert!(graph.is_following(&key(10), &key(12)));
        assert_eq!(graph.get_follow_distance(&key(12)), 1);
        assert_eq!(graph.get_followers_by_user(&key(12)).len(), 2);
        assert_eq!(graph.get_muted_by_user(&key(10)).len(), 1);
        assert_eq!(graph.export_state().unique_ids.len(), 3);
        let restored = SocialGraph::from_binary(&key(10), &graph.to_binary().unwrap()).unwrap();
        assert_eq!(graph.to_binary().unwrap(), restored.to_binary().unwrap());
    }
}

#[test]
fn alias_owner_uses_newest_whole_list_including_empty_lists() {
    let ids = [(0, key(10)), (1, key(11)), (2, key(12)), (8, key(11))];
    let bytes = binary(
        3,
        &ids,
        &[(0, 5, vec![8]), (1, 10, vec![2]), (8, 11, vec![])],
        &[(1, 10, vec![2]), (8, 11, vec![])],
    );
    let graph = SocialGraph::from_binary(&key(10), &bytes).unwrap();
    assert!(graph.get_followed_by_user(&key(11)).is_empty());
    assert!(graph.get_muted_by_user(&key(11)).is_empty());
    assert_eq!(graph.get_follow_list_created_at(&key(11)), Some(11));
    assert!(graph.get_followers_by_user(&key(12)).is_empty());
}

#[test]
fn state_import_repairs_aliases_and_keeps_winning_revision() {
    let mut state = SocialGraph::new(&key(10)).export_state();
    state
        .unique_ids
        .extend([(key(11), 1), (key(11).to_uppercase(), 8), (key(12), 2)]);
    state.followed_by_user = vec![(0, vec![8]), (1, vec![2]), (8, vec![])];
    state.follow_list_created_at = vec![(0, 1), (1, 10), (8, 10)];
    state.list_revisions = vec![
        (
            1,
            3,
            ListRevision {
                id: key(15),
                created_at: 10,
                complete: true,
            },
        ),
        (
            8,
            3,
            ListRevision {
                id: key(14),
                created_at: 10,
                complete: true,
            },
        ),
    ];
    let graph = SocialGraph::from_state(state).unwrap();
    assert_eq!(graph.get_follow_distance(&key(11)), 1);
    assert!(graph.get_followed_by_user(&key(11)).is_empty());
    assert_eq!(graph.get_list_revision(&key(11), 3).unwrap().id, key(14));
    assert!(graph.get_list_revision(&key(11), 3).unwrap().complete);
}

#[test]
fn hex_identity_is_case_insensitive_but_arbitrary_ids_are_not() {
    let mut ids = UniqueIds::default();
    assert_eq!(
        ids.id(&key(10)).unwrap(),
        ids.id(&key(10).to_uppercase()).unwrap()
    );
    assert_eq!(
        ids.existing_id(&key(10).to_uppercase()),
        ids.existing_id(&key(10))
    );
    assert_ne!(ids.id("Alice").unwrap(), ids.id("alice").unwrap());
}

#[test]
fn unknown_equal_time_follow_conflicts_are_rejected_but_mutes_are_retained() {
    let ids = [
        (0, key(10)),
        (1, key(11)),
        (2, key(12)),
        (8, key(11)),
        (3, key(13)),
    ];
    let conflict = binary(
        3,
        &ids,
        &[(0, 1, vec![1]), (1, 10, vec![2]), (8, 10, vec![3])],
        &[],
    );
    assert!(matches!(
        SocialGraph::from_binary(&key(10), &conflict),
        Err(SocialGraphError::ConflictingFollowLists(1))
    ));
    let mutes = binary(
        3,
        &ids,
        &[(0, 1, vec![1])],
        &[(1, 10, vec![2]), (8, 10, vec![3])],
    );
    let graph = SocialGraph::from_binary(&key(10), &mutes).unwrap();
    assert_eq!(graph.get_muted_by_user(&key(11)).len(), 2);
    assert!(graph.get_list_revision(&key(11), 10_000).is_none());
}
