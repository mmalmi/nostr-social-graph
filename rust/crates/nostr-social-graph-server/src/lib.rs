use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use indexmap::IndexMap;
use nostr_sdk::hashes::{Hash, sha256};
use nostr_sdk::{
    Client as NostrClient, Filter, Keys, Kind, PublicKey, RelayPoolNotification, Timestamp,
    ToBech32,
};
use nostr_social_graph::{BinaryBudget, NostrEvent, SocialGraph, SocialGraphState};
use nostr_social_graph_hashtree::HashtreeSocialGraph;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

mod forwarding;
mod relay_mirror;
use forwarding::Forwarder;

pub use relay_mirror::{
    RelayMirrorConfig, allowed_pubkeys_from_graph, event_is_allowed, render_allowlist,
    run_relay_mirror,
};

const PROFILE_NAME_MAX_LENGTH: usize = 100;
const PROFILE_PICTURE_URL_MAX_LENGTH: usize = 255;
pub const DEFAULT_SOCIAL_GRAPH_ROOT: &str =
    "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0";
pub const DEFAULT_RELAY_URLS: &[&str] = &[];

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ServerError>;

#[derive(Debug, Serialize)]
struct FuseIndexJson {
    keys: Vec<FuseIndexKey>,
    records: Vec<FuseIndexRecord>,
}

#[derive(Debug, Serialize)]
struct FuseIndexKey {
    path: [&'static str; 1],
    id: &'static str,
    weight: u8,
    src: &'static str,
}

#[derive(Debug, Serialize)]
struct FuseIndexRecord {
    i: usize,
    #[serde(rename = "$")]
    fields: BTreeMap<String, FuseIndexField>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum FuseIndexField {
    Single(FuseIndexValue),
}

#[derive(Debug, Serialize)]
struct FuseIndexValue {
    v: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    i: Option<usize>,
    n: FuseNorm,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum FuseNorm {
    Integer(u8),
    Float(f64),
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    data_path: PathBuf,
    rows_by_pubkey: IndexMap<String, Vec<String>>,
    latest_timestamps: BTreeMap<String, u64>,
}

impl ProfileStore {
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self> {
        let data_path = path.as_ref().to_path_buf();
        let rows_by_pubkey = if data_path.exists() {
            let rows = serde_json::from_slice::<Vec<Vec<String>>>(&fs::read(&data_path)?)?;
            rows.into_iter()
                .filter_map(|row| row.first().cloned().map(|pubkey| (pubkey, row)))
                .collect()
        } else {
            IndexMap::new()
        };

        Ok(Self {
            data_path,
            rows_by_pubkey,
            latest_timestamps: BTreeMap::new(),
        })
    }

    pub fn profile_count(&self) -> usize {
        self.rows_by_pubkey.len()
    }

    pub fn snapshot(&self, max_bytes: Option<usize>, no_pictures: bool) -> Vec<Vec<String>> {
        let mut rows: Vec<Vec<String>> = self.rows_by_pubkey.values().cloned().collect();

        if no_pictures {
            rows = rows
                .into_iter()
                .map(|row| {
                    let base = row.into_iter().take(3).collect::<Vec<_>>();
                    let mut end = base.len();
                    while end > 0 && base[end - 1].is_empty() {
                        end -= 1;
                    }
                    base.into_iter().take(end).collect()
                })
                .collect();
        }

        let Some(max_bytes) = max_bytes else {
            return rows;
        };

        let mut current_size = 2usize;
        let mut result = Vec::new();
        for row in rows {
            let row_size = usize::from(!result.is_empty())
                + 2
                + row.iter().map(|value| 2 + value.len()).sum::<usize>();
            if current_size + row_size > max_bytes {
                break;
            }
            current_size += row_size;
            result.push(row);
        }

        result
    }

    pub fn apply_event(&mut self, event: &NostrEvent) -> bool {
        let current_timestamp = self.latest_timestamps.get(&event.pubkey).copied();
        if current_timestamp.is_some_and(|current| event.created_at <= current) {
            return false;
        }

        let Ok(profile) = serde_json::from_str::<Value>(&event.content) else {
            return false;
        };
        let Some(profile) = profile.as_object() else {
            return false;
        };

        let canonical = canonicalize_profile(profile);
        let Some(name) = canonical.primary_name else {
            return false;
        };

        self.latest_timestamps
            .insert(event.pubkey.clone(), event.created_at);

        let mut row = vec![event.pubkey.clone(), name];
        if let Some(nip05) = canonical.nip05 {
            row.push(nip05);
        } else if profile_picture(profile).is_some() {
            row.push(String::new());
        }
        if let Some(picture) = profile_picture(profile) {
            row.push(picture);
        }

        self.rows_by_pubkey.insert(event.pubkey.clone(), row);
        true
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let rows = self.rows_by_pubkey.values().collect::<Vec<_>>();
        fs::write(&self.data_path, serde_json::to_vec(&rows)?)?;
        Ok(())
    }

    pub fn write_profile_index(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            serde_json::to_vec(&build_profile_index(self.rows_by_pubkey.values()))?,
        )?;
        Ok(())
    }
}

fn build_profile_index<'a>(rows: impl IntoIterator<Item = &'a Vec<String>>) -> FuseIndexJson {
    FuseIndexJson {
        keys: vec![
            FuseIndexKey {
                path: ["name"],
                id: "name",
                weight: 1,
                src: "name",
            },
            FuseIndexKey {
                path: ["pubKey"],
                id: "pubKey",
                weight: 1,
                src: "pubKey",
            },
            FuseIndexKey {
                path: ["nip05"],
                id: "nip05",
                weight: 1,
                src: "nip05",
            },
        ],
        records: rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| build_profile_index_record(row, index))
            .collect(),
    }
}

fn build_profile_index_record(row: &[String], doc_index: usize) -> FuseIndexRecord {
    let mut fields = BTreeMap::new();
    insert_profile_index_value(&mut fields, 0, row.get(1));
    insert_profile_index_value(&mut fields, 1, row.first());
    insert_profile_index_value(&mut fields, 2, row.get(2).filter(|value| !value.is_empty()));

    FuseIndexRecord {
        i: doc_index,
        fields,
    }
}

fn insert_profile_index_value(
    fields: &mut BTreeMap<String, FuseIndexField>,
    key_index: usize,
    value: Option<&String>,
) {
    let Some(value) = value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    fields.insert(
        key_index.to_string(),
        FuseIndexField::Single(FuseIndexValue {
            v: value.to_string(),
            i: None,
            n: fuse_norm(value),
        }),
    );
}

fn fuse_norm(value: &str) -> FuseNorm {
    let token_count = value
        .split(' ')
        .filter(|token| !token.is_empty())
        .count()
        .max(1) as f64;
    let rounded = ((1.0 / token_count.sqrt()) * 1000.0).round() / 1000.0;
    if (rounded - 1.0).abs() < f64::EPSILON {
        FuseNorm::Integer(1)
    } else {
        FuseNorm::Float(rounded)
    }
}

/// Crawl progress, distinct from event timestamps (a quiet graph can still be fresh).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphFreshness {
    pub started_at: u64,
    pub last_graph_success_at: Option<u64>,
    pub last_full_reconciliation_at: Option<u64>,
    pub last_graph_progress_at: Option<u64>,
    pub last_live_event_at: Option<u64>,
    pub graph_since: Option<u64>,
    pub consecutive_graph_failures: u64,
    pub live_connected: bool,
    pub forwarding_enabled: bool,
    pub last_forwarded_at: Option<u64>,
    pub last_forward_reconciliation_at: Option<u64>,
    pub forwarding_failures: u64,
}

impl GraphFreshness {
    pub fn healthy_at(&self, now: u64) -> bool {
        self.live_connected
            && (!self.forwarding_enabled
                || (self.forwarding_failures == 0
                    && self
                        .last_forward_reconciliation_at
                        .is_some_and(|at| now.saturating_sub(at) < 172800)))
            && self.consecutive_graph_failures == 0
            && self
                .last_graph_success_at
                .is_some_and(|at| now.saturating_sub(at) < 7200)
            && self
                .last_full_reconciliation_at
                .is_some_and(|at| now.saturating_sub(at) < 172800)
    }
}

#[derive(Clone)]
pub struct AppState {
    graph: Arc<RwLock<SocialGraph>>,
    profiles: Arc<RwLock<ProfileStore>>,
    profile_index_path: Arc<PathBuf>,
    freshness: Arc<RwLock<GraphFreshness>>,
    mobile_snapshot: Arc<Mutex<BTreeMap<String, MobileSnapshot>>>,
}

impl AppState {
    pub fn new_shared(
        graph: Arc<RwLock<SocialGraph>>,
        profiles: Arc<RwLock<ProfileStore>>,
        profile_index_path: impl AsRef<Path>,
    ) -> Self {
        Self {
            graph,
            profiles,
            profile_index_path: Arc::new(profile_index_path.as_ref().to_path_buf()),
            freshness: Arc::new(RwLock::new(GraphFreshness::default())),
            mobile_snapshot: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn for_tests(
        graph: SocialGraph,
        profiles: ProfileStore,
        profile_index_path: impl AsRef<Path>,
    ) -> Self {
        Self::new_shared(
            Arc::new(RwLock::new(graph)),
            Arc::new(RwLock::new(profiles)),
            profile_index_path,
        )
    }

    pub fn graph(&self) -> Arc<RwLock<SocialGraph>> {
        Arc::clone(&self.graph)
    }

    pub fn profiles(&self) -> Arc<RwLock<ProfileStore>> {
        Arc::clone(&self.profiles)
    }

    pub fn profile_index_path(&self) -> &Path {
        self.profile_index_path.as_ref().as_path()
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root_page))
        .route("/social-graph", get(social_graph_binary))
        .route("/social-graph/mobile", get(mobile_graph_binary))
        .route("/healthz", get(health))
        .route("/allowlist", get(allowlist))
        .route("/profile-data", get(profile_data))
        .route("/profile-index", get(profile_index))
        .with_state(state)
}

pub fn build_router_with_cors(state: AppState, allow_origin: Option<&str>) -> Router {
    let cors = match allow_origin {
        Some("*") | None => CorsLayer::new().allow_origin(Any),
        Some(origin) => match HeaderValue::from_str(origin) {
            Ok(value) => CorsLayer::new().allow_origin(value),
            Err(_) => CorsLayer::new().allow_origin(Any),
        },
    };

    build_router(state).layer(cors.expose_headers([
        axum::http::header::ETAG,
        axum::http::HeaderName::from_static("x-graph-root"),
    ]))
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub data_dir: PathBuf,
    pub graph_db_dir: PathBuf,
    pub legacy_graph_binary_path: Option<PathBuf>,
    pub profile_data_path: PathBuf,
    pub profile_index_path: PathBuf,
    pub root: String,
    pub relay_urls: Vec<String>,
    pub crawl_distance: Option<u32>,
    pub profile_distance: Option<u32>,
    pub profile_limit: Option<usize>,
    pub crawl_batch_size: usize,
    pub profile_batch_size: usize,
    pub sync_interval: Duration,
    pub allow_origin: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let data_dir = std::env::var_os("DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.join("data"));
        let graph_db_dir = std::env::var_os("SOCIAL_GRAPH_DB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("socialGraph.hashtree"));
        let legacy_graph_binary_path = std::env::var_os("LEGACY_SOCIAL_GRAPH_BINARY_PATH")
            .map(PathBuf::from)
            .or_else(|| {
                let default = data_dir.join("socialGraph.large.bin");
                default.exists().then_some(default)
            });
        let profile_data_path = data_dir.join("profileData.large.json");
        let profile_index_path = data_dir.join("profileIndex.json");

        Self {
            port: parse_env_u16("PORT", 3000),
            data_dir,
            graph_db_dir,
            legacy_graph_binary_path,
            profile_data_path,
            profile_index_path,
            root: std::env::var("SOCIAL_GRAPH_ROOT")
                .unwrap_or_else(|_| DEFAULT_SOCIAL_GRAPH_ROOT.to_string()),
            relay_urls: parse_relay_urls(std::env::var("RELAY_URLS").ok()),
            crawl_distance: parse_optional_distance(
                std::env::var("SOCIAL_GRAPH_CRAWL_DISTANCE")
                    .ok()
                    .or_else(|| std::env::var("CRAWL_DISTANCE").ok()),
                Some(4),
            ),
            profile_distance: parse_optional_distance(
                std::env::var("PROFILE_CRAWL_DISTANCE").ok(),
                None,
            ),
            profile_limit: parse_optional_usize(std::env::var("PROFILE_CRAWL_LIMIT").ok()),
            crawl_batch_size: parse_env_usize("CRAWL_BATCH_SIZE", 500),
            profile_batch_size: parse_env_usize("PROFILE_BATCH_SIZE", 100),
            sync_interval: Duration::from_millis(parse_env_u64("SYNC_INTERVAL_MS", 30_000)),
            allow_origin: std::env::var("ALLOW_ORIGIN").ok(),
        }
    }
}

pub fn load_or_bootstrap_graph(
    root: &str,
    graph_db_dir: impl AsRef<Path>,
    legacy_graph_binary_path: Option<&Path>,
) -> Result<SocialGraph> {
    let graph_db_dir = graph_db_dir.as_ref();
    let mut store = HashtreeSocialGraph::open(graph_db_dir, root)
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    let state = store.export_state();

    let graph_is_empty = state.followed_by_user.is_empty() && state.muted_by_user.is_empty();
    if graph_is_empty
        && let Some(graph_binary_path) = legacy_graph_binary_path.filter(|path| path.exists())
    {
        let graph = SocialGraph::from_binary(root, &fs::read(graph_binary_path)?)
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
        store
            .replace_state(&graph.export_state())
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
        info!(
            "imported graph snapshot from {} into hashtree store {}",
            graph_binary_path.display(),
            graph_db_dir.display()
        );
        return Ok(graph);
    }

    SocialGraph::from_state(state)
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))
}

pub fn load_graph_read_only(
    root: &str,
    graph_db_dir: impl AsRef<Path>,
    legacy_graph_binary_path: Option<&Path>,
) -> Result<SocialGraph> {
    let graph_db_dir = graph_db_dir.as_ref();
    let state = if path_has_entries(graph_db_dir) {
        let store = HashtreeSocialGraph::open(graph_db_dir, root)
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
        store.export_state()
    } else {
        empty_graph_state(root)
    };

    let graph_is_empty = state.followed_by_user.is_empty() && state.muted_by_user.is_empty();
    if graph_is_empty
        && let Some(graph_binary_path) = legacy_graph_binary_path.filter(|path| path.exists())
    {
        return SocialGraph::from_binary(root, &fs::read(graph_binary_path)?)
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())));
    }

    SocialGraph::from_state(state)
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))
}

pub fn persist_graph_snapshot(
    root: &str,
    graph_db_dir: impl AsRef<Path>,
    graph: &SocialGraph,
) -> Result<()> {
    let graph_db_dir = graph_db_dir.as_ref();
    let mut store = HashtreeSocialGraph::open(graph_db_dir, root)
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    store
        .replace_state(&graph.export_state())
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    info!("persisted graph snapshot to {}", graph_db_dir.display());
    Ok(())
}

fn empty_graph_state(root: &str) -> SocialGraphState {
    SocialGraphState {
        root: root.to_string(),
        unique_ids: Vec::new(),
        follow_distance_by_user: Vec::new(),
        users_by_follow_distance: Vec::new(),
        followed_by_user: Vec::new(),
        followers_by_user: Vec::new(),
        follow_list_created_at: Vec::new(),
        muted_by_user: Vec::new(),
        user_muted_by: Vec::new(),
        mute_list_created_at: Vec::new(),
        list_revisions: Vec::new(),
    }
}

pub async fn run(config: ServerConfig) -> Result<()> {
    if config.relay_urls.is_empty() {
        return Err(ServerError::Io(std::io::Error::other(
            "RELAY_URLS must contain at least one Nostr relay for graph syncing",
        )));
    }
    fs::create_dir_all(&config.data_dir)?;
    // Filesystem modification time is not a successful crawl watermark. Always
    // repair all known lists on startup, also hydrating legacy revision IDs.
    let initial_graph_since = None;
    let initial_profile_since = modified_unix_timestamp(&config.profile_data_path)
        .map(|timestamp| timestamp.saturating_sub(1));

    let graph = load_or_bootstrap_graph(
        &config.root,
        &config.graph_db_dir,
        config.legacy_graph_binary_path.as_deref(),
    )?;
    let profiles = ProfileStore::load_or_default(&config.profile_data_path)?;
    profiles.write_profile_index(&config.profile_index_path)?;

    info!(
        "loaded graph with {} users and {} profiles",
        graph.size().users,
        profiles.profile_count()
    );

    forwarding::refresh_allowlist(&graph)?;
    let graph = Arc::new(RwLock::new(graph));
    let profiles = Arc::new(RwLock::new(profiles));
    let state = AppState::new_shared(
        Arc::clone(&graph),
        Arc::clone(&profiles),
        &config.profile_index_path,
    );

    state
        .freshness
        .write()
        .expect("freshness lock poisoned")
        .started_at = unix_timestamp();
    let freshness = Arc::clone(&state.freshness);
    let client = connect_client(&config).await?;
    subscribe_live_notifications(&client).await?;
    freshness
        .write()
        .expect("freshness lock poisoned")
        .live_connected = true;

    let forwarder = Forwarder::from_env(&config.data_dir, Arc::clone(&freshness)).await?;
    let sync_forwarder = forwarder.clone();
    let sync_graph = Arc::clone(&graph);
    let sync_profiles = Arc::clone(&profiles);
    let sync_config = config.clone();
    let sync_client = client.clone();
    let sync_freshness = Arc::clone(&freshness);
    tokio::spawn(async move {
        if let Err(error) = sync_loop(
            sync_client,
            sync_config,
            sync_graph,
            sync_profiles,
            initial_graph_since,
            initial_profile_since,
            Arc::clone(&sync_freshness),
            sync_forwarder,
        )
        .await
        {
            sync_freshness
                .write()
                .expect("freshness lock poisoned")
                .consecutive_graph_failures += 1;
            error!("background sync failed: {error}");
        }
    });

    let live_graph = Arc::clone(&graph);
    let live_profiles = Arc::clone(&profiles);
    let live_config = config.clone();
    let live_client = client.clone();
    tokio::spawn(async move {
        if let Err(error) = notification_loop(
            live_client,
            live_config,
            live_graph,
            live_profiles,
            Arc::clone(&freshness),
            forwarder,
        )
        .await
        {
            error!("notification loop failed: {error}");
        }
        freshness
            .write()
            .expect("freshness lock poisoned")
            .live_connected = false;
    });

    let app = build_router_with_cors(state, config.allow_origin.as_deref());
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, config.port)).await?;
    info!("server listening on http://0.0.0.0:{}", config.port);
    axum::serve(listener, app).await.map_err(ServerError::from)
}

#[derive(Debug, Deserialize)]
struct SocialGraphQuery {
    #[serde(rename = "maxNodes")]
    max_nodes: Option<usize>,
    #[serde(rename = "maxEdges")]
    max_edges: Option<usize>,
    #[serde(rename = "maxDistance")]
    max_distance: Option<u32>,
    #[serde(rename = "maxEdgesPerNode")]
    max_edges_per_node: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProfileDataQuery {
    #[serde(rename = "maxBytes")]
    max_bytes: Option<usize>,
    #[serde(rename = "noPictures")]
    no_pictures: Option<bool>,
}

async fn root_page(State(state): State<AppState>) -> Html<String> {
    let graph = state.graph.read().expect("graph lock poisoned");
    let stats = graph.size();
    let root_npub = graph
        .get_root()
        .parse::<PublicKey>()
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| graph.get_root().to_string());
    let profile_count = state
        .profiles
        .read()
        .expect("profile lock poisoned")
        .profile_count();

    let rows = stats
        .size_by_distance
        .iter()
        .map(|(distance, count)| format!("<tr><td>{distance}</td><td>{count}</td></tr>"))
        .collect::<Vec<_>>()
        .join("");

    Html(format!(
        "<!DOCTYPE html>\
         <html>\
           <head>\
             <title>Nostr Social Graph Stats</title>\
             <style>\
               body {{ font-family: sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; }}\
               .stats {{ background: #f5f5f5; padding: 20px; border-radius: 8px; }}\
               .stats h2 {{ margin-top: 0; }}\
               .stats p {{ margin: 10px 0; }}\
               .distance-stats {{ margin-top: 20px; }}\
               .distance-stats h3 {{ margin-bottom: 10px; }}\
               .distance-stats table {{ width: 100%; border-collapse: collapse; }}\
               .distance-stats th, .distance-stats td {{ padding: 8px; text-align: left; border-bottom: 1px solid #ddd; }}\
               .distance-stats th {{ background: #eee; }}\
               .stats a {{ color: #0066cc; text-decoration: none; }}\
               .stats a:hover {{ text-decoration: underline; }}\
               .downloads {{ margin-top: 20px; background: #f5f5f5; padding: 20px; border-radius: 8px; }}\
               .downloads h3 {{ margin-top: 0; }}\
               .downloads ul {{ list-style: none; padding: 0; margin: 0; }}\
               .downloads li {{ margin: 10px 0; }}\
               .downloads a {{ color: #0066cc; text-decoration: none; }}\
               .downloads a:hover {{ text-decoration: underline; }}\
               .profile-stats {{ margin-top: 20px; background: #f5f5f5; padding: 20px; border-radius: 8px; }}\
               .profile-stats h3 {{ margin-top: 0; }}\
               .profile-stats p {{ margin: 10px 0; }}\
             </style>\
           </head>\
           <body>\
             <div class=\"stats\">\
               <h2>Social Graph Statistics</h2>\
               <p>Graph root: <a href=\"https://iris.to/{root_npub}\" target=\"_blank\">{root_npub}</a></p>\
               <p>Total users: {users}</p>\
               <p>Total follows: {follows}</p>\
               <p>Total mutes: {mutes}</p>\
               <div class=\"distance-stats\">\
                 <h3>Users by Follow Distance</h3>\
                 <table>\
                   <thead>\
                     <tr>\
                       <th>Distance</th>\
                       <th>Users</th>\
                     </tr>\
                   </thead>\
                   <tbody>{rows}</tbody>\
                 </table>\
               </div>\
             </div>\
             <div class=\"profile-stats\">\
               <h3>Profile Data Statistics</h3>\
               <p>Total indexed profiles: {profile_count}</p>\
             </div>\
             <div class=\"downloads\">\
               <h3>Download Data</h3>\
               <ul>\
                 <li><a href=\"/social-graph\">Download Social Graph (Binary)</a></li>\
                 <li><a href=\"/social-graph?maxNodes=10000&maxEdges=50000\">Download Social Graph (Binary, Limited)</a></li>\
                 <li><a href=\"/social-graph?maxDistance=2\">Download Social Graph (Binary, Distance ≤ 2)</a></li>\
                 <li><a href=\"/social-graph?maxDistance=2&maxEdges=20000\">Download Social Graph (Binary, Distance ≤ 2, Limited Edges)</a></li>\
                 <li><a href=\"/social-graph?maxEdgesPerNode=100\">Download Social Graph (Binary, ≤100 edges per user)</a></li>\
                 <li><a href=\"/social-graph?maxDistance=2&maxEdgesPerNode=50\">Download Social Graph (Binary, Distance ≤ 2, ≤50 edges per user)</a></li>\
                 <li><a href=\"/profile-data\">Download Profile Data</a></li>\
                 <li><a href=\"/profile-index\">Download Profile Index</a></li>\
               </ul>\
               <p><small>\
                 You can customize the download size using query parameters:<br/>\
                 <code>?maxNodes=N</code> - Limit to N unique users<br/>\
                 <code>?maxEdges=N</code> - Limit to N follow/mute relationships<br/>\
                 <code>?maxDistance=N</code> - Include only users within N follow hops from root<br/>\
                 <code>?maxEdgesPerNode=N</code> - Limit each user to N follow/mute relationships<br/>\
                 Parameters can be combined: <code>?maxDistance=2&maxEdgesPerNode=100</code>\
               </small></p>\
             </div>\
           </body>\
         </html>",
        root_npub = root_npub,
        users = stats.users,
        follows = stats.follows,
        mutes = stats.mutes,
        rows = rows,
        profile_count = profile_count
    ))
}

async fn health(State(state): State<AppState>) -> Response {
    let freshness = state
        .freshness
        .read()
        .expect("freshness lock poisoned")
        .clone();
    let healthy = freshness.healthy_at(unix_timestamp());
    let graph = state.graph.read().expect("graph lock poisoned");
    let stats = graph.size();
    let mut response = (
        if healthy {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(
            serde_json::json!({"healthy": healthy, "freshness": freshness,
            "users": stats.users, "follows": stats.follows, "mutes": stats.mutes}),
        ),
    )
        .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

struct MobileSnapshot {
    generated_at: u64,
    bytes: axum::body::Bytes,
    etag: HeaderValue,
}

/// Mobile bootstrap is intentionally a projection, never a replacement for a
/// client's own graph. Binary revision completeness flags survive projection.
#[derive(Default, Deserialize)]
struct MobileGraphQuery {
    root: Option<String>,
}

async fn mobile_graph_binary(
    State(state): State<AppState>,
    Query(query): Query<MobileGraphQuery>,
    headers: HeaderMap,
) -> Response {
    let now = unix_timestamp();
    let default_root = state
        .graph
        .read()
        .expect("graph lock")
        .get_root()
        .to_string();
    let root = query.root.unwrap_or_else(|| default_root.clone());
    // Only maintained public discovery roots: arbitrary roots would permit
    // unbounded traversal/cache work controlled by unauthenticated requests.
    if root != default_root
        && root != "336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b"
    {
        return (StatusCode::BAD_REQUEST, "Unsupported discovery root").into_response();
    }

    let mut cached = state
        .mobile_snapshot
        .lock()
        .expect("snapshot lock poisoned");
    if cached
        .get(&root)
        .is_none_or(|snapshot| now.saturating_sub(snapshot.generated_at) >= 60)
    {
        let graph = state.graph.read().expect("graph lock poisoned");
        let budget = BinaryBudget {
            max_nodes: Some(30_000),
            max_edges: Some(180_000),
            max_distance: Some(4),
            max_edges_per_node: Some(500),
        };
        match graph.to_binary_for_root_with_budget(&root, budget) {
            Ok(binary) if binary.len() <= 4 * 1024 * 1024 => {
                let etag =
                    HeaderValue::from_str(&format!("\"{}:{}\"", root, sha256::Hash::hash(&binary)))
                        .expect("hash header");
                cached.insert(
                    root.clone(),
                    MobileSnapshot {
                        generated_at: now,
                        bytes: binary.into(),
                        etag,
                    },
                );
            }
            _ => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Mobile graph exceeds its byte budget",
                )
                    .into_response();
            }
        }
    }
    let snapshot = cached.get(&root).expect("snapshot generated");
    let mut response = if headers.get(axum::http::header::IF_NONE_MATCH) == Some(&snapshot.etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        snapshot.bytes.clone().into_response()
    };
    response
        .headers_mut()
        .insert(axum::http::header::ETAG, snapshot.etag.clone());
    response.headers_mut().insert(
        "x-graph-root",
        HeaderValue::from_str(&root).expect("root header"),
    );
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, stale-while-revalidate=60"),
    );
    response
}

async fn social_graph_binary(
    State(state): State<AppState>,
    Query(query): Query<SocialGraphQuery>,
) -> Response {
    let graph = state.graph.read().expect("graph lock poisoned");
    let budget = BinaryBudget {
        max_nodes: query.max_nodes,
        max_edges: query.max_edges,
        max_distance: query.max_distance,
        max_edges_per_node: query.max_edges_per_node,
    };

    match graph.to_binary_with_budget_version(budget, 3) {
        Ok(binary) => {
            let mut response = Response::new(binary.into_response().into_body());
            let headers = response.headers_mut();
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            headers.insert(
                CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"social-graph.bin\""),
            );
            headers.insert(
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=60, stale-while-revalidate=60"),
            );
            response
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to build social graph binary: {error}"),
        )
            .into_response(),
    }
}

async fn allowlist(
    State(state): State<AppState>,
    Query(query): Query<SocialGraphQuery>,
) -> Response {
    let graph = state.graph.read().expect("graph lock poisoned");
    let mut body = String::new();
    for pubkey in graph.users_in_distance_order(query.max_distance) {
        body.push_str(&pubkey);
        body.push('\n');
    }

    let mut response = body.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, stale-while-revalidate=60"),
    );
    response
}

async fn profile_data(
    State(state): State<AppState>,
    Query(query): Query<ProfileDataQuery>,
) -> Response {
    let rows = state
        .profiles
        .read()
        .expect("profile lock poisoned")
        .snapshot(query.max_bytes, query.no_pictures.unwrap_or(false));

    let mut response = Json(rows).into_response();
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, stale-while-revalidate=60"),
    );
    response
}

async fn profile_index(State(state): State<AppState>) -> Response {
    match tokio::fs::read(state.profile_index_path()).await {
        Ok(bytes) => {
            let mut response = Response::new(bytes.into_response().into_body());
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            );
            response.headers_mut().insert(
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=60, stale-while-revalidate=60"),
            );
            response
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "profile index not found").into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read profile index: {error}"),
        )
            .into_response(),
    }
}

#[derive(Debug, Clone)]
struct CanonicalProfile {
    primary_name: Option<String>,
    nip05: Option<String>,
}

fn canonicalize_profile(profile: &serde_json::Map<String, Value>) -> CanonicalProfile {
    let names = extract_profile_names(profile);
    let primary_name = names.first().cloned();
    let nip05 = normalize_nip05(profile.get("nip05"), primary_name.as_deref());

    CanonicalProfile {
        primary_name,
        nip05,
    }
}

fn extract_profile_names(profile: &serde_json::Map<String, Value>) -> Vec<String> {
    let candidates = [
        profile.get("display_name"),
        profile.get("displayName"),
        profile.get("name"),
        profile.get("username"),
    ];
    let mut names = Vec::new();
    let mut seen = BTreeMap::<String, ()>::new();

    for candidate in candidates.into_iter().flatten() {
        let Some(normalized) = normalize_name_value(candidate) else {
            continue;
        };
        let key = normalized.to_lowercase();
        if seen.contains_key(&key) {
            continue;
        }
        seen.insert(key, ());
        names.push(normalized);
    }

    names
}

fn normalize_name_value(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    let trimmed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(PROFILE_NAME_MAX_LENGTH).collect())
}

fn normalize_nip05(value: Option<&Value>, primary_name: Option<&str>) -> Option<String> {
    let value = value?.as_str()?;
    let local_part = value
        .split('@')
        .next()?
        .trim()
        .to_lowercase()
        .chars()
        .take(PROFILE_NAME_MAX_LENGTH)
        .collect::<String>();
    if local_part.is_empty() || local_part.len() == 1 || local_part.starts_with("npub1") {
        return None;
    }
    if primary_name.is_some_and(|name| {
        name.to_lowercase()
            .split_whitespace()
            .collect::<String>()
            .contains(&local_part)
    }) {
        return None;
    }
    Some(local_part)
}

fn profile_picture(profile: &serde_json::Map<String, Value>) -> Option<String> {
    let picture = profile.get("picture")?.as_str()?.trim();
    if picture.is_empty() || picture.len() >= PROFILE_PICTURE_URL_MAX_LENGTH {
        return None;
    }
    Some(picture.trim_start_matches("https://").to_string())
}

async fn sync_loop(
    client: NostrClient,
    config: ServerConfig,
    graph: Arc<RwLock<SocialGraph>>,
    profiles: Arc<RwLock<ProfileStore>>,
    initial_graph_since: Option<u64>,
    initial_profile_since: Option<u64>,
    freshness: Arc<RwLock<GraphFreshness>>,
    forwarder: Option<Forwarder>,
) -> Result<()> {
    let mut graph_since = initial_graph_since;
    let mut profile_since = initial_profile_since;
    let mut last_full_reconciliation_at = 0;

    loop {
        let cycle_started_at = unix_timestamp();

        let full_reconciliation =
            cycle_started_at.saturating_sub(last_full_reconciliation_at) >= 86400;
        let requested_since = if full_reconciliation {
            None
        } else {
            graph_since
        };
        if full_reconciliation {
            freshness
                .write()
                .expect("freshness lock")
                .forwarding_failures = 0;
        }
        let graph_success = match sync_graph_once(
            &client,
            &config,
            &graph,
            requested_since,
            &freshness,
            forwarder.as_ref(),
        )
        .await
        {
            Ok(()) => true,
            Err(error) => {
                freshness
                    .write()
                    .expect("freshness lock poisoned")
                    .consecutive_graph_failures += 1;
                warn!("graph sync failed: {error}");
                false
            }
        };
        if let Some(forwarder) = &forwarder {
            forwarder.drain().await;
        }
        {
            let mut graph_guard = graph.write().expect("graph lock poisoned");
            let removed = graph_guard.remove_muted_not_followed_users();
            if removed > 0 {
                info!("removed {removed} muted users without followers");
            }
        }

        {
            let graph_guard = graph.read().expect("graph lock poisoned");
            persist_graph_snapshot(&config.root, &config.graph_db_dir, &graph_guard)?;
            forwarding::refresh_allowlist(&graph_guard)?;
        }
        if graph_success {
            graph_since = Some(cycle_started_at.saturating_sub(300));
            let now = unix_timestamp();
            if full_reconciliation {
                last_full_reconciliation_at = now;
            }
            let mut status = freshness.write().expect("freshness lock poisoned");
            status.last_graph_success_at = Some(now);
            status.last_full_reconciliation_at = Some(last_full_reconciliation_at);
            status.graph_since = graph_since;
            status.consecutive_graph_failures = 0;
            if full_reconciliation && status.forwarding_failures == 0 && status.forwarding_enabled {
                status.last_forward_reconciliation_at = Some(now);
            }
            let status_path = config.data_dir.join("graph-freshness.json");
            let temporary = config.data_dir.join("graph-freshness.json.tmp");
            fs::write(&temporary, serde_json::to_vec(&*status)?)?;
            fs::rename(temporary, status_path)?;
        }
        // Profiles cannot delay persisting a successfully reconciled graph.
        match sync_profiles_once(&client, &config, &graph, &profiles, profile_since).await {
            Ok(()) => profile_since = Some(cycle_started_at.saturating_sub(300)),
            Err(error) => warn!("profile sync failed: {error}"),
        }
        {
            let profiles_guard = profiles.read().expect("profile lock poisoned");
            profiles_guard.save()?;
            profiles_guard.write_profile_index(&config.profile_index_path)?;
        }
        info!(
            "sync complete: users={} profiles={}",
            graph.read().expect("graph lock poisoned").size().users,
            profiles
                .read()
                .expect("profile lock poisoned")
                .profile_count()
        );

        tokio::time::sleep(config.sync_interval).await;
    }
}

async fn connect_client(config: &ServerConfig) -> Result<NostrClient> {
    let client = NostrClient::new(Keys::generate());
    for relay in &config.relay_urls {
        client
            .add_relay(relay)
            .await
            .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    }
    client.connect().await;
    client.wait_for_connection(Duration::from_secs(5)).await;
    Ok(client)
}

async fn subscribe_live_notifications(client: &NostrClient) -> Result<()> {
    let since = Timestamp::from(unix_timestamp());
    client
        .subscribe(
            Filter::new()
                .kinds(vec![Kind::from(3_u16), Kind::from(10_000_u16)])
                .since(since),
            None,
        )
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    client
        .subscribe(Filter::new().kind(Kind::from(0_u16)).since(since), None)
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    Ok(())
}

async fn notification_loop(
    client: NostrClient,
    config: ServerConfig,
    graph: Arc<RwLock<SocialGraph>>,
    profiles: Arc<RwLock<ProfileStore>>,
    freshness: Arc<RwLock<GraphFreshness>>,
    forwarder: Option<Forwarder>,
) -> Result<()> {
    let mut notifications = client.notifications();

    loop {
        match notifications.recv().await {
            Ok(RelayPoolNotification::Event { event, .. }) => {
                if matches!(event.kind.as_u16(), 3 | 10000) {
                    freshness
                        .write()
                        .expect("freshness lock poisoned")
                        .last_live_event_at = Some(unix_timestamp());
                }
                handle_live_event(&config, &graph, &profiles, &event);
                if let Some(forwarder) = &forwarder {
                    let winner = graph
                        .read()
                        .expect("graph lock")
                        .get_list_revision(&event.pubkey.to_hex(), event.kind.as_u16() as u32)
                        .is_some_and(|revision| revision.id == event.id.to_hex());
                    if winner {
                        forwarder.try_enqueue(&event);
                    }
                }
            }
            Ok(RelayPoolNotification::Shutdown) => {
                warn!("relay notification channel shutdown");
                return Ok(());
            }
            Ok(RelayPoolNotification::Message { .. }) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!("notification loop lagged and skipped {skipped} events");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                warn!("notification loop closed");
                return Ok(());
            }
        }
    }
}

async fn sync_graph_once(
    client: &NostrClient,
    config: &ServerConfig,
    graph: &Arc<RwLock<SocialGraph>>,
    since: Option<u64>,
    freshness: &Arc<RwLock<GraphFreshness>>,
    forwarder: Option<&Forwarder>,
) -> Result<()> {
    let mut current_distance = 0u32;

    loop {
        if config
            .crawl_distance
            .is_some_and(|max_distance| current_distance > max_distance)
        {
            break;
        }

        forwarding::refresh_allowlist(&graph.read().expect("graph lock"))?;
        let authors = graph
            .read()
            .expect("graph lock poisoned")
            .get_users_by_follow_distance(current_distance);

        if authors.is_empty() {
            let max_known_distance = graph
                .read()
                .expect("graph lock poisoned")
                .size()
                .size_by_distance
                .keys()
                .copied()
                .max()
                .unwrap_or(0);
            if current_distance >= max_known_distance {
                break;
            }
            current_distance = current_distance.saturating_add(1);
            continue;
        }

        for author_chunk in authors.chunks(config.crawl_batch_size.clamp(1, 128)) {
            freshness
                .write()
                .expect("freshness lock poisoned")
                .last_graph_progress_at = Some(unix_timestamp());
            let (new_authors, known_authors) = {
                let guard = graph.read().expect("graph lock poisoned");
                author_chunk
                    .iter()
                    .cloned()
                    .partition::<Vec<_>, _>(|author| {
                        guard.get_follow_list_created_at(author).is_none()
                    })
            };

            if !new_authors.is_empty() {
                fetch_graph_events(client, &new_authors, None, graph, forwarder).await?;
            }
            if !known_authors.is_empty() && since.is_some() {
                fetch_graph_events(client, &known_authors, since, graph, forwarder).await?;
            } else if !known_authors.is_empty() && since.is_none() {
                fetch_graph_events(client, &known_authors, None, graph, forwarder).await?;
            }
        }

        current_distance = current_distance.saturating_add(1);
    }

    Ok(())
}

// The SDK convenience fetch returns Ok(empty) even if every relay is offline.
// A crawl watermark requires an actual EOSE, not merely a completed timeout.
async fn fetch_graph_batch(client: &NostrClient, filter: Filter) -> Result<Vec<nostr_sdk::Event>> {
    use nostr_sdk::{RelayMessage, SubscriptionId};
    let id = SubscriptionId::generate();
    let mut notifications = client.notifications();
    let subscribed = client
        .subscribe_with_id(id.clone(), filter.clone(), None)
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;
    if subscribed.success.is_empty() {
        client.unsubscribe(&id).await;
        return Err(ServerError::Io(std::io::Error::other(
            "No graph relay accepted the crawl subscription",
        )));
    }
    let mut pending = subscribed.success;
    let mut completed = false;
    let mut winners = BTreeMap::<(String, u16), nostr_sdk::Event>::new();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match notifications.recv().await {
                Ok(RelayPoolNotification::Message { relay_url, message }) => match message {
                    RelayMessage::Event {
                        subscription_id,
                        event,
                    } if subscription_id.as_ref() == &id => {
                        if filter.match_event(&event, Default::default())
                            && event.verify().is_ok()
                        {
                            let key = (event.pubkey.to_hex(), event.kind.as_u16());
                            let replace = winners.get(&key).is_none_or(|old| {
                                event.created_at > old.created_at
                                    || (event.created_at == old.created_at && event.id < old.id)
                            });
                            if replace {
                                winners.insert(key, event.into_owned());
                            }
                        }
                    }
                    RelayMessage::EndOfStoredEvents(subscription_id)
                        if subscription_id.as_ref() == &id =>
                    {
                        completed = true;
                        pending.remove(&relay_url);
                        if pending.is_empty() {
                            return Ok(());
                        }
                    }
                    _ => {}
                },
                Ok(RelayPoolNotification::Shutdown) => return Err("Graph relay pool shut down"),
                Err(_) => return Err("Graph crawl notification stream lost messages"),
                _ => {}
            }
        }
    })
    .await;
    client.unsubscribe(&id).await;
    if let Ok(Err(error)) = result {
        return Err(ServerError::Io(std::io::Error::other(error)));
    }
    if !completed {
        return Err(ServerError::Io(std::io::Error::other(
            "Graph crawl timed out without EOSE",
        )));
    }
    Ok(winners.into_values().collect())
}

async fn fetch_graph_events(
    client: &NostrClient,
    authors: &[String],
    since: Option<u64>,
    graph: &Arc<RwLock<SocialGraph>>,
    forwarder: Option<&Forwarder>,
) -> Result<()> {
    let pubkeys = parse_public_keys(authors);
    if pubkeys.is_empty() {
        return Ok(());
    }

    let filter = graph_filter(pubkeys, since);
    let events = fetch_graph_batch(client, filter).await?;

    let mut forward = Vec::new();
    let mut applied = 0usize;
    {
        let mut guard = graph.write().expect("graph lock poisoned");
        for event in events.into_iter() {
            if let Some(parsed) = sdk_event_to_social_graph_event(&event) {
                guard.handle_event(&parsed, true, 1.0);
                applied += 1;
                if guard
                    .get_list_revision(&parsed.pubkey, parsed.kind)
                    .is_some_and(|revision| revision.id == parsed.id)
                {
                    forward.push(event);
                }
            }
        }
    }
    if let Some(forwarder) = forwarder {
        for event in forward {
            forwarder.enqueue(&event).await;
        }
    }
    if applied > 0 {
        info!(
            "applied {applied} graph events for {} authors",
            authors.len()
        );
    }
    Ok(())
}

fn handle_live_event(
    config: &ServerConfig,
    graph: &Arc<RwLock<SocialGraph>>,
    profiles: &Arc<RwLock<ProfileStore>>,
    event: &nostr_sdk::Event,
) {
    let Some(event) = sdk_event_to_social_graph_event(event) else {
        return;
    };

    match event.kind {
        3 | 10_000 => {
            graph
                .write()
                .expect("graph lock poisoned")
                .handle_event(&event, false, 1.0);
        }
        0 => {
            let distance = graph
                .read()
                .expect("graph lock poisoned")
                .get_follow_distance(&event.pubkey);
            if distance < 1000
                && config
                    .profile_distance
                    .is_none_or(|max_distance| distance <= max_distance)
            {
                profiles
                    .write()
                    .expect("profile lock poisoned")
                    .apply_event(&event);
            }
        }
        _ => {}
    }
}

async fn sync_profiles_once(
    client: &NostrClient,
    config: &ServerConfig,
    graph: &Arc<RwLock<SocialGraph>>,
    profiles: &Arc<RwLock<ProfileStore>>,
    since: Option<u64>,
) -> Result<()> {
    let mut authors = graph
        .read()
        .expect("graph lock poisoned")
        .users_in_distance_order(config.profile_distance);
    if let Some(limit) = config.profile_limit {
        authors.truncate(limit);
    }

    for author_chunk in authors.chunks(config.profile_batch_size.max(1)) {
        let (new_authors, known_authors) = {
            let guard = profiles.read().expect("profile lock poisoned");
            author_chunk
                .iter()
                .cloned()
                .partition::<Vec<_>, _>(|author| !guard.rows_by_pubkey.contains_key(author))
        };

        if !new_authors.is_empty() {
            fetch_profile_events(client, &new_authors, None, profiles).await?;
        }
        if !known_authors.is_empty() && since.is_some() {
            fetch_profile_events(client, &known_authors, since, profiles).await?;
        } else if !known_authors.is_empty() && since.is_none() {
            fetch_profile_events(client, &known_authors, None, profiles).await?;
        }
    }

    Ok(())
}

async fn fetch_profile_events(
    client: &NostrClient,
    authors: &[String],
    since: Option<u64>,
    profiles: &Arc<RwLock<ProfileStore>>,
) -> Result<()> {
    let pubkeys = parse_public_keys(authors);
    if pubkeys.is_empty() {
        return Ok(());
    }

    let filter = profile_filter(pubkeys, since);
    let events = client
        .fetch_events(filter, Duration::from_secs(10))
        .await
        .map_err(|error| ServerError::Io(std::io::Error::other(error.to_string())))?;

    let mut guard = profiles.write().expect("profile lock poisoned");
    let mut applied = 0usize;
    for event in events.into_iter() {
        if let Some(event) = sdk_event_to_social_graph_event(&event)
            && guard.apply_event(&event)
        {
            applied += 1;
        }
    }
    if applied > 0 {
        info!(
            "applied {applied} profile events for {} authors",
            authors.len()
        );
    }
    Ok(())
}

fn parse_public_keys(authors: &[String]) -> Vec<PublicKey> {
    authors
        .iter()
        .filter_map(|author| author.parse::<PublicKey>().ok())
        .collect()
}

fn graph_filter(pubkeys: Vec<PublicKey>, since: Option<u64>) -> Filter {
    let filter = Filter::new()
        .authors(pubkeys)
        .kinds(vec![Kind::from(3_u16), Kind::from(10_000_u16)]);
    if let Some(since) = since {
        filter.since(Timestamp::from(since))
    } else {
        filter
    }
}

fn profile_filter(pubkeys: Vec<PublicKey>, since: Option<u64>) -> Filter {
    let filter = Filter::new().authors(pubkeys).kind(Kind::from(0_u16));
    if let Some(since) = since {
        filter.since(Timestamp::from(since))
    } else {
        filter
    }
}

fn sdk_event_to_social_graph_event<T: serde::Serialize>(event: &T) -> Option<NostrEvent> {
    let value = serde_json::to_value(event).ok()?;
    let tags = value
        .get("tags")?
        .as_array()?
        .iter()
        .map(|tag| {
            tag.as_array()?
                .iter()
                .map(|part| part.as_str().map(ToOwned::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .collect::<Option<Vec<_>>>()?;

    Some(NostrEvent {
        created_at: value.get("created_at")?.as_u64()?,
        content: value.get("content")?.as_str()?.to_string(),
        tags,
        kind: value.get("kind")?.as_u64()? as u32,
        pubkey: value.get("pubkey")?.as_str()?.to_string(),
        id: value.get("id")?.as_str()?.to_string(),
        sig: value.get("sig")?.as_str()?.to_string(),
    })
}

fn parse_relay_urls(raw: Option<String>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
    .filter(|urls| !urls.is_empty())
    .unwrap_or_else(|| {
        DEFAULT_RELAY_URLS
            .iter()
            .map(|url| (*url).to_string())
            .collect()
    })
}

fn parse_env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default)
}

fn parse_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn parse_optional_usize(raw: Option<String>) -> Option<usize> {
    raw.and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn parse_optional_distance(raw: Option<String>, default: Option<u32>) -> Option<u32> {
    match raw.as_deref().map(str::trim) {
        Some("") => default,
        Some("all") => None,
        Some(value) => value.parse::<u32>().ok().or(default),
        None => default,
    }
}

fn path_has_entries(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}

fn modified_unix_timestamp(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
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

    #[tokio::test]
    async fn server_rejects_missing_crawl_relays_before_opening_data() {
        let mut config = ServerConfig::from_env();
        config.relay_urls.clear();
        let error = run(config).await.unwrap_err();
        assert!(error.to_string().contains("RELAY_URLS must contain"));
    }

    #[test]
    fn relay_urls_are_empty_until_explicitly_configured() {
        assert!(DEFAULT_RELAY_URLS.is_empty());
        assert!(parse_relay_urls(None).is_empty());
        assert!(parse_relay_urls(Some(" , ".to_string())).is_empty());
    }

    #[test]
    fn relay_urls_parse_explicit_configuration() {
        assert_eq!(
            parse_relay_urls(Some(
                "wss://relay-a.example, wss://relay-b.example ".to_string()
            )),
            vec![
                "wss://relay-a.example".to_string(),
                "wss://relay-b.example".to_string()
            ]
        );
    }
}

#[cfg(test)]
mod graph_crawl_tests {
    use super::*;
    #[tokio::test]
    async fn disconnected_relays_cannot_mark_an_empty_crawl_successful() {
        let client = NostrClient::new(Keys::generate());
        let filter = graph_filter(
            vec![PublicKey::from_hex(DEFAULT_SOCIAL_GRAPH_ROOT).unwrap()],
            None,
        );
        assert!(fetch_graph_batch(&client, filter).await.is_err());
    }
}
