use std::fmt::Debug;

use nostr_social_graph::{NostrEvent, SocialGraph, SocialGraphBackend};
use nostr_social_graph_hashtree::HashtreeSocialGraph;
use tempfile::TempDir;

const ADAM: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";
const FIATJAF: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const SNOWDEN: &str = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";
const SIRIUS: &str = "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0";
const CHARLIE: &str = "f7c7b694f0d0e8cb2d19d7ec7d1a7932ff39fd8fbe6cb4f67a7fdbb2a9e5d0f1";

#[derive(Debug, PartialEq, Eq)]
struct BackendSummary {
    root: String,
    adam_follows_fiatjaf: bool,
    fiatjaf_follows_snowden: bool,
    distances: Vec<(&'static str, u32)>,
    follows: Vec<String>,
    followers: Vec<String>,
    muted_by: Vec<String>,
    muted_by_users: Vec<String>,
    follow_list_created_at: Option<u64>,
    mute_list_created_at: Option<u64>,
    sirius_overmuted: bool,
}

#[test]
fn memory_and_hashtree_backends_have_matching_runtime_behavior() {
    let mut memory = SocialGraph::new(ADAM);
    let memory_summary = exercise_backend(&mut memory).unwrap();

    let tempdir = TempDir::new().unwrap();
    let mut hashtree = HashtreeSocialGraph::open(tempdir.path(), ADAM).unwrap();
    let hashtree_summary = exercise_backend(&mut hashtree).unwrap();

    assert_eq!(hashtree_summary, memory_summary);
}

#[test]
fn flush_persists_snapshot_and_reopen_reads_it() {
    let tempdir = TempDir::new().unwrap();
    let mut store = HashtreeSocialGraph::open(tempdir.path(), ADAM).unwrap();

    store
        .handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0)
        .unwrap();
    store
        .handle_event(&event(FIATJAF, 3, 1_100, vec![SNOWDEN]), true, 1.0)
        .unwrap();
    assert!(store.has_unflushed_changes());
    store.flush().unwrap();
    assert!(!store.has_unflushed_changes());

    let cid = store.latest_cid().expect("flush should write a snapshot");
    assert!(cid.key.is_none(), "social graph snapshots should be public");
    assert!(store.snapshot_exists(&cid).unwrap());
    drop(store);

    let reopened = HashtreeSocialGraph::open(tempdir.path(), SIRIUS).unwrap();
    assert_eq!(reopened.get_root().unwrap(), ADAM);
    assert!(reopened.is_following(ADAM, FIATJAF).unwrap());
    assert!(reopened.is_following(FIATJAF, SNOWDEN).unwrap());
    assert_eq!(reopened.get_follow_distance(SNOWDEN).unwrap(), 2);
}

#[test]
fn unflushed_changes_are_visible_but_not_durable() {
    let tempdir = TempDir::new().unwrap();
    let mut store = HashtreeSocialGraph::open(tempdir.path(), ADAM).unwrap();

    store
        .handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0)
        .unwrap();
    assert!(store.has_unflushed_changes());
    assert!(store.is_following(ADAM, FIATJAF).unwrap());
    drop(store);

    let reopened = HashtreeSocialGraph::open(tempdir.path(), ADAM).unwrap();
    assert!(!reopened.is_following(ADAM, FIATJAF).unwrap());
}

fn exercise_backend<B>(backend: &mut B) -> Result<BackendSummary, B::Error>
where
    B: SocialGraphBackend,
    B::Error: Debug,
{
    backend.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF, SNOWDEN]), true, 1.0)?;
    backend.handle_event(&event(FIATJAF, 3, 1_100, vec![SNOWDEN, SIRIUS]), true, 1.0)?;
    backend.handle_event(&event(SNOWDEN, 3, 1_200, vec![CHARLIE]), true, 1.0)?;
    backend.handle_event(&event(FIATJAF, 10_000, 1_300, vec![SIRIUS]), true, 1.0)?;
    backend.handle_event(&event(SNOWDEN, 10_000, 1_301, vec![SIRIUS]), true, 1.0)?;
    backend.set_root(FIATJAF)?;
    backend.flush()?;

    let mut follows = backend.get_followed_by_user(FIATJAF)?;
    follows.sort();
    let mut followers = backend.get_followers_by_user(SNOWDEN)?;
    followers.sort();
    let mut muted_by = backend.get_muted_by_user(FIATJAF)?;
    muted_by.sort();
    let mut muted_by_users = backend.get_user_muted_by(SIRIUS)?;
    muted_by_users.sort();

    Ok(BackendSummary {
        root: backend.get_root()?,
        adam_follows_fiatjaf: backend.is_following(ADAM, FIATJAF)?,
        fiatjaf_follows_snowden: backend.is_following(FIATJAF, SNOWDEN)?,
        distances: vec![
            (ADAM, backend.get_follow_distance(ADAM)?),
            (FIATJAF, backend.get_follow_distance(FIATJAF)?),
            (SNOWDEN, backend.get_follow_distance(SNOWDEN)?),
            (SIRIUS, backend.get_follow_distance(SIRIUS)?),
            (CHARLIE, backend.get_follow_distance(CHARLIE)?),
        ],
        follows,
        followers,
        muted_by,
        muted_by_users,
        follow_list_created_at: backend.get_follow_list_created_at(FIATJAF)?,
        mute_list_created_at: backend.get_mute_list_created_at(FIATJAF)?,
        sirius_overmuted: backend.is_overmuted(SIRIUS, 1.0)?,
    })
}

fn event(pubkey: &str, kind: u32, created_at: u64, tagged: Vec<&str>) -> NostrEvent {
    NostrEvent {
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
