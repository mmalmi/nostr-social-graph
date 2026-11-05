use std::fs;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nostr_social_graph::SocialGraph;
use nostr_social_graph_server::{AppState, ProfileStore, build_router};
use tempfile::TempDir;
use tower::ServiceExt;

const ADAM: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";
const FIATJAF: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const SNOWDEN: &str = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";
const SIRIUS: &str = "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0";

#[tokio::test]
async fn root_page_renders_graph_and_profile_stats() {
    let tempdir = TempDir::new().unwrap();
    let profile_data_path = tempdir.path().join("profileData.large.json");
    let profile_index_path = tempdir.path().join("profileIndex.json");
    fs::write(
        &profile_data_path,
        serde_json::to_vec(&vec![
            vec![ADAM.to_string(), "Adam".to_string()],
            vec![FIATJAF.to_string(), "fiatjaf".to_string()],
        ])
        .unwrap(),
    )
    .unwrap();
    fs::write(&profile_index_path, br#"{"version":1}"#).unwrap();

    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF, SNOWDEN]), true, 1.0);
    graph.handle_event(&event(FIATJAF, 3, 1_100, vec![SIRIUS]), true, 1.0);

    let profiles = ProfileStore::load_or_default(&profile_data_path).unwrap();
    let app = build_router(AppState::for_tests(graph, profiles, profile_index_path));

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("Total users: 4"));
    assert!(html.contains("Total follows: 3"));
    assert!(html.contains("Total indexed profiles: 2"));
}

#[tokio::test]
async fn social_graph_endpoint_honors_budget_queries() {
    let tempdir = TempDir::new().unwrap();
    let profile_data_path = tempdir.path().join("profileData.large.json");
    let profile_index_path = tempdir.path().join("profileIndex.json");
    fs::write(&profile_data_path, b"[]").unwrap();
    fs::write(&profile_index_path, br#"{"version":1}"#).unwrap();

    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF, SNOWDEN]), true, 1.0);
    graph.handle_event(&event(FIATJAF, 3, 1_100, vec![SIRIUS]), true, 1.0);

    let profiles = ProfileStore::load_or_default(&profile_data_path).unwrap();
    let app = build_router(AppState::for_tests(graph, profiles, profile_index_path));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/social-graph?maxDistance=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap(),
        "application/octet-stream"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let graph = SocialGraph::from_binary(ADAM, &body).unwrap();
    assert_eq!(graph.get_follow_distance(ADAM), 0);
    assert_eq!(graph.get_follow_distance(FIATJAF), 1);
    assert_eq!(graph.get_follow_distance(SNOWDEN), 1);
    assert_eq!(graph.get_follow_distance(SIRIUS), 2);
}

#[tokio::test]
async fn allowlist_endpoint_honors_max_distance() {
    let tempdir = TempDir::new().unwrap();
    let profile_data_path = tempdir.path().join("profileData.large.json");
    let profile_index_path = tempdir.path().join("profileIndex.json");
    fs::write(&profile_data_path, b"[]").unwrap();
    fs::write(&profile_index_path, br#"{"version":1}"#).unwrap();

    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF, SNOWDEN]), true, 1.0);
    graph.handle_event(&event(FIATJAF, 3, 1_100, vec![SIRIUS]), true, 1.0);

    let profiles = ProfileStore::load_or_default(&profile_data_path).unwrap();
    let app = build_router(AppState::for_tests(graph, profiles, profile_index_path));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/allowlist?maxDistance=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap(),
        "text/plain; charset=utf-8"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let allowlist = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(allowlist, format!("{ADAM}\n{FIATJAF}\n{SNOWDEN}\n"));
}

#[tokio::test]
async fn profile_data_endpoint_applies_no_pictures_and_byte_limits() {
    let tempdir = TempDir::new().unwrap();
    let profile_data_path = tempdir.path().join("profileData.large.json");
    let profile_index_path = tempdir.path().join("profileIndex.json");
    fs::write(
        &profile_data_path,
        serde_json::to_vec(&vec![
            vec![
                ADAM.to_string(),
                "Adam".to_string(),
                "adam".to_string(),
                "img.one".to_string(),
            ],
            vec![FIATJAF.to_string(), "fiatjaf".to_string()],
        ])
        .unwrap(),
    )
    .unwrap();
    fs::write(&profile_index_path, br#"{"version":1}"#).unwrap();

    let profiles = ProfileStore::load_or_default(&profile_data_path).unwrap();
    let app = build_router(AppState::for_tests(
        SocialGraph::new(ADAM),
        profiles,
        profile_index_path,
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/profile-data?noPictures=true&maxBytes=90")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let rows: Vec<Vec<String>> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        rows,
        vec![vec![
            ADAM.to_string(),
            "Adam".to_string(),
            "adam".to_string()
        ]]
    );
}

#[tokio::test]
async fn profile_index_endpoint_serves_existing_file() {
    let tempdir = TempDir::new().unwrap();
    let profile_data_path = tempdir.path().join("profileData.large.json");
    let profile_index_path = tempdir.path().join("profileIndex.json");
    fs::write(&profile_data_path, b"[]").unwrap();
    fs::write(&profile_index_path, br#"{"version":7}"#).unwrap();

    let profiles = ProfileStore::load_or_default(&profile_data_path).unwrap();
    let app = build_router(AppState::for_tests(
        SocialGraph::new(ADAM),
        profiles,
        profile_index_path,
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/profile-index")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap(),
        "application/json; charset=utf-8"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), br#"{"version":7}"#);
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

#[test]
fn freshness_requires_successful_recent_repair_and_live_loop() {
    use nostr_social_graph_server::GraphFreshness;
    let mut status = GraphFreshness::default();
    assert!(!status.healthy_at(200_000));
    status.live_connected = true;
    status.last_graph_success_at = Some(199_999);
    status.last_full_reconciliation_at = Some(199_999);
    assert!(status.healthy_at(200_000));
    status.consecutive_graph_failures = 1;
    assert!(!status.healthy_at(200_000));
    status.consecutive_graph_failures = 0;
    assert!(!status.healthy_at(210_000));
    status.last_graph_success_at = Some(399_999);
    assert!(!status.healthy_at(400_000));
}

#[tokio::test]
async fn health_does_not_claim_an_uncrawled_graph_is_current() {
    let tempdir = TempDir::new().unwrap();
    let profiles = ProfileStore::load_or_default(tempdir.path().join("profiles.json")).unwrap();
    let app = build_router(AppState::for_tests(
        SocialGraph::new(ADAM),
        profiles,
        tempdir.path().join("index.json"),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["healthy"], false);
    assert!(status["freshness"]["last_graph_success_at"].is_null());
}

#[tokio::test]
async fn mobile_snapshot_is_bounded_conditional_and_preserves_partial_revision() {
    let tempdir = TempDir::new().unwrap();
    let mut graph = SocialGraph::new(ADAM);
    let targets: Vec<String> = (1..=600).map(|i| format!("{i:064x}")).collect();
    let mut follows = event(ADAM, 3, 1_000, targets.iter().map(String::as_str).collect());
    follows.id = "aa".repeat(32);
    graph.handle_event(&follows, true, 1.0);
    let mut mutes = event(FIATJAF, 10000, 1_001, vec![SNOWDEN]);
    mutes.id = "bb".repeat(32);
    graph.handle_event(&mutes, true, 1.0);
    let profiles = ProfileStore::load_or_default(tempdir.path().join("profiles.json")).unwrap();
    let app = build_router(AppState::for_tests(
        graph,
        profiles,
        tempdir.path().join("index.json"),
    ));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/social-graph/mobile")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let etag = response.headers().get("etag").unwrap().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() < 4 * 1024 * 1024);
    let loaded = SocialGraph::from_binary(ADAM, &bytes).unwrap();
    assert_eq!(loaded.size().follows, 500);
    let revision = loaded.get_list_revision(ADAM, 3).unwrap();
    assert_eq!(revision.id, follows.id);
    assert!(!revision.complete);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/social-graph/mobile")
                .header("if-none-match", etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}
