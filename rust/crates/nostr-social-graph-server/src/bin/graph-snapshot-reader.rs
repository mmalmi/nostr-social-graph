//! Serve a verified local generation installed by a separate snapshot subscriber.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use nostr_sdk::hashes::{Hash, sha256};
use nostr_social_graph::SocialGraph;
use nostr_social_graph_server::{
    AppState, DEFAULT_SOCIAL_GRAPH_ROOT, ProfileStore, SNAPSHOT_FILES, SnapshotManifest,
    build_router, graph_cors_layer, trim_allocator,
};
use serde::Deserialize;
use std::{
    env, fs, io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

const CACHE: &str = "public, max-age=60, stale-while-revalidate=60";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone)]
struct Reader {
    loaded: Arc<RwLock<Option<Arc<Generation>>>>,
    max_age: u64,
}

struct Generation {
    directory: PathBuf,
    completed_at: u64,
    generated_at: u64,
    graph_since: u64,
    root: String,
    id: String,
    routes: Router,
}

#[derive(Clone)]
struct Files {
    canonical: Router,
    graph_v3: Bytes,
    mobile: Bytes,
    mobile_etag: HeaderValue,
    root: HeaderValue,
    allowlist: Bytes,
    index: Bytes,
}

fn read_member(directory: &Path, name: &str) -> io::Result<Vec<u8>> {
    let path = directory.join(name);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid("snapshot member is not a regular file"));
    }
    fs::read(path)
}

fn read_verified_member(
    directory: &Path,
    manifest: &SnapshotManifest,
    name: &str,
) -> io::Result<Vec<u8>> {
    let entry = manifest
        .files
        .get(name)
        .ok_or_else(|| invalid("missing manifest member"))?;
    let data = read_member(directory, name)?;
    if data.len() as u64 != entry.size || sha256::Hash::hash(&data).to_string() != entry.sha256 {
        return Err(invalid(&format!(
            "snapshot size or digest mismatch for {name}"
        )));
    }
    Ok(data)
}

fn load(directory: &Path, root: &str, previous: Option<&Generation>) -> io::Result<Generation> {
    let bytes = read_member(directory, "manifest.json")?;
    let id = sha256::Hash::hash(&bytes).to_string();
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes)?;
    if manifest.version != 1
        || manifest.root != root
        || manifest.files.len() != SNAPSHOT_FILES.len()
        || SNAPSHOT_FILES
            .iter()
            .any(|name| !manifest.files.contains_key(*name))
        || manifest.graph_completed_at == 0
        || manifest.graph_completed_at > manifest.generated_at
        || manifest.generated_at > now()
        || manifest.graph_since > manifest.graph_completed_at
    {
        return Err(invalid("unsupported or inconsistent snapshot manifest"));
    }
    if previous.is_some_and(|g| {
        manifest.graph_completed_at < g.completed_at || manifest.generated_at < g.generated_at
    }) {
        return Err(invalid("snapshot generation moved backwards"));
    }

    // Verify and decode transient parse-only files first, then immediately drop
    // their raw buffers so peak memory stays minimal during generation reloads.
    let graph = {
        let graph_v4 = read_verified_member(directory, &manifest, "social-graph-v4.bin")?;
        if graph_v4.first() != Some(&4) {
            return Err(invalid("unexpected graph format or mobile budget"));
        }
        SocialGraph::from_binary(root, &graph_v4).map_err(io::Error::other)?
    };
    let profiles = {
        let profile_bytes = read_verified_member(directory, &manifest, "profile-data.json")?;
        let profile_path = directory.join("profile-data.json");
        ProfileStore::from_snapshot_bytes(profile_path, &profile_bytes).map_err(io::Error::other)?
    };
    let index = Bytes::from(read_verified_member(
        directory,
        &manifest,
        "profile-index.json",
    )?);
    // Verify both arrays without allocating an additional copy of every search record.
    #[derive(Deserialize)]
    struct IndexShape {
        keys: Vec<serde::de::IgnoredAny>,
        records: Vec<serde::de::IgnoredAny>,
    }
    let shape: IndexShape = serde_json::from_slice(&index)?;
    if shape.keys.is_empty() || shape.records.len() != profiles.profile_count() {
        return Err(invalid("profile index and profile rows disagree"));
    }

    let graph_v3 = Bytes::from(read_verified_member(
        directory,
        &manifest,
        "social-graph.bin",
    )?);
    let mobile = Bytes::from(read_verified_member(directory, &manifest, "mobile.bin")?);
    if graph_v3.first() != Some(&3) || mobile.first() != Some(&4) || mobile.len() > 4 * 1024 * 1024
    {
        return Err(invalid("unexpected graph format or mobile budget"));
    }

    let allowlist = Bytes::from(read_verified_member(directory, &manifest, "allowlist.txt")?);
    std::str::from_utf8(&allowlist).map_err(io::Error::other)?;
    let state = AppState::new_shared(
        Arc::new(RwLock::new(graph)),
        Arc::new(RwLock::new(profiles)),
        directory.join("profile-index.json"),
    );
    let canonical = build_router(state);
    let mobile_etag = HeaderValue::from_str(&format!(
        "\"{root}:{}\"",
        manifest.files["mobile.bin"].sha256
    ))
    .map_err(io::Error::other)?;
    let routes = Router::new()
        .route("/social-graph", get(graph_response))
        .route("/social-graph/mobile", get(mobile_response))
        .route("/allowlist", get(allowlist_response))
        .route("/profile-index", get(index_response))
        .with_state(Files {
            canonical: canonical.clone(),
            graph_v3,
            mobile,
            mobile_etag,
            root: HeaderValue::from_str(root).map_err(io::Error::other)?,
            allowlist,
            index,
        })
        .fallback_service(canonical);
    Ok(Generation {
        directory: directory.to_path_buf(),
        completed_at: manifest.graph_completed_at,
        generated_at: manifest.generated_at,
        graph_since: manifest.graph_since,
        root: root.to_owned(),
        id,
        routes,
    })
}

fn raw(data: Bytes, content_type: &'static str) -> Response {
    ([(CONTENT_TYPE, content_type), (CACHE_CONTROL, CACHE)], data).into_response()
}

async fn forward(routes: Router, request: Request) -> Response {
    routes.oneshot(request).await.expect("router is infallible")
}

async fn graph_response(State(files): State<Files>, request: Request) -> Response {
    if request.uri().query().is_some_and(|q| !q.is_empty()) {
        return forward(files.canonical, request).await;
    }
    let mut response = raw(files.graph_v3, "application/octet-stream");
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"social-graph.bin\""),
    );
    response
}

async fn allowlist_response(State(files): State<Files>, request: Request) -> Response {
    if request.uri().query().is_some_and(|q| !q.is_empty()) {
        return forward(files.canonical, request).await;
    }
    raw(files.allowlist, "text/plain; charset=utf-8")
}

async fn index_response(State(files): State<Files>) -> Response {
    raw(files.index, "application/json; charset=utf-8")
}

#[derive(Deserialize)]
struct MobileQuery {
    root: Option<String>,
}

async fn mobile_response(
    State(files): State<Files>,
    Query(query): Query<MobileQuery>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    if query
        .root
        .as_deref()
        .is_some_and(|r| r.as_bytes() != files.root.as_bytes())
    {
        return forward(files.canonical, request).await;
    }
    let mut response = if headers.get(IF_NONE_MATCH) == Some(&files.mobile_etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        files.mobile.into_response()
    };
    response.headers_mut().insert(ETAG, files.mobile_etag);
    response.headers_mut().insert("x-graph-root", files.root);
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(CACHE));
    response
}

async fn health(State(reader): State<Reader>) -> Response {
    let loaded = reader.loaded.read().expect("reader lock").clone();
    let age = loaded
        .as_ref()
        .map(|g| now().saturating_sub(g.completed_at));
    let healthy = age.is_some_and(|seconds| seconds < reader.max_age);
    let generation = loaded.map(|g| {
        serde_json::json!({
            "id": g.id, "root": g.root, "graph_completed_at": g.completed_at,
            "generated_at": g.generated_at, "graph_since": g.graph_since,
        })
    });
    (
        if healthy {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        [(CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
           "healthy": healthy, "mode": "snapshot-reader", "age_seconds": age,
           "max_age_seconds": reader.max_age, "generation": generation,
        })),
    )
        .into_response()
}

async fn dispatch(State(reader): State<Reader>, request: Request) -> Response {
    let routes = reader
        .loaded
        .read()
        .expect("reader lock")
        .as_ref()
        .map(|g| g.routes.clone());
    match routes {
        Some(routes) => forward(routes, request).await,
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(CACHE_CONTROL, "no-store")],
            "Waiting for a verified snapshot",
        )
            .into_response(),
    }
}

async fn watch(reader: Reader, current: PathBuf, root: String, interval: Duration) {
    let mut last_failure = None;
    loop {
        let current = current.clone();
        let root = root.clone();
        let previous = reader.loaded.read().expect("reader lock").clone();
        let attempt = tokio::task::spawn_blocking(move || {
            let directory = fs::canonicalize(&current)?;
            if previous.as_ref().is_some_and(|g| g.directory == directory) {
                return Ok(None);
            }
            let generation = load(&directory, &root, previous.as_deref())?;
            if fs::canonicalize(&current)? != directory {
                return Err(invalid("current changed during verification"));
            }
            Ok(Some(generation))
        })
        .await
        .map_err(io::Error::other)
        .and_then(|result| result);
        match attempt {
            Ok(generation) => {
                if let Some(generation) = generation {
                    tracing::info!(id = %generation.id, completed_at = generation.completed_at, "loaded snapshot");
                    let previous = reader
                        .loaded
                        .write()
                        .expect("reader lock")
                        .replace(Arc::new(generation));
                    // Dropping a large graph can be expensive. Let new requests
                    // use the replacement while the old indexes are released,
                    // then trim glibc arenas back to the OS.
                    let _ = tokio::task::spawn_blocking(move || {
                        drop(previous);
                        trim_allocator();
                    })
                    .await;
                }
                last_failure = None;
            }
            Err(error) => {
                let message = error.to_string();
                if last_failure.as_ref() != Some(&message) {
                    tracing::warn!(%error, "snapshot reload failed");
                    last_failure = Some(message);
                }
            }
        }
        tokio::time::sleep(interval).await;
    }
}

fn setting<T: std::str::FromStr>(name: &str, fallback: T) -> io::Result<T> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| invalid(&format!("invalid {name}"))),
        Err(env::VarError::NotPresent) => Ok(fallback),
        Err(error) => Err(io::Error::other(error)),
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let current = env::var_os("GRAPH_SNAPSHOT_CURRENT")
        .map(PathBuf::from)
        .ok_or_else(|| invalid("GRAPH_SNAPSHOT_CURRENT must point to the installed generation"))?;
    let root =
        env::var("SOCIAL_GRAPH_ROOT").unwrap_or_else(|_| DEFAULT_SOCIAL_GRAPH_ROOT.to_owned());
    let port = setting("PORT", 3000u16)?;
    let ip = setting("BIND_ADDRESS", IpAddr::from([127, 0, 0, 1]))?;
    let max_age = setting("GRAPH_SNAPSHOT_MAX_AGE_SECS", 7200u64)?;
    let reload = setting("GRAPH_SNAPSHOT_RELOAD_SECS", 5u64)?;
    if max_age == 0 || reload == 0 {
        return Err(invalid("snapshot age and reload interval must be positive"));
    }
    let reader = Reader {
        loaded: Arc::new(RwLock::new(None)),
        max_age,
    };
    let app = Router::new()
        .route("/healthz", get(health))
        .fallback(dispatch)
        .with_state(reader.clone())
        .layer(graph_cors_layer(env::var("ALLOW_ORIGIN").ok().as_deref()));
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(ip, port)).await?;
    tokio::spawn(watch(reader, current, root, Duration::from_secs(reload)));
    axum::serve(listener, app).await
}
