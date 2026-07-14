#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nostr_social_graph::{BinaryBudget, NostrEvent, SocialGraph};
use serde::{Deserialize, Serialize};

pub const ADAM: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";
pub const FIATJAF: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
pub const SNOWDEN: &str = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";
pub const SIRIUS: &str = "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0";
pub const BOB: &str = "4132aeeee5c7b3497d260c922758e804a9cf9c0933d3e333bfd15f7695db3852";
pub const CHARLIE: &str = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
pub const DIANA: &str = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSummary {
    pub root: String,
    pub binary_hex: Option<String>,
    pub distances: Vec<(String, u32)>,
    pub follows: Vec<(String, Vec<String>)>,
    pub followers: Vec<(String, Vec<String>)>,
    pub mutes: Vec<(String, Vec<String>)>,
    pub muters: Vec<(String, Vec<String>)>,
    pub follow_list_created_at: Vec<(String, Option<u64>)>,
    pub mute_list_created_at: Vec<(String, Option<u64>)>,
}

pub fn summary(graph: &mut SocialGraph) -> GraphSummary {
    GraphSummary {
        root: graph.get_root().to_string(),
        binary_hex: None,
        distances: keyed_users()
            .into_iter()
            .map(|user| (user.to_string(), graph.get_follow_distance(user)))
            .collect(),
        follows: owners()
            .into_iter()
            .map(|user| (user.to_string(), sorted(graph.get_followed_by_user(user))))
            .collect(),
        followers: keyed_users()
            .into_iter()
            .map(|user| (user.to_string(), sorted(graph.get_followers_by_user(user))))
            .collect(),
        mutes: owners()
            .into_iter()
            .map(|user| (user.to_string(), sorted(graph.get_muted_by_user(user))))
            .collect(),
        muters: keyed_users()
            .into_iter()
            .map(|user| (user.to_string(), sorted(graph.get_user_muted_by(user))))
            .collect(),
        follow_list_created_at: owners()
            .into_iter()
            .map(|user| (user.to_string(), graph.get_follow_list_created_at(user)))
            .collect(),
        mute_list_created_at: owners()
            .into_iter()
            .map(|user| (user.to_string(), graph.get_mute_list_created_at(user)))
            .collect(),
    }
}

pub fn ts_fixture_emit(name: &str) -> GraphSummary {
    run_fixture(["emit", name])
}

pub fn ts_fixture_emit_budgeted(name: &str, budget: BinaryBudget) -> GraphSummary {
    let max_nodes = budget_arg(budget.max_nodes);
    let max_edges = budget_arg(budget.max_edges);
    let max_distance = budget_arg(budget.max_distance.map(|value| value as usize));
    let max_edges_per_node = budget_arg(budget.max_edges_per_node);
    run_fixture_owned(vec![
        "emit-budget".to_string(),
        name.to_string(),
        max_nodes,
        max_edges,
        max_distance,
        max_edges_per_node,
    ])
}

pub fn ts_fixture_load(root: &str, path: &Path) -> GraphSummary {
    run_fixture(["load", root, path.to_str().unwrap()])
}

pub fn without_binary(mut summary: GraphSummary) -> GraphSummary {
    summary.binary_hex = None;
    summary
}

pub fn scenario_events() -> Vec<NostrEvent> {
    vec![
        event(ADAM, 3, 1_000, vec![BOB, FIATJAF]),
        event(FIATJAF, 3, 1_100, vec![SNOWDEN]),
        event(BOB, 10_000, 1_200, vec![SNOWDEN]),
        event(ADAM, 10_000, 1_300, vec![CHARLIE]),
        event(ADAM, 10_000, 900, vec![SNOWDEN]),
        event(FIATJAF, 3, 1_400, vec![SIRIUS]),
    ]
}

pub fn event(pubkey: &str, kind: u32, created_at: u64, tagged: Vec<&str>) -> NostrEvent {
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

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf()
}

fn run_fixture<const N: usize>(args: [&str; N]) -> GraphSummary {
    let owned_args: Vec<String> = args.into_iter().map(ToOwned::to_owned).collect();
    run_fixture_owned(owned_args)
}

fn run_fixture_owned(args: Vec<String>) -> GraphSummary {
    let repo_root = repo_root();
    let output = Command::new("pnpm")
        .arg("--filter")
        .arg("nostr-social-graph")
        .arg("exec")
        .arg("tsx")
        .arg("scripts/rustInteropFixture.ts")
        .args(&args)
        .current_dir(&repo_root)
        .output()
        .expect("run ts fixture");
    assert!(
        output.status.success(),
        "fixture failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("fixture json")
}

fn budget_arg(value: Option<usize>) -> String {
    value.map(|number| number.to_string()).unwrap_or_default()
}

fn owners() -> Vec<&'static str> {
    vec![ADAM, BOB, FIATJAF]
}

fn keyed_users() -> Vec<&'static str> {
    vec![ADAM, BOB, FIATJAF, SNOWDEN, SIRIUS, CHARLIE]
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

pub fn read_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).expect("read file")
}
