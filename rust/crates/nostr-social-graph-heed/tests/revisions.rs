use nostr_social_graph::{NostrEvent, SocialGraph};
use nostr_social_graph_heed::HeedSocialGraph;
fn key(n: u8) -> String {
    format!("{n:064x}")
}
#[test]
fn persisted_ids_and_ties_survive_reopen_and_export() {
    let directory = tempfile::tempdir().unwrap();
    for kind in [3, 10000] {
        let mut graph = HeedSocialGraph::open(directory.path(), &key(0)).unwrap();
        let mut event = NostrEvent {
            pubkey: key(0),
            id: key(2),
            kind,
            created_at: 100,
            tags: vec![],
            content: String::new(),
            sig: String::new(),
        };
        graph.handle_event(&event, true, f64::INFINITY).unwrap();
        event.id = key(1);
        event.tags = vec![vec!["p".into(), key(3)]];
        graph.handle_event(&event, true, f64::INFINITY).unwrap();
        drop(graph);
        let graph = HeedSocialGraph::open(directory.path(), &key(0)).unwrap();
        let revision = graph.get_list_revision(&key(0), kind).unwrap().unwrap();
        assert_eq!(revision.id, key(1));
        assert!(revision.complete);
        let exported = SocialGraph::from_state(graph.export_state().unwrap()).unwrap();
        assert_eq!(exported.get_list_revision(&key(0), kind), Some(revision));
    }
}
