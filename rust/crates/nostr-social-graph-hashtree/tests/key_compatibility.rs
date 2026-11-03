use hashtree_core::{Cid, EncryptionMode, decrypt_object, encrypt_object, sha256};
use hashtree_fs::FsBlobStore;
use nostr_social_graph::SocialGraphBackend;
use nostr_social_graph_hashtree::{HashtreeSocialGraph, SnapshotManifest};
use std::fs;
use tempfile::TempDir;

#[test]
fn reopen_and_checkpoint_public_legacy_and_versioned_graphs() {
    // A graph with one follow, serialized by nostr-social-graph 3.0.1 and
    // encrypted by @hashtree/core 0.3.1 before versioned keys existed.
    let legacy: SnapshotManifest =
        serde_json::from_str(include_str!("fixtures/legacy-root.json")).unwrap();
    let legacy_cid = Cid::parse(&legacy.cid).unwrap();
    let legacy_blob = include_bytes!("fixtures/legacy-graph.chk");
    let bytes = decrypt_object(legacy_blob, &legacy_cid.key.unwrap()).unwrap();
    let mut roots = vec![
        (legacy_cid, legacy_blob.to_vec()),
        (Cid::public(sha256(&bytes)), bytes.clone()),
    ];
    for mode in [EncryptionMode::ChkV1, EncryptionMode::RndV1] {
        let (blob, key) = encrypt_object(&bytes, mode).unwrap();
        roots.push((Cid::encrypted(sha256(&blob), key), blob));
    }

    for (cid, blob) in roots {
        let dir = TempDir::new().unwrap();
        let store = FsBlobStore::new(dir.path().join("blobs")).unwrap();
        store.put_sync(cid.hash, &blob).unwrap();
        let manifest = SnapshotManifest {
            cid: cid.to_string(),
            ..legacy.clone()
        };
        fs::write(
            dir.path().join("social-graph-root.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let mut graph = HashtreeSocialGraph::open(dir.path(), &legacy.root_pubkey).unwrap();
        assert_eq!(graph.latest_cid(), Some(cid.clone()));
        assert!(
            graph
                .is_following(&legacy.root_pubkey, &"22".repeat(32))
                .unwrap()
        );
        graph.replace_state(&graph.export_state()).unwrap();
        assert!(graph.latest_cid().unwrap().key.is_none());
        assert!(graph.snapshot_exists(&cid).unwrap());
        let reopened = HashtreeSocialGraph::open(dir.path(), &legacy.root_pubkey).unwrap();
        assert!(
            reopened
                .is_following(&legacy.root_pubkey, &"22".repeat(32))
                .unwrap()
        );
    }
}
