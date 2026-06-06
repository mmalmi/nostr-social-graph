use nostr_social_graph::{SocialGraph, SocialGraphBackend};
use nostr_social_graph_hashtree::HashtreeSocialGraph;
use nostr_social_graph_server::{load_or_bootstrap_graph, persist_graph_snapshot};
use tempfile::TempDir;

const ADAM: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";
const FIATJAF: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const SNOWDEN: &str = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";

#[test]
fn bootstrapping_imports_existing_binary_snapshot_into_hashtree() {
    let tempdir = TempDir::new().unwrap();
    let db_dir = tempdir.path().join("socialGraph.hashtree");
    let binary_path = tempdir.path().join("socialGraph.large.bin");

    let mut original = SocialGraph::new(ADAM);
    original.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0);
    original.handle_event(&event(FIATJAF, 3, 1_100, vec![SNOWDEN]), true, 1.0);
    std::fs::write(&binary_path, original.to_binary().unwrap()).unwrap();

    let graph = load_or_bootstrap_graph(ADAM, &db_dir, Some(binary_path.as_path())).unwrap();
    assert!(graph.is_following(ADAM, FIATJAF));
    assert!(graph.is_following(FIATJAF, SNOWDEN));

    let reopened = HashtreeSocialGraph::open(&db_dir, ADAM).unwrap();
    assert!(reopened.is_following(ADAM, FIATJAF).unwrap());
    assert!(reopened.is_following(FIATJAF, SNOWDEN).unwrap());
    assert!(reopened.latest_cid().is_some());
}

#[test]
fn persisting_snapshot_updates_hashtree_without_writing_binary_snapshot() {
    let tempdir = TempDir::new().unwrap();
    let db_dir = tempdir.path().join("socialGraph.hashtree");
    let binary_path = tempdir.path().join("socialGraph.large.bin");

    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(FIATJAF, 3, 1_100, vec![SNOWDEN]), true, 1.0);

    persist_graph_snapshot(ADAM, &db_dir, &graph).unwrap();

    let reopened = HashtreeSocialGraph::open(&db_dir, ADAM).unwrap();
    assert!(reopened.is_following(ADAM, FIATJAF).unwrap());
    assert!(reopened.is_following(FIATJAF, SNOWDEN).unwrap());
    assert!(reopened.latest_cid().is_some());
    assert!(!binary_path.exists());
}

fn event(
    pubkey: &str,
    kind: u32,
    created_at: u64,
    tagged: Vec<&str>,
) -> nostr_social_graph::NostrEvent {
    nostr_social_graph::NostrEvent {
        created_at,
        content: String::new(),
        tags: tagged
            .into_iter()
            .map(|pk| vec!["p".to_string(), pk.to_string()])
            .collect(),
        kind,
        pubkey: pubkey.to_string(),
        id: format!("{pubkey}:{kind}:{created_at}"),
        sig: "00".repeat(64),
    }
}
