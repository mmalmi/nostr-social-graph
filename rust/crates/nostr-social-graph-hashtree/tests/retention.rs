use futures::executor::block_on;
use hashtree_core::{HashTree, HashTreeConfig, Store, collect_hashes};
use hashtree_fs::FsBlobStore;
use nostr_social_graph::{NostrEvent, SocialGraph, SocialGraphBackend};
use nostr_social_graph_hashtree::HashtreeSocialGraph;
use std::collections::HashSet;
use std::sync::Arc;
use tempfile::TempDir;

const ROOT: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";

fn update(graph: &mut HashtreeSocialGraph, revision: u64) {
    graph
        .handle_event(
            &NostrEvent {
                pubkey: ROOT.into(),
                kind: 3,
                created_at: revision,
                tags: vec![vec!["p".into(), format!("{revision:064x}")]],
                content: String::new(),
                id: format!("{revision:064x}"),
                sig: "00".repeat(64),
            },
            true,
            1.0,
        )
        .unwrap();
}

#[test]
fn repeated_checkpoints_retain_only_current_previous_and_explicit_pins() {
    let dir = TempDir::new().unwrap();
    let blobs = Arc::new(FsBlobStore::new(dir.path().join("blobs")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(blobs.clone()).public());
    let (pinned, _) = block_on(tree.put(b"explicit retained data")).unwrap();
    block_on(blobs.pin(&pinned.hash)).unwrap();
    let mut roots = Vec::new();
    for revision in 1..=12 {
        let mut graph = HashtreeSocialGraph::open(dir.path(), ROOT).unwrap();
        update(&mut graph, revision);
        graph.flush().unwrap();
        roots.push(graph.latest_cid().unwrap());
        assert!(
            graph
                .is_following(ROOT, &format!("{revision:064x}"))
                .unwrap()
        );
    }
    let mut retained = HashSet::from([pinned.hash]);
    for root in roots.iter().rev().take(2) {
        retained.extend(block_on(collect_hashes(&tree, root, 1)).unwrap());
        assert!(block_on(tree.get(root, None)).unwrap().is_some());
    }
    let actual: HashSet<_> = blobs.list().unwrap().into_iter().collect();
    assert_eq!(actual, retained, "obsolete snapshots must not accumulate");
    assert!(dir.path().join("social-graph-root.previous.json").is_file());
    // Saving an identical state must not replace the distinct rollback root.
    let previous = std::fs::read(dir.path().join("social-graph-root.previous.json")).unwrap();
    let mut graph = HashtreeSocialGraph::open(dir.path(), ROOT).unwrap();
    graph.replace_state(&graph.export_state()).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("social-graph-root.previous.json")).unwrap(),
        previous
    );
}

#[test]
fn stale_writer_cannot_replace_another_checkpoint_or_collect_its_blobs() {
    let dir = TempDir::new().unwrap();
    let mut first = HashtreeSocialGraph::open(dir.path(), ROOT).unwrap();
    let mut stale = HashtreeSocialGraph::open(dir.path(), ROOT).unwrap();
    update(&mut first, 1);
    first.flush().unwrap();
    let before = std::fs::read(dir.path().join("social-graph-root.json")).unwrap();
    update(&mut stale, 2);
    assert!(stale.flush().is_err(), "stale writer must be rejected");
    assert_eq!(
        std::fs::read(dir.path().join("social-graph-root.json")).unwrap(),
        before
    );
}

#[test]
fn corrupt_current_snapshot_prevents_publication_and_cleanup() {
    let dir = TempDir::new().unwrap();
    let mut graph = HashtreeSocialGraph::open(dir.path(), ROOT).unwrap();
    update(&mut graph, 1);
    graph.flush().unwrap();
    let blobs = FsBlobStore::new(dir.path().join("blobs")).unwrap();
    blobs
        .delete_sync(&graph.latest_cid().unwrap().hash)
        .unwrap();
    let before = std::fs::read(dir.path().join("social-graph-root.json")).unwrap();
    update(&mut graph, 2);
    assert!(graph.flush().is_err());
    assert_eq!(
        std::fs::read(dir.path().join("social-graph-root.json")).unwrap(),
        before
    );
}

#[test]
fn chunked_checkpoints_and_rollback_survive_collection() {
    let dir = TempDir::new().unwrap();
    let blobs = Arc::new(FsBlobStore::new(dir.path().join("blobs")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(blobs.clone()).public());
    let mut roots = Vec::new();
    for revision in 1..=4 {
        let mut graph = HashtreeSocialGraph::open(dir.path(), ROOT).unwrap();
        graph
            .handle_event(
                &NostrEvent {
                    pubkey: ROOT.into(),
                    kind: 3,
                    created_at: revision,
                    tags: (revision..70_000 + revision)
                        .map(|key| vec!["p".into(), format!("{key:064x}")])
                        .collect(),
                    content: String::new(),
                    id: format!("{revision:064x}"),
                    sig: "00".repeat(64),
                },
                true,
                1.0,
            )
            .unwrap();
        graph.flush().unwrap();
        roots.push(graph.latest_cid().unwrap());
    }
    let mut retained = HashSet::new();
    for root in roots.iter().rev().take(2) {
        let hashes = block_on(collect_hashes(&tree, root, 1)).unwrap();
        assert!(hashes.len() > 1, "exercise a multi-blob snapshot");
        retained.extend(hashes);
        let data = block_on(tree.get(root, None)).unwrap().unwrap();
        let graph = SocialGraph::from_binary(ROOT, &data).unwrap();
        assert!(graph.is_following(ROOT, &format!("{:064x}", 100)));
        assert_eq!(graph.get_followed_by_user(ROOT).len(), 70_000);
    }
    assert_eq!(
        blobs.list().unwrap().into_iter().collect::<HashSet<_>>(),
        retained
    );
    // Restoring the saved manifest is sufficient to reopen the rollback root.
    std::fs::copy(
        dir.path().join("social-graph-root.previous.json"),
        dir.path().join("social-graph-root.json"),
    )
    .unwrap();
    let rollback = HashtreeSocialGraph::open(dir.path(), ROOT).unwrap();
    assert!(rollback.is_following(ROOT, &format!("{:064x}", 3)).unwrap());
}
