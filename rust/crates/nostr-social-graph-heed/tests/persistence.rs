use heed::byteorder::BigEndian;
use heed::types::{Bytes, SerdeBincode, Str, U32};
use heed::{Database, EnvOpenOptions};
use nostr_social_graph::NostrEvent;
use nostr_social_graph_heed::HeedSocialGraph;
use tempfile::TempDir;

const ADAM: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";
const FIATJAF: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const SNOWDEN: &str = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";
const SIRIUS: &str = "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0";

#[test]
fn writes_are_immediately_visible_and_survive_reopen() {
    let tempdir = TempDir::new().unwrap();
    let mut store = HeedSocialGraph::open(tempdir.path(), ADAM).unwrap();
    store
        .handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0)
        .unwrap();
    store
        .handle_event(&event(FIATJAF, 3, 1_100, vec![SNOWDEN]), true, 1.0)
        .unwrap();
    assert!(!store.has_unflushed_changes());
    assert!(store.is_following(ADAM, FIATJAF).unwrap());
    assert!(store.is_following(FIATJAF, SNOWDEN).unwrap());
    assert_eq!(store.get_follow_distance(ADAM).unwrap(), 0);
    assert_eq!(store.get_follow_distance(FIATJAF).unwrap(), 1);
    assert_eq!(store.get_follow_distance(SNOWDEN).unwrap(), 2);
    drop(store);

    let reopened = HeedSocialGraph::open(tempdir.path(), SIRIUS).unwrap();
    assert_eq!(reopened.get_root().unwrap(), ADAM);
    assert!(reopened.is_following(ADAM, FIATJAF).unwrap());
    assert!(reopened.is_following(FIATJAF, SNOWDEN).unwrap());
    assert_eq!(reopened.get_follow_distance(ADAM).unwrap(), 0);
    assert_eq!(reopened.get_follow_distance(FIATJAF).unwrap(), 1);
    assert_eq!(reopened.get_follow_distance(SNOWDEN).unwrap(), 2);
}

#[test]
fn writes_materialize_graph_in_lmdb_tables_without_flush() {
    let tempdir = TempDir::new().unwrap();
    let mut store = HeedSocialGraph::open(tempdir.path(), ADAM).unwrap();
    store
        .handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0)
        .unwrap();
    store
        .handle_event(&event(FIATJAF, 3, 1_100, vec![SNOWDEN]), true, 1.0)
        .unwrap();
    drop(store);

    let env = unsafe {
        EnvOpenOptions::new()
            .map_size(4 * 1024 * 1024 * 1024)
            .max_dbs(16)
            .open(tempdir.path())
            .unwrap()
    };
    let rtxn = env.read_txn().unwrap();

    let metadata: Database<Str, Bytes> =
        env.open_database(&rtxn, Some("metadata")).unwrap().unwrap();
    let str_to_unique_id: Database<Str, U32<BigEndian>> = env
        .open_database(&rtxn, Some("str_to_unique_id"))
        .unwrap()
        .unwrap();
    let followed_by_user: Database<U32<BigEndian>, SerdeBincode<Vec<u32>>> = env
        .open_database(&rtxn, Some("followed_by_user"))
        .unwrap()
        .unwrap();
    let followers_by_user: Database<U32<BigEndian>, SerdeBincode<Vec<u32>>> = env
        .open_database(&rtxn, Some("followers_by_user"))
        .unwrap()
        .unwrap();
    let follow_distance_by_user: Database<U32<BigEndian>, U32<BigEndian>> = env
        .open_database(&rtxn, Some("follow_distance_by_user"))
        .unwrap()
        .unwrap();

    assert_eq!(
        std::str::from_utf8(metadata.get(&rtxn, "root").unwrap().unwrap()).unwrap(),
        ADAM
    );
    assert!(metadata.get(&rtxn, "snapshot").unwrap().is_none());

    let adam_id = str_to_unique_id.get(&rtxn, ADAM).unwrap().unwrap();
    let fiatjaf_id = str_to_unique_id.get(&rtxn, FIATJAF).unwrap().unwrap();
    let snowden_id = str_to_unique_id.get(&rtxn, SNOWDEN).unwrap().unwrap();

    assert_eq!(
        followed_by_user.get(&rtxn, &adam_id).unwrap().unwrap(),
        vec![fiatjaf_id]
    );
    assert_eq!(
        followers_by_user.get(&rtxn, &fiatjaf_id).unwrap().unwrap(),
        vec![adam_id]
    );
    assert_eq!(
        follow_distance_by_user.get(&rtxn, &adam_id).unwrap(),
        Some(0)
    );
    assert_eq!(
        follow_distance_by_user.get(&rtxn, &fiatjaf_id).unwrap(),
        Some(1)
    );
    assert_eq!(
        follow_distance_by_user.get(&rtxn, &snowden_id).unwrap(),
        Some(2)
    );
}

#[test]
fn root_changes_persist_without_flush() {
    let tempdir = TempDir::new().unwrap();
    let mut store = HeedSocialGraph::open(tempdir.path(), ADAM).unwrap();
    store
        .handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0)
        .unwrap();
    store
        .handle_event(&event(FIATJAF, 3, 1_100, vec![SNOWDEN]), true, 1.0)
        .unwrap();
    store.set_root(FIATJAF).unwrap();
    assert!(!store.has_unflushed_changes());
    assert_eq!(store.get_root().unwrap(), FIATJAF);
    assert_eq!(store.get_follow_distance(FIATJAF).unwrap(), 0);
    assert_eq!(store.get_follow_distance(SNOWDEN).unwrap(), 1);
    assert_eq!(store.get_follow_distance(ADAM).unwrap(), 1000);
    drop(store);

    let reopened = HeedSocialGraph::open(tempdir.path(), ADAM).unwrap();
    assert_eq!(reopened.get_root().unwrap(), FIATJAF);
    assert_eq!(reopened.get_follow_distance(FIATJAF).unwrap(), 0);
    assert_eq!(reopened.get_follow_distance(SNOWDEN).unwrap(), 1);
    assert_eq!(reopened.get_follow_distance(ADAM).unwrap(), 1000);
}

#[test]
fn overmute_queries_read_from_lmdb_state() {
    let tempdir = TempDir::new().unwrap();
    let mut store = HeedSocialGraph::open(tempdir.path(), ADAM).unwrap();
    store
        .handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF, SNOWDEN]), true, 1.0)
        .unwrap();
    store
        .handle_event(&event(FIATJAF, 10_000, 1_100, vec![SIRIUS]), true, 1.0)
        .unwrap();
    store
        .handle_event(&event(SNOWDEN, 10_000, 1_101, vec![SIRIUS]), true, 1.0)
        .unwrap();

    assert!(store.is_overmuted(SIRIUS, 1.0).unwrap());
    drop(store);

    let reopened = HeedSocialGraph::open(tempdir.path(), ADAM).unwrap();
    assert!(reopened.is_overmuted(SIRIUS, 1.0).unwrap());
}

#[test]
fn overmute_early_exit_and_deduped_follow_list_preserve_semantics() {
    let tempdir = TempDir::new().unwrap();
    let mut store = HeedSocialGraph::open(tempdir.path(), ADAM).unwrap();
    store
        .handle_event(
            &event(ADAM, 3, 1_000, vec![FIATJAF, SNOWDEN, FIATJAF]),
            true,
            1.0,
        )
        .unwrap();
    // Distance-1 follower (FIATJAF) beats distance-2 muter (SIRIUS)
    store
        .handle_event(&event(FIATJAF, 3, 1_001, vec![SNOWDEN, SIRIUS]), true, 1.0)
        .unwrap();
    store
        .handle_event(&event(SIRIUS, 10_000, 1_002, vec![SNOWDEN]), true, 1.0)
        .unwrap();

    assert_eq!(
        store.get_followed_by_user(ADAM).unwrap(),
        vec![FIATJAF.to_string(), SNOWDEN.to_string()]
    );
    assert!(!store.is_overmuted(SNOWDEN, 1.0).unwrap());
    assert!(!store.is_overmuted(SNOWDEN, f64::INFINITY).unwrap());
    assert!(!store.is_overmuted(SNOWDEN, 0.0).unwrap());
    assert!(!store.is_overmuted(SNOWDEN, f64::NAN).unwrap());
    assert!(
        !store
            .is_overmuted(&ADAM.to_uppercase(), f64::INFINITY)
            .unwrap()
    );
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
