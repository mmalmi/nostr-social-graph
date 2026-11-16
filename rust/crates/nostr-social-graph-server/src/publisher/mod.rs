use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nostr_sdk::hashes::{Hash, sha256};
use nostr_social_graph::{BinaryBudget, SocialGraph};
use tokio::sync::mpsc;

use super::{
    SNAPSHOT_FILES, SnapshotFile, SnapshotManifest, allowlist_text, mobile_snapshot_bytes,
};
use named_tree::NamedTree;

mod named_tree;

pub(super) struct GraphFiles {
    full_v3: Vec<u8>,
    full_v4: Vec<u8>,
    mobile: Vec<u8>,
    discovery_mobile: Option<Vec<u8>>,
    allowlist: String,
}

impl GraphFiles {
    // The caller holds the same graph read lock used to persist this checkpoint.
    // Produce both projections from that graph; decoding v4 would duplicate all
    // adjacency, reverse and distance indexes on the crawler.
    pub(super) fn capture(graph: &SocialGraph, full_v4: Vec<u8>) -> io::Result<Self> {
        let started = Instant::now();
        let full_v3 = graph
            .to_binary_with_budget_version(BinaryBudget::default(), 3)
            .map_err(io::Error::other)?;
        let mobile = mobile_snapshot_bytes(graph, graph.get_root())?;
        let discovery_mobile = (graph.get_root() != super::MOBILE_DISCOVERY_ROOT)
            .then(|| mobile_snapshot_bytes(graph, super::MOBILE_DISCOVERY_ROOT))
            .transpose()?;
        let allowlist = allowlist_text(graph, None);
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            full_v3_bytes = full_v3.len(),
            full_v4_bytes = full_v4.len(),
            mobile_bytes = mobile.len(),
            allowlist_bytes = allowlist.len(),
            "captured named graph exports"
        );
        Ok(Self {
            full_v3,
            full_v4,
            mobile,
            discovery_mobile,
            allowlist,
        })
    }
}

pub(super) struct Snapshot {
    pub(super) root: String,
    pub(super) graph: GraphFiles,
    pub(super) rows: Vec<u8>,
    pub(super) index: Vec<u8>,
    pub(super) graph_completed_at: u64,
    pub(super) graph_since: u64,
    pub(super) generated_at: u64,
}

impl Snapshot {
    fn stage(&self) -> io::Result<tempfile::TempDir> {
        let stage = tempfile::Builder::new().prefix("graph-public-").tempdir()?;
        let directory = stage.path().join("files");
        fs::create_dir(&directory)?;
        let data: [&[u8]; 6] = [
            &self.graph.full_v3,
            &self.graph.full_v4,
            &self.graph.mobile,
            self.graph.allowlist.as_bytes(),
            &self.rows,
            &self.index,
        ];
        let bytes = data.iter().map(|data| data.len() as u64).sum::<u64>()
            + self
                .graph
                .discovery_mobile
                .as_ref()
                .map_or(0, |bytes| bytes.len() as u64);
        check_stage_capacity(&directory, bytes.saturating_add(4096))?;
        let mut files = std::collections::BTreeMap::new();
        for (name, bytes) in SNAPSHOT_FILES.into_iter().zip(data) {
            fs::write(directory.join(name), bytes)?;
            files.insert(
                name.to_owned(),
                SnapshotFile {
                    size: bytes.len() as u64,
                    sha256: sha256::Hash::hash(bytes).to_string(),
                },
            );
        }
        let mut mobile_roots = std::collections::BTreeMap::new();
        if let Some(bytes) = &self.graph.discovery_mobile {
            let root = super::MOBILE_DISCOVERY_ROOT;
            let file = format!("mobile-{root}.bin");
            fs::write(directory.join(&file), bytes)?;
            mobile_roots.insert(
                root.to_owned(),
                super::snapshot_manifest::MobileProjection {
                    file,
                    metadata: SnapshotFile {
                        size: bytes.len() as u64,
                        sha256: sha256::Hash::hash(bytes).to_string(),
                    },
                },
            );
        }
        let manifest = SnapshotManifest {
            version: 1,
            root: self.root.clone(),
            graph_completed_at: self.graph_completed_at,
            graph_since: self.graph_since,
            generated_at: self.generated_at,
            files,
            mobile_roots,
        };
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest)?,
        )?;
        Ok(stage)
    }
}

fn check_stage_capacity(path: &Path, bytes: u64) -> io::Result<()> {
    let reserve = std::env::var("GRAPH_CHECKPOINT_MIN_FREE_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5 * 1024 * 1024 * 1024);
    let free = super::forwarding::free_bytes(path)
        .ok_or_else(|| io::Error::other("Cannot verify snapshot staging capacity"))?;
    if free.saturating_sub(reserve) < bytes {
        return Err(io::Error::other(
            "Insufficient free space for snapshot staging",
        ));
    }
    Ok(())
}

type Pending = Arc<Mutex<Option<Arc<Snapshot>>>>;

pub(super) struct SnapshotPublisher {
    pending: Pending,
    wake: mpsc::Sender<()>,
    interval: Duration,
    last_capture: Option<Instant>,
}

impl SnapshotPublisher {
    pub(super) fn from_env() -> io::Result<Option<Self>> {
        let Some(tree) = NamedTree::from_env()? else {
            return Ok(None);
        };
        let seconds = match std::env::var("GRAPH_PUBLISH_INTERVAL_SECS") {
            Err(std::env::VarError::NotPresent) => 900,
            value => value
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| {
                    io::Error::other("GRAPH_PUBLISH_INTERVAL_SECS must be a positive integer")
                })?,
        };
        let pending = Arc::new(Mutex::new(None));
        let (wake, events) = mpsc::channel(1);
        tokio::spawn(worker(tree, pending.clone(), events));
        Ok(Some(Self {
            pending,
            wake,
            interval: Duration::from_secs(seconds),
            last_capture: None,
        }))
    }

    pub(super) fn due(&self) -> bool {
        self.last_capture
            .is_none_or(|at| at.elapsed() >= self.interval)
    }

    pub(super) fn submit(&mut self, snapshot: Snapshot) {
        let snapshot = Arc::new(snapshot);
        self.pending
            .lock()
            .expect("pending snapshot lock")
            .replace(snapshot);
        // A full channel already has a wakeup for the newest complete generation.
        let _ = self.wake.try_send(());
        self.last_capture = Some(Instant::now());
    }
}

enum Generation {
    Captured(Arc<Snapshot>),
    Staged(tempfile::TempDir),
}

impl Generation {
    async fn stage(self) -> Result<tempfile::TempDir, (Self, io::Error)> {
        match self {
            Self::Staged(directory) => Ok(directory),
            Self::Captured(snapshot) => {
                let source = Arc::clone(&snapshot);
                tokio::task::spawn_blocking(move || source.stage())
                    .await
                    .map_err(io::Error::other)
                    .and_then(|result| result)
                    .map_err(|error| (Self::Captured(snapshot), error))
            }
        }
    }

    async fn publish(self, tree: &NamedTree) -> Result<(), (Self, io::Error)> {
        // After staging, the worker holds the files on disk, not the graph or
        // serialized buffers. A network retry can use the same complete stage.
        let directory = self.stage().await?;
        tree.publish(&directory.path().join("files"))
            .await
            .map_err(|error| (Self::Staged(directory), error))
    }
}

async fn worker(tree: NamedTree, pending: Pending, mut events: mpsc::Receiver<()>) {
    while events.recv().await.is_some() {
        let mut retry = None;
        loop {
            let latest = pending.lock().expect("pending snapshot lock").take();
            let next = latest.map(Generation::Captured).or(retry.take());
            let Some(generation) = next else { break };
            match generation.publish(&tree).await {
                Ok(()) => tracing::info!("published named social graph snapshot"),
                Err((generation, error)) => {
                    tracing::warn!(
                        "named snapshot publication failed; retrying in 30 seconds: {error}"
                    );
                    retry = Some(generation);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    if events.is_closed() {
                        return;
                    }
                }
            }
        }
    }
}
