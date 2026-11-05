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
