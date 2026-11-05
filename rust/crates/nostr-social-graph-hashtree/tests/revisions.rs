use nostr_social_graph::{NostrEvent, SocialGraphBackend};
use nostr_social_graph_hashtree::HashtreeSocialGraph;
fn key(n: u8) -> String {
    format!("{n:064x}")
}
#[test]
fn revisions_survive_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let mut graph = HashtreeSocialGraph::open(directory.path(), &key(0)).unwrap();
    for kind in [3, 10000] {
        graph
            .handle_event(
                &NostrEvent {
                    pubkey: key(0),
                    id: key(kind as u8),
                    kind,
                    created_at: 100,
                    tags: vec![],
                    content: String::new(),
                    sig: String::new(),
                },
                true,
                f64::INFINITY,
            )
            .unwrap();
    }
    graph.flush().unwrap();
    drop(graph);
    let graph = HashtreeSocialGraph::open(directory.path(), &key(0)).unwrap();
    for kind in [3, 10000] {
        assert!(graph.get_list_revision(&key(0), kind).unwrap().complete);
    }
}
