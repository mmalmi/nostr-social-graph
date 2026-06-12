use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nostr_lmdb::NostrLMDB;
use nostr_sdk::JsonUtil;
use nostr_sdk::prelude::{
    Client, ClientBuilder, Event, Filter, Kind, PublicKey, RelayPoolNotification, SyncOptions,
    Timestamp,
};
use nostr_social_graph::SocialGraph;
use tracing::{error, info, warn};

use crate::{
    DEFAULT_RELAY_URLS, DEFAULT_SOCIAL_GRAPH_ROOT, Result, ServerError, load_graph_read_only,
};

pub const ALLOWED_EVENT_KINDS: [u16; 3] = [0, 3, 10_000];
const LOCAL_RELAY_MAX_EVENT_SIZE: usize = 524_288;
const LOCAL_RELAY_MAX_NUM_TAGS: usize = 25_000;
const LOCAL_RELAY_MAX_TAG_VALUE_SIZE: usize = 4_096;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AllowedAuthors {
    sorted_hex: BTreeSet<String>,
    parsed: HashSet<PublicKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalRelayRejectReason {
    ProtectedEvent,
    TooManyTags { count: usize },
    TagValueTooLarge { kind: String, size: usize },
    InvalidFixedSizeTag { kind: String, value: String },
    EventTooLarge { size: usize },
    SerializationFailed,
}

impl fmt::Display for LocalRelayRejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtectedEvent => write!(f, "event marked as protected"),
            Self::TooManyTags { count } => {
                write!(f, "too many tags: {count} > {LOCAL_RELAY_MAX_NUM_TAGS}")
            }
            Self::TagValueTooLarge { kind, size } => write!(
                f,
                "tag value too large for {kind}: {size} > {LOCAL_RELAY_MAX_TAG_VALUE_SIZE}"
            ),
            Self::InvalidFixedSizeTag { kind, value } => write!(
                f,
                "invalid fixed-size tag {kind}: expected 64 hex chars, got {}",
                value.len()
            ),
            Self::EventTooLarge { size } => {
                write!(f, "event too large: {size} > {LOCAL_RELAY_MAX_EVENT_SIZE}")
            }
            Self::SerializationFailed => write!(f, "failed to serialize event"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RelayMirrorConfig {
    pub root: String,
    pub sync_relay_urls: Vec<String>,
    pub live_relay_urls: Vec<String>,
    pub allowlist_url: Option<String>,
    pub graph_db_dir: PathBuf,
    pub legacy_graph_binary_path: Option<PathBuf>,
    pub graph_snapshot_url: Option<String>,
    pub state_dir: PathBuf,
    pub allowlist_path: PathBuf,
    pub local_relay_url: String,
    pub allowed_distance: Option<u32>,
    pub authors_per_filter: usize,
    pub sync_interval: Duration,
    pub negentropy_initial_timeout: Duration,
    pub live_subscription_lookback: Duration,
}

impl RelayMirrorConfig {
    pub fn from_env() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let data_dir = std::env::var_os("DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.join("data"));
        let graph_db_dir = std::env::var_os("SOCIAL_GRAPH_DB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("socialGraph.heed"));
        let legacy_graph_binary_path = std::env::var_os("LEGACY_SOCIAL_GRAPH_BINARY_PATH")
            .map(PathBuf::from)
            .or_else(|| {
                let default = data_dir.join("socialGraph.large.bin");
                default.exists().then_some(default)
            });
        let graph_snapshot_url = std::env::var("GRAPH_RELAY_GRAPH_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let allowlist_url = std::env::var("GRAPH_RELAY_ALLOWLIST_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let state_dir = std::env::var_os("GRAPH_RELAY_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("graph-relay"));
        let allowlist_path = std::env::var_os("GRAPH_RELAY_ALLOWLIST_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| state_dir.join("allowlist.txt"));
        let sync_relay_urls = resolve_sync_relay_urls(
            std::env::var("GRAPH_RELAY_SYNC_RELAY_URLS")
                .ok()
                .or_else(|| std::env::var("RELAY_URLS").ok()),
        );
        let live_relay_urls = resolve_live_relay_urls(
            std::env::var("GRAPH_RELAY_LIVE_RELAY_URLS").ok(),
            &sync_relay_urls,
        );

        Self {
            root: std::env::var("SOCIAL_GRAPH_ROOT")
                .unwrap_or_else(|_| DEFAULT_SOCIAL_GRAPH_ROOT.to_string()),
            sync_relay_urls,
            live_relay_urls,
            allowlist_url,
            graph_db_dir,
            legacy_graph_binary_path,
            graph_snapshot_url,
            state_dir,
            allowlist_path,
            local_relay_url: std::env::var("GRAPH_RELAY_LOCAL_URL")
                .unwrap_or_else(|_| "ws://127.0.0.1:7777".to_string()),
            allowed_distance: parse_optional_distance(
                std::env::var("GRAPH_RELAY_DISTANCE")
                    .ok()
                    .or_else(|| std::env::var("SOCIAL_GRAPH_CRAWL_DISTANCE").ok())
                    .or_else(|| std::env::var("CRAWL_DISTANCE").ok()),
                Some(4),
            ),
            authors_per_filter: parse_env_usize("GRAPH_RELAY_AUTHORS_PER_FILTER", 256),
            sync_interval: Duration::from_millis(parse_env_u64(
                "GRAPH_RELAY_SYNC_INTERVAL_MS",
                300_000,
            )),
            negentropy_initial_timeout: Duration::from_millis(parse_env_u64(
                "GRAPH_RELAY_NEGENTROPY_TIMEOUT_MS",
                10_000,
            )),
            live_subscription_lookback: Duration::from_secs(parse_env_u64(
                "GRAPH_RELAY_LIVE_LOOKBACK_SECONDS",
                900,
            )),
        }
    }
}

pub fn allowed_pubkeys_from_graph(
    graph: &SocialGraph,
    max_distance: Option<u32>,
) -> BTreeSet<String> {
    graph
        .users_in_distance_order(max_distance)
        .into_iter()
        .collect()
}

pub fn render_allowlist(pubkeys: &BTreeSet<String>) -> String {
    let mut contents = String::new();
    for pubkey in pubkeys {
        contents.push_str(pubkey);
        contents.push('\n');
    }
    contents
}

pub fn event_is_allowed(event: &Event, allowed_pubkeys: &HashSet<PublicKey>) -> bool {
    ALLOWED_EVENT_KINDS.contains(&event.kind.as_u16()) && allowed_pubkeys.contains(&event.pubkey)
}

pub async fn run_relay_mirror(config: RelayMirrorConfig) -> Result<()> {
    fs::create_dir_all(&config.state_dir)?;
    if let Some(parent) = config.allowlist_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let upstream_db = NostrLMDB::open(config.state_dir.join("nostr-lmdb"))
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    let upstream = ClientBuilder::default().database(upstream_db).build();
    let relay_urls = config
        .sync_relay_urls
        .iter()
        .chain(config.live_relay_urls.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    for relay in relay_urls {
        upstream
            .add_read_relay(relay)
            .await
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    }
    upstream.connect().await;
    upstream.wait_for_connection(Duration::from_secs(5)).await;
    info!(
        "graph relay sync relays: {}",
        config.sync_relay_urls.join(", ")
    );
    info!(
        "graph relay live relays: {}",
        config.live_relay_urls.join(", ")
    );

    let local = Client::default();
    local
        .add_write_relay(&config.local_relay_url)
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    local.connect().await;
    local.wait_for_connection(Duration::from_secs(5)).await;

    let allowed = Arc::new(RwLock::new(AllowedAuthors::default()));
    refresh_allowed_authors(&config, &allowed).await?;
    subscribe_live_notifications(&upstream, config.live_subscription_lookback).await?;
    publish_snapshot(&upstream, &local, &config, &allowed).await?;

    let live_upstream = upstream.clone();
    let live_local = local.clone();
    let live_allowed = Arc::clone(&allowed);
    tokio::spawn(async move {
        if let Err(error) = notification_loop(live_upstream, live_local, live_allowed).await {
            error!("graph relay live notification loop failed: {error}");
        }
    });

    loop {
        refresh_allowed_authors(&config, &allowed).await?;
        sync_once(&upstream, &local, &config, &allowed).await?;
        tokio::time::sleep(config.sync_interval).await;
    }
}

async fn refresh_allowed_authors(
    config: &RelayMirrorConfig,
    allowed: &Arc<RwLock<AllowedAuthors>>,
) -> Result<()> {
    let sorted_hex = match load_allowed_pubkeys(config).await {
        Ok(sorted_hex) => sorted_hex,
        Err(error) => {
            if let Some(previous) = load_existing_allowlist(config, allowed)? {
                warn!(
                    "failed to refresh graph relay allowlist; keeping previous {} authors: {error}",
                    previous.len()
                );
                previous
            } else {
                return Err(error);
            }
        }
    };
    let parsed = sorted_hex
        .iter()
        .filter_map(|pubkey| match pubkey.parse::<PublicKey>() {
            Ok(pubkey) => Some(pubkey),
            Err(error) => {
                warn!("skipping invalid graph pubkey {pubkey}: {error}");
                None
            }
        })
        .collect::<HashSet<_>>();
    let next = AllowedAuthors { sorted_hex, parsed };

    let mut guard = allowed.write().expect("allowed authors lock poisoned");
    if *guard == next && config.allowlist_path.exists() {
        return Ok(());
    }

    write_allowlist(&config.allowlist_path, &next.sorted_hex)?;
    info!(
        "updated graph relay allowlist: {} authors -> {}",
        next.sorted_hex.len(),
        config.allowlist_path.display()
    );
    *guard = next;
    Ok(())
}

async fn load_allowed_pubkeys(config: &RelayMirrorConfig) -> Result<BTreeSet<String>> {
    if let Some(url) = config.allowlist_url.as_deref() {
        match load_allowlist_from_url(url).await {
            Ok(pubkeys) => return Ok(pubkeys),
            Err(error) => warn!("failed to load graph relay allowlist from {url}: {error}"),
        }
    }

    let graph = load_graph_snapshot(config).await?;
    Ok(allowed_pubkeys_from_graph(&graph, config.allowed_distance))
}

fn load_existing_allowlist(
    config: &RelayMirrorConfig,
    allowed: &Arc<RwLock<AllowedAuthors>>,
) -> Result<Option<BTreeSet<String>>> {
    let in_memory = {
        let guard = allowed.read().expect("allowed authors lock poisoned");
        (!guard.sorted_hex.is_empty()).then(|| guard.sorted_hex.clone())
    };
    if in_memory.is_some() {
        return Ok(in_memory);
    }

    if !config.allowlist_path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&config.allowlist_path)?;
    let parsed = parse_allowlist_text(&contents);
    if parsed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

async fn load_graph_snapshot(config: &RelayMirrorConfig) -> Result<SocialGraph> {
    if let Some(url) = config.graph_snapshot_url.as_deref() {
        match load_graph_from_url(&config.root, url).await {
            Ok(graph) => return Ok(graph),
            Err(error) => {
                warn!("failed to load graph relay snapshot from {url}: {error}");
            }
        }
    }

    load_graph_read_only(
        &config.root,
        &config.graph_db_dir,
        config.legacy_graph_binary_path.as_deref(),
    )
}

async fn load_graph_from_url(root: &str, url: &str) -> Result<SocialGraph> {
    let response = reqwest::get(url)
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    let response = response
        .error_for_status()
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    let binary = response
        .bytes()
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    SocialGraph::from_binary(root, &binary)
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))
}

async fn load_allowlist_from_url(url: &str) -> Result<BTreeSet<String>> {
    let response = reqwest::get(url)
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    let response = response
        .error_for_status()
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    let payload = response
        .bytes()
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;

    if let Ok(pubkeys) = serde_json::from_slice::<Vec<String>>(&payload) {
        return Ok(pubkeys
            .into_iter()
            .map(|pubkey| pubkey.trim().to_string())
            .filter(|pubkey| !pubkey.is_empty())
            .collect());
    }

    let text = std::str::from_utf8(&payload)
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    Ok(parse_allowlist_text(text))
}

fn parse_allowlist_text(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

async fn subscribe_live_notifications(client: &Client, lookback: Duration) -> Result<()> {
    let kinds = allowed_kinds();
    let since = unix_timestamp().saturating_sub(lookback.as_secs());
    client
        .subscribe(
            Filter::new().kinds(kinds).since(Timestamp::from(since)),
            None,
        )
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    Ok(())
}

async fn notification_loop(
    upstream: Client,
    local: Client,
    allowed: Arc<RwLock<AllowedAuthors>>,
) -> Result<()> {
    let mut notifications = upstream.notifications();

    loop {
        match notifications.recv().await {
            Ok(RelayPoolNotification::Event { event, .. }) => {
                if is_allowed_event(&event, &allowed) {
                    publish_event(&local, &event).await?;
                }
            }
            Ok(RelayPoolNotification::Shutdown) => return Ok(()),
            Ok(RelayPoolNotification::Message { .. }) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!("graph relay notification loop skipped {skipped} events");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

async fn sync_once(
    upstream: &Client,
    local: &Client,
    config: &RelayMirrorConfig,
    allowed: &Arc<RwLock<AllowedAuthors>>,
) -> Result<()> {
    let authors = {
        let guard = allowed.read().expect("allowed authors lock poisoned");
        guard.parsed.iter().copied().collect::<Vec<_>>()
    };
    if authors.is_empty() {
        warn!("graph relay allowlist is empty, skipping sync");
        return Ok(());
    }

    let kinds = allowed_kinds();
    let mut synced = 0usize;
    let mut forwarded = 0usize;
    let authors_per_filter = config.authors_per_filter.max(1);
    let total_chunks = authors.len().div_ceil(authors_per_filter);
    for (index, chunk) in authors.chunks(authors_per_filter).enumerate() {
        let filter = Filter::new().authors(chunk.to_vec()).kinds(kinds.clone());
        let output = upstream
            .sync_with(
                config.sync_relay_urls.iter().map(String::as_str),
                filter,
                &SyncOptions::new().initial_timeout(config.negentropy_initial_timeout),
            )
            .await
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;

        for (relay, failure) in &output.failed {
            warn!("graph relay sync failed for {relay}: {failure}");
        }

        synced += output.received.len();
        forwarded += publish_events_by_ids(upstream, local, &output.received, allowed).await?;
        let processed_chunks = index + 1;
        if processed_chunks == 1 || processed_chunks == total_chunks || processed_chunks % 25 == 0 {
            info!(
                "graph relay sync progress: chunks={processed_chunks}/{total_chunks} synced={synced} forwarded={forwarded}"
            );
        }
    }

    info!("graph relay sync complete: synced={synced} forwarded={forwarded}");
    Ok(())
}

async fn publish_snapshot(
    upstream: &Client,
    local: &Client,
    config: &RelayMirrorConfig,
    allowed: &Arc<RwLock<AllowedAuthors>>,
) -> Result<()> {
    let authors = {
        let guard = allowed.read().expect("allowed authors lock poisoned");
        guard.parsed.iter().copied().collect::<Vec<_>>()
    };
    if authors.is_empty() {
        return Ok(());
    }

    let kinds = allowed_kinds();
    let mut forwarded = 0usize;
    for chunk in authors.chunks(config.authors_per_filter.max(1)) {
        let events = upstream
            .database()
            .query(Filter::new().authors(chunk.to_vec()).kinds(kinds.clone()))
            .await
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
        for event in events.into_iter() {
            if is_allowed_event(&event, allowed) && publish_event(local, &event).await? {
                forwarded += 1;
            }
        }
    }

    info!("graph relay startup republished {forwarded} stored events");
    Ok(())
}

async fn publish_events_by_ids(
    upstream: &Client,
    local: &Client,
    ids: &HashSet<nostr_sdk::prelude::EventId>,
    allowed: &Arc<RwLock<AllowedAuthors>>,
) -> Result<usize> {
    let mut forwarded = 0usize;
    for id in ids {
        let Some(event) = upstream
            .database()
            .event_by_id(id)
            .await
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?
        else {
            continue;
        };
        if is_allowed_event(&event, allowed) && publish_event(local, &event).await? {
            forwarded += 1;
        }
    }
    Ok(forwarded)
}

async fn publish_event(local: &Client, event: &Event) -> Result<bool> {
    if let Some(reason) = local_relay_reject_reason(event) {
        warn!(
            "graph relay skipped local publish for {} kind {} from {}: {reason}",
            event.id, event.kind, event.pubkey
        );
        return Ok(false);
    }

    let output = local
        .send_event(event)
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    let accepted = output.failed.is_empty();
    for (relay, failure) in output.failed {
        warn!("graph relay publish failed for {relay}: {failure}");
    }
    Ok(accepted)
}

fn is_allowed_event(event: &Event, allowed: &Arc<RwLock<AllowedAuthors>>) -> bool {
    let guard = allowed.read().expect("allowed authors lock poisoned");
    event_is_allowed(event, &guard.parsed)
}

fn write_allowlist(path: &Path, pubkeys: &BTreeSet<String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, render_allowlist(pubkeys))?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn allowed_kinds() -> Vec<Kind> {
    ALLOWED_EVENT_KINDS
        .iter()
        .map(|kind| Kind::from(*kind))
        .collect()
}

fn local_relay_reject_reason(event: &Event) -> Option<LocalRelayRejectReason> {
    if event.is_protected() {
        return Some(LocalRelayRejectReason::ProtectedEvent);
    }

    let tag_count = event.tags.len();
    if tag_count > LOCAL_RELAY_MAX_NUM_TAGS {
        return Some(LocalRelayRejectReason::TooManyTags { count: tag_count });
    }

    for tag in event.tags.iter() {
        let values = tag.as_slice();
        let kind = values.first().cloned().unwrap_or_default();

        if values
            .iter()
            .any(|value| value.len() > LOCAL_RELAY_MAX_TAG_VALUE_SIZE)
        {
            let size = values.iter().map(String::len).max().unwrap_or_default();
            return Some(LocalRelayRejectReason::TagValueTooLarge { kind, size });
        }

        if matches!(kind.as_str(), "p" | "e") {
            let Some(value) = values.get(1) else {
                return Some(LocalRelayRejectReason::InvalidFixedSizeTag {
                    kind,
                    value: String::new(),
                });
            };
            if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Some(LocalRelayRejectReason::InvalidFixedSizeTag {
                    kind,
                    value: value.clone(),
                });
            }
        }
    }

    let size = match event.try_as_json() {
        Ok(json) => json.len(),
        Err(error) => {
            warn!(
                "failed to serialize graph relay event {}: {error}",
                event.id
            );
            return Some(LocalRelayRejectReason::SerializationFailed);
        }
    };
    if size > LOCAL_RELAY_MAX_EVENT_SIZE {
        return Some(LocalRelayRejectReason::EventTooLarge { size });
    }

    None
}

fn parse_relay_urls(raw: Option<String>) -> Option<Vec<String>> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
    .filter(|urls| !urls.is_empty())
}

fn resolve_sync_relay_urls(raw: Option<String>) -> Vec<String> {
    parse_relay_urls(raw).unwrap_or_else(|| {
        DEFAULT_RELAY_URLS
            .iter()
            .map(|url| (*url).to_string())
            .collect()
    })
}

fn resolve_live_relay_urls(raw: Option<String>, sync_relay_urls: &[String]) -> Vec<String> {
    parse_relay_urls(raw).unwrap_or_else(|| sync_relay_urls.to_vec())
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn parse_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn parse_optional_distance(raw: Option<String>, default: Option<u32>) -> Option<u32> {
    match raw {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() || value.eq_ignore_ascii_case("none") {
                None
            } else {
                value.parse::<u32>().ok().or(default)
            }
        }
        None => default,
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Response, StatusCode};
    use axum::routing::get;
    use nostr_sdk::JsonUtil;
    use nostr_social_graph::NostrEvent;
    use serde_json::json;
    use tempfile::TempDir;

    const ADAM: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";
    const FIATJAF: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    const BOB: &str = "4132aeeee5c7b3497d260c922758e804a9cf9c0933d3e333bfd15f7695db3852";

    #[tokio::test]
    async fn refresh_allowed_authors_can_load_graph_from_http_snapshot() {
        let graph = scenario_graph();
        let payload = graph.to_binary().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/social-graph",
            get(move || {
                let payload = payload.clone();
                async move {
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from(payload))
                        .unwrap()
                }
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let tempdir = TempDir::new().unwrap();
        let allowlist_path = tempdir.path().join("allowlist.txt");
        let allowed = Arc::new(RwLock::new(AllowedAuthors::default()));
        let config = RelayMirrorConfig {
            root: ADAM.to_string(),
            sync_relay_urls: Vec::new(),
            live_relay_urls: Vec::new(),
            allowlist_url: None,
            graph_db_dir: tempdir.path().join("missing.heed"),
            legacy_graph_binary_path: None,
            graph_snapshot_url: Some(format!("http://{address}/social-graph")),
            state_dir: tempdir.path().join("state"),
            allowlist_path: allowlist_path.clone(),
            local_relay_url: "ws://127.0.0.1:7777".to_string(),
            allowed_distance: Some(1),
            authors_per_filter: 32,
            sync_interval: Duration::from_secs(60),
            negentropy_initial_timeout: Duration::from_secs(10),
            live_subscription_lookback: Duration::from_secs(900),
        };

        refresh_allowed_authors(&config, &allowed).await.unwrap();

        let rendered = fs::read_to_string(allowlist_path).unwrap();
        assert_eq!(rendered, format!("{ADAM}\n{FIATJAF}\n{BOB}\n"));
        assert_eq!(allowed.read().unwrap().parsed.len(), 3);

        server.abort();
    }

    #[test]
    fn relay_url_resolution_defaults_sync_relays_to_project_defaults() {
        assert_eq!(
            resolve_sync_relay_urls(None),
            DEFAULT_RELAY_URLS
                .iter()
                .map(|url| (*url).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn refresh_allowed_authors_can_load_allowlist_from_http_endpoint() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/allowlist",
            get(|| async move { format!("{ADAM}\n{FIATJAF}\n{BOB}\n") }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let tempdir = TempDir::new().unwrap();
        let allowlist_path = tempdir.path().join("allowlist.txt");
        let allowed = Arc::new(RwLock::new(AllowedAuthors::default()));
        let config = RelayMirrorConfig {
            root: ADAM.to_string(),
            sync_relay_urls: Vec::new(),
            live_relay_urls: Vec::new(),
            allowlist_url: Some(format!("http://{address}/allowlist")),
            graph_db_dir: tempdir.path().join("missing.heed"),
            legacy_graph_binary_path: None,
            graph_snapshot_url: None,
            state_dir: tempdir.path().join("state"),
            allowlist_path: allowlist_path.clone(),
            local_relay_url: "ws://127.0.0.1:7777".to_string(),
            allowed_distance: Some(1),
            authors_per_filter: 32,
            sync_interval: Duration::from_secs(60),
            negentropy_initial_timeout: Duration::from_secs(10),
            live_subscription_lookback: Duration::from_secs(900),
        };

        refresh_allowed_authors(&config, &allowed).await.unwrap();

        let rendered = fs::read_to_string(allowlist_path).unwrap();
        assert_eq!(rendered, format!("{ADAM}\n{FIATJAF}\n{BOB}\n"));
        assert_eq!(allowed.read().unwrap().parsed.len(), 3);

        server.abort();
    }

    #[test]
    fn parse_allowlist_text_ignores_blank_lines() {
        assert_eq!(
            parse_allowlist_text(&format!("\n{ADAM}\n\n{FIATJAF}\n")),
            BTreeSet::from([ADAM.to_string(), FIATJAF.to_string()])
        );
    }

    #[test]
    fn relay_url_resolution_defaults_live_relays_to_sync_relays() {
        let sync_relay_urls = resolve_sync_relay_urls(Some(
            "wss://relay-a.example,wss://relay-b.example".to_string(),
        ));

        assert_eq!(
            resolve_live_relay_urls(None, &sync_relay_urls),
            sync_relay_urls
        );
    }

    #[test]
    fn relay_url_resolution_prefers_explicit_live_relays() {
        let sync_relay_urls = resolve_sync_relay_urls(Some("wss://sync-only.example".to_string()));

        assert_eq!(
            resolve_live_relay_urls(
                Some("wss://live-a.example, wss://live-b.example".to_string()),
                &sync_relay_urls,
            ),
            vec![
                "wss://live-a.example".to_string(),
                "wss://live-b.example".to_string(),
            ]
        );
    }

    #[test]
    fn local_relay_reject_reason_rejects_protected_events() {
        let event = relay_event(vec![vec!["-"]], String::new());

        assert_eq!(
            local_relay_reject_reason(&event),
            Some(LocalRelayRejectReason::ProtectedEvent)
        );
    }

    #[test]
    fn local_relay_reject_reason_rejects_invalid_fixed_size_tags() {
        let event = relay_event(vec![vec!["p", "short"]], String::new());

        assert!(matches!(
            local_relay_reject_reason(&event),
            Some(LocalRelayRejectReason::InvalidFixedSizeTag { kind, .. }) if kind == "p"
        ));
    }

    #[test]
    fn local_relay_reject_reason_rejects_oversized_events() {
        let event = relay_event(
            Vec::<Vec<&str>>::new(),
            "x".repeat(LOCAL_RELAY_MAX_EVENT_SIZE),
        );

        assert!(matches!(
            local_relay_reject_reason(&event),
            Some(LocalRelayRejectReason::EventTooLarge { size })
                if size > LOCAL_RELAY_MAX_EVENT_SIZE
        ));
    }

    #[test]
    fn local_relay_reject_reason_accepts_normal_follow_lists() {
        let event = relay_event(vec![vec!["p", BOB]], String::new());

        assert_eq!(local_relay_reject_reason(&event), None);
    }

    fn scenario_graph() -> SocialGraph {
        let mut graph = SocialGraph::new(ADAM);
        for event in [
            event(ADAM, 3, 1_000, vec![BOB, FIATJAF]),
            event(FIATJAF, 3, 1_100, vec![ADAM]),
        ] {
            graph.handle_event(&event, true, 1.0);
        }
        graph
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

    fn relay_event<S>(tags: Vec<Vec<S>>, content: String) -> Event
    where
        S: AsRef<str>,
    {
        Event::from_json(
            json!({
                "id": "0".repeat(64),
                "pubkey": ADAM,
                "created_at": 1_000,
                "kind": 3,
                "tags": tags
                    .into_iter()
                    .map(|tag| {
                        tag.into_iter()
                            .map(|value| value.as_ref().to_string())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
                "content": content,
                "sig": "0".repeat(128),
            })
            .to_string(),
        )
        .unwrap()
    }
}
