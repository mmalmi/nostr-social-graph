use nostr_social_graph::{BinaryBudget, NostrEvent, SocialGraph};
fn key(n: u8) -> String {
    format!("{n:064x}")
}
fn event(kind: u32, id: u8, at: u64, targets: &[u8]) -> NostrEvent {
    NostrEvent {
        pubkey: key(0),
        kind,
        id: key(id),
        created_at: at,
        tags: targets.iter().map(|n| vec!["p".into(), key(*n)]).collect(),
        content: String::new(),
        sig: String::new(),
    }
}
#[test]
fn revisions_roundtrip_and_equal_time_winner() {
    for kind in [3, 10000] {
        let mut graph = SocialGraph::new(&key(0));
        graph.handle_event(&event(kind, 2, 100, &[1, 2]), true, f64::INFINITY);
        graph.handle_event(&event(kind, 3, 100, &[]), true, f64::INFINITY);
        assert_eq!(graph.get_list_revision(&key(0), kind).unwrap().id, key(2));
        graph.handle_event(&event(kind, 1, 100, &[]), true, f64::INFINITY);
        let restored = SocialGraph::from_binary(&key(0), &graph.to_binary().unwrap()).unwrap();
        let revision = restored.get_list_revision(&key(0), kind).unwrap();
        assert_eq!(revision.id, key(1));
        assert!(revision.complete);
    }
}
#[test]
fn projection_is_incomplete_and_repaired_by_same_event() {
    for kind in [3, 10000] {
        let mut graph = SocialGraph::new(&key(0));
        let ev = event(kind, 2, 100, &[1, 2]);
        graph.handle_event(&ev, true, f64::INFINITY);
        let binary = graph
            .to_binary_with_budget(BinaryBudget {
                max_edges: Some(1),
                ..Default::default()
            })
            .unwrap();
        let mut restored = SocialGraph::from_binary(&key(0), &binary).unwrap();
        assert!(!restored.get_list_revision(&key(0), kind).unwrap().complete);
        restored.handle_event(&ev, true, f64::INFINITY);
        assert!(restored.get_list_revision(&key(0), kind).unwrap().complete);
    }
}

#[test]
fn legacy_export_is_readable_without_revision_metadata() {
    let mut graph = SocialGraph::new(&key(0));
    graph.handle_event(&event(3, 2, 100, &[1, 2]), true, f64::INFINITY);
    let bytes = graph
        .to_binary_with_budget_version(BinaryBudget::default(), 3)
        .unwrap();
    assert_eq!(bytes[0], 3);
    let restored = SocialGraph::from_binary(&key(0), &bytes).unwrap();
    assert_eq!(
        restored.get_followed_by_user(&key(0)),
        graph.get_followed_by_user(&key(0))
    );
    assert!(restored.get_list_revisions().is_empty());
}

#[test]
fn malformed_counts_and_varints_are_rejected_before_allocation() {
    for version in [2, 3, 4] {
        assert!(SocialGraph::from_binary(&key(0), &[version, 0, 255, 255, 255, 255, 7]).is_err());
        let mut bytes = vec![version];
        bytes.extend([255; 20]);
        assert!(SocialGraph::from_binary(&key(0), &bytes).is_err());
    }
}

#[test]
fn alternate_root_projection_preserves_source_and_caps_working_graph() {
    let mut graph = SocialGraph::new(&key(0));
    graph.handle_event(&event(3, 2, 100, &[1, 2]), true, f64::INFINITY);
    let mut other = event(3, 3, 101, &[3, 4, 5]);
    other.pubkey = key(7);
    graph.handle_event(&other, true, f64::INFINITY);
    let bytes = graph
        .to_binary_for_root_with_budget(
            &key(7),
            BinaryBudget {
                max_nodes: Some(3),
                max_edges: Some(2),
                max_distance: Some(1),
                max_edges_per_node: Some(2),
            },
        )
        .unwrap();
    let projected = SocialGraph::from_binary(&key(7), &bytes).unwrap();
    assert_eq!(
        projected.get_followed_by_user(&key(7)),
        vec![key(3), key(4)]
    );
    assert!(!projected.get_list_revision(&key(7), 3).unwrap().complete);
    assert_eq!(graph.get_root(), key(0));
    assert_eq!(graph.get_followed_by_user(&key(7)).len(), 3);
    assert!(graph.get_list_revision(&key(7), 3).unwrap().complete);
}
