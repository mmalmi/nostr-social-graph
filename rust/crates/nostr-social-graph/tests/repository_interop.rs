mod support;

use std::fs;
use std::path::PathBuf;

use nostr_social_graph::{BinaryBudget, SocialGraph};
use support::*;
use tempfile::NamedTempFile;

#[test]
fn empty_graph_binary_matches_typescript() {
    let mut graph = SocialGraph::new(ADAM);
    let rust_binary = graph.to_binary().expect("rust binary");
    let ts_fixture = ts_fixture_emit("empty");
    assert_eq!(
        hex::encode(rust_binary),
        ts_fixture.binary_hex.clone().unwrap()
    );
    assert_eq!(summary(&mut graph), without_binary(ts_fixture));
}

#[test]
fn complex_graph_round_trips_with_typescript() {
    let ts_fixture = ts_fixture_emit("default");

    let mut graph = SocialGraph::new(ADAM);
    for event in scenario_events() {
        graph.handle_event(&event, true, 1.0);
    }

    let rust_binary = graph.to_binary().expect("rust binary");
    assert_eq!(
        hex::encode(&rust_binary),
        ts_fixture.binary_hex.clone().unwrap()
    );
    assert_eq!(summary(&mut graph), without_binary(ts_fixture.clone()));

    let mut rust_from_ts = SocialGraph::from_binary(
        ADAM,
        &hex::decode(ts_fixture.binary_hex.clone().unwrap()).unwrap(),
    )
    .expect("rust load ts binary");
    assert_eq!(
        summary(&mut rust_from_ts),
        without_binary(ts_fixture.clone())
    );

    let temp = NamedTempFile::new().expect("temp file");
    fs::write(temp.path(), rust_binary).expect("write rust binary");
    let mut ts_from_rust = ts_fixture_load(ADAM, temp.path());
    assert_eq!(
        summary(&mut rust_from_ts),
        without_binary(ts_from_rust.clone())
    );

    let mut alt_root = SocialGraph::from_binary(FIATJAF, &fs::read(temp.path()).unwrap())
        .expect("rust load rust binary as alternate root");
    ts_from_rust = ts_fixture_load(FIATJAF, temp.path());
    assert_eq!(summary(&mut alt_root), without_binary(ts_from_rust));
}

#[test]
fn typed_id_binary_matches_typescript_for_uuid_and_string_nodes() {
    const ROOT: &str = "local:root";
    const IDENTITY: &str = "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a";
    const EXTERNAL: &str = "external:nvpn-peer:exit-a";

    let ts_fixture = ts_fixture_emit("arbitrary-ids");
    let binary = hex::decode(ts_fixture.binary_hex.clone().unwrap()).unwrap();
    let graph = SocialGraph::from_binary(ROOT, &binary).expect("rust load ts typed-id binary");

    assert_eq!(graph.get_root(), ROOT);
    assert!(graph.is_following(ROOT, IDENTITY));
    assert!(graph.is_following(IDENTITY, EXTERNAL));
    assert_eq!(graph.get_follow_distance(IDENTITY), 1);
    assert_eq!(graph.get_follow_distance(EXTERNAL), 2);
    assert_eq!(hex::encode(graph.to_binary().unwrap()), hex::encode(binary));
}

#[test]
fn budgeted_binary_matches_typescript_for_multiple_limits() {
    let budgets = [
        BinaryBudget {
            max_nodes: Some(2),
            ..BinaryBudget::default()
        },
        BinaryBudget {
            max_edges: Some(2),
            ..BinaryBudget::default()
        },
        BinaryBudget {
            max_distance: Some(1),
            ..BinaryBudget::default()
        },
        BinaryBudget {
            max_edges_per_node: Some(1),
            ..BinaryBudget::default()
        },
        BinaryBudget {
            max_nodes: Some(3),
            max_edges: Some(2),
            max_distance: Some(1),
            max_edges_per_node: Some(1),
        },
    ];

    for budget in budgets {
        let ts_fixture = ts_fixture_emit_budgeted("default", budget);
        let mut graph = SocialGraph::new(ADAM);
        for event in scenario_events() {
            graph.handle_event(&event, true, 1.0);
        }

        let rust_binary = graph
            .to_binary_with_budget(budget)
            .expect("budgeted rust binary");
        assert_eq!(
            hex::encode(&rust_binary),
            ts_fixture.binary_hex.clone().unwrap(),
            "budget mismatch for {budget:?}"
        );

        let mut rust_from_ts = SocialGraph::from_binary(
            ADAM,
            &hex::decode(ts_fixture.binary_hex.clone().unwrap()).unwrap(),
        )
        .expect("load budgeted ts binary");
        let temp = NamedTempFile::new().expect("temp file");
        fs::write(temp.path(), &rust_binary).expect("write rust budgeted binary");
        let mut ts_from_rust = ts_fixture_load(ADAM, temp.path());

        assert_eq!(
            summary(&mut rust_from_ts),
            without_binary(ts_fixture.clone())
        );
        assert_eq!(
            summary(&mut rust_from_ts),
            without_binary(ts_from_rust.clone())
        );

        let mut alt_root = SocialGraph::from_binary(FIATJAF, &rust_binary)
            .expect("load rust budgeted binary with alternate root");
        ts_from_rust = ts_fixture_load(FIATJAF, temp.path());
        assert_eq!(summary(&mut alt_root), without_binary(ts_from_rust));
    }
}

#[test]
fn budgeted_chunk_output_reassembles_to_direct_binary() {
    let budget = BinaryBudget {
        max_nodes: Some(3),
        max_edges: Some(2),
        max_distance: Some(1),
        max_edges_per_node: Some(1),
    };
    let mut graph = SocialGraph::new(ADAM);
    for event in scenario_events() {
        graph.handle_event(&event, true, 1.0);
    }

    let direct = graph
        .to_binary_with_budget(budget)
        .expect("direct budgeted binary");
    let chunks = graph
        .to_binary_chunks_with_budget(budget)
        .expect("chunked budgeted binary");

    assert!(!chunks.is_empty());
    assert!(chunks.iter().all(|chunk| !chunk.is_empty()));

    let reassembled: Vec<u8> = chunks.into_iter().flatten().collect();
    assert_eq!(reassembled, direct);
}

#[test]
fn real_binary_fixture_matches_typescript_summary_for_default_root() {
    let path = real_binary_path();
    assert!(
        path.exists(),
        "missing real binary fixture at {}",
        path.display()
    );
    let mut graph = SocialGraph::from_binary(ADAM, &read_bytes(&path)).unwrap();
    let ts_summary = ts_fixture_load(ADAM, &path);
    assert_eq!(summary(&mut graph), without_binary(ts_summary));
}

#[test]
fn real_binary_fixture_matches_typescript_summary_for_alternate_root() {
    let path = real_binary_path();
    assert!(
        path.exists(),
        "missing real binary fixture at {}",
        path.display()
    );
    let mut graph = SocialGraph::from_binary(SIRIUS, &read_bytes(&path)).unwrap();
    let ts_summary = ts_fixture_load(SIRIUS, &path);
    assert_eq!(summary(&mut graph), without_binary(ts_summary));
}

fn real_binary_path() -> PathBuf {
    repo_root().join("ts/data/socialGraph.bin")
}
