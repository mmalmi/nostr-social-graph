//! Hashtree-backed storage for `nostr-social-graph`.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::executor::block_on;
use hashtree_core::{
    Cid, CidParseError, HashTree, HashTreeConfig, HashTreeError, Store, StoreError, collect_hashes,
    sha256, to_hex,
};
use hashtree_fs::FsBlobStore;
use nostr_social_graph::{
    NostrEvent, SocialGraph, SocialGraphBackend, SocialGraphError, SocialGraphState,
};
use serde::{Deserialize, Serialize};

const MANIFEST_FILE: &str = "social-graph-root.json";
const PREVIOUS_MANIFEST_FILE: &str = "social-graph-root.previous.json";
const BLOBS_DIR: &str = "blobs";
const MANIFEST_SCHEMA: &str = "nostr-social-graph.hashtree.v1";
const MAX_SNAPSHOT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum HashtreeSocialGraphError {
    #[error(transparent)]
    Graph(#[from] SocialGraphError),
    #[error(transparent)]
    HashTree(#[from] HashTreeError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Cid(#[from] CidParseError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unsupported social graph hashtree manifest schema {0}")]
    UnsupportedManifestSchema(String),
    #[error("stored social graph root is empty")]
    EmptyStoredRoot,
    #[error("snapshot {cid} is missing from hashtree storage")]
    MissingSnapshot { cid: String },
    #[error("snapshot {cid} size {actual} does not match manifest size {expected}")]
    SnapshotSizeMismatch {
        cid: String,
        expected: u64,
        actual: u64,
    },
    #[error("social graph checkpoint changed since it was opened; reopen before writing")]
    StaleCheckpoint,
    #[error("snapshot blob failed hash verification: {0}")]
    CorruptBlob(String),
}

pub type Result<T> = std::result::Result<T, HashtreeSocialGraphError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub schema: String,
    pub root_pubkey: String,
    pub cid: String,
    pub size: u64,
}

impl SnapshotManifest {
    fn new(root_pubkey: String, cid: &Cid, size: u64) -> Self {
        Self {
            schema: MANIFEST_SCHEMA.to_string(),
            root_pubkey,
            cid: cid.to_string(),
            size,
        }
    }

    fn validate(&self) -> Result<Cid> {
        if self.schema != MANIFEST_SCHEMA {
            return Err(HashtreeSocialGraphError::UnsupportedManifestSchema(
                self.schema.clone(),
            ));
        }
        if self.root_pubkey.trim().is_empty() {
            return Err(HashtreeSocialGraphError::EmptyStoredRoot);
        }
        Ok(Cid::parse(&self.cid)?)
    }
}

pub struct HashtreeSocialGraph {
    manifest_path: PathBuf,
    store: Arc<FsBlobStore>,
    tree: HashTree<FsBlobStore>,
    graph: SocialGraph,
    manifest: Option<SnapshotManifest>,
    dirty: bool,
}

impl HashtreeSocialGraph {
    pub fn open<P: AsRef<Path>>(path: P, default_root: &str) -> Result<Self> {
        let path = path.as_ref();
        fs::create_dir_all(path)?;
        // Readers must finish loading a checkpoint before a writer can retire it.
        let _lock = lock_store(path, false)?;

        let store = Arc::new(FsBlobStore::new(path.join(BLOBS_DIR))?);
        let tree = HashTree::new(HashTreeConfig::new(store.clone()).public());
        let manifest_path = path.join(MANIFEST_FILE);
        let manifest = read_manifest(&manifest_path)?;

        let graph = match &manifest {
            Some(manifest) => {
                let cid = manifest.validate()?;
                let data =
                    block_on(tree.get(&cid, Some(MAX_SNAPSHOT_BYTES)))?.ok_or_else(|| {
                        HashtreeSocialGraphError::MissingSnapshot {
                            cid: manifest.cid.clone(),
                        }
                    })?;
                let actual = data.len() as u64;
                if actual != manifest.size {
                    return Err(HashtreeSocialGraphError::SnapshotSizeMismatch {
                        cid: manifest.cid.clone(),
                        expected: manifest.size,
                        actual,
                    });
                }
                SocialGraph::from_binary(&manifest.root_pubkey, &data)?
            }
            None => SocialGraph::new(default_root),
        };

        Ok(Self {
            manifest_path,
            store,
            tree,
            graph,
            manifest,
            dirty: false,
        })
    }

    pub fn latest_cid(&self) -> Option<Cid> {
        self.manifest
            .as_ref()
            .and_then(|manifest| Cid::parse(&manifest.cid).ok())
    }

    pub fn latest_manifest(&self) -> Option<&SnapshotManifest> {
        self.manifest.as_ref()
    }

    pub fn snapshot_exists(&self, cid: &Cid) -> Result<bool> {
        Ok(block_on(self.store.has(&cid.hash))?)
    }

    pub fn export_state(&self) -> SocialGraphState {
        self.graph.export_state()
    }

    pub fn replace_state(&mut self, state: &SocialGraphState) -> Result<()> {
        self.graph = SocialGraph::from_state(state.clone())?;
        self.write_snapshot()
    }

    fn write_snapshot(&mut self) -> Result<()> {
        let directory = self.manifest_path.parent().expect("manifest parent");
        let _lock = lock_store(directory, true)?;
        let previous = read_manifest(&self.manifest_path)?;
        if previous != self.manifest {
            return Err(HashtreeSocialGraphError::StaleCheckpoint);
        }
        // Fail closed before publishing or removing anything if the durable
        // checkpoint is incomplete. This also provides the rollback root.
        let mut retained = HashSet::new();
        if let Some(manifest) = &previous {
            retained.extend(self.verified_snapshot_hashes(manifest)?);
        }
        let root = self.graph.get_root().to_string();
        let data = self.graph.to_binary()?;
        let (cid, size) = block_on(self.tree.put(&data))?;
        let manifest = SnapshotManifest::new(root, &cid, size);
        let current_hashes = self.verified_snapshot_hashes(&manifest)?;
        retained.extend(current_hashes);
        let previous_path = directory.join(PREVIOUS_MANIFEST_FILE);
        if previous.as_ref() == Some(&manifest) {
            if let Some(previous) = read_manifest(&previous_path)? {
                // An unchanged flush must not discard the distinct rollback root.
                retained.extend(self.verified_snapshot_hashes(&previous)?);
            }
        }
        // Both generations, including checkpoints created by older writers,
        // must be durable before either manifest is committed or GC starts.
        sync_blob_files(directory, &retained)?;
        if previous.as_ref() != Some(&manifest) {
            if let Some(previous) = &previous {
                write_manifest_atomic(&previous_path, previous)?;
            }
        }
        write_manifest_atomic(&self.manifest_path, &manifest)?;
        self.manifest = Some(manifest);
        self.dirty = false;
        // This directory is exclusively the graph's checkpoint store, not a
        // shared Blossom cache. Retain both committed generations and all
        // explicitly pinned blobs; only unreachable old snapshots are garbage.
        for hash in self.store.list()? {
            if !retained.contains(&hash) && self.store.pin_count(&hash) == 0 {
                self.store.delete_sync(&hash)?;
            }
        }
        Ok(())
    }

    fn verified_snapshot_hashes(&self, manifest: &SnapshotManifest) -> Result<HashSet<[u8; 32]>> {
        let cid = manifest.validate()?;
        let data = block_on(self.tree.get(&cid, Some(MAX_SNAPSHOT_BYTES)))?.ok_or_else(|| {
            HashtreeSocialGraphError::MissingSnapshot {
                cid: manifest.cid.clone(),
            }
        })?;
        if data.len() as u64 != manifest.size {
            return Err(HashtreeSocialGraphError::SnapshotSizeMismatch {
                cid: manifest.cid.clone(),
                expected: manifest.size,
                actual: data.len() as u64,
            });
        }
        // Traversal alone tolerates missing blobs in the SDK. Full readback,
        // graph decoding and explicit hash checks must precede any deletion.
        SocialGraph::from_binary(&manifest.root_pubkey, &data)?;
        let hashes = block_on(collect_hashes(&self.tree, &cid, 1))?;
        for hash in &hashes {
            let blob = self.store.get_sync(hash)?.ok_or_else(|| {
                HashtreeSocialGraphError::MissingSnapshot {
                    cid: manifest.cid.clone(),
                }
            })?;
            if sha256(&blob) != *hash {
                return Err(HashtreeSocialGraphError::CorruptBlob(to_hex(hash)));
            }
        }
        Ok(hashes)
    }
}

impl SocialGraphBackend for HashtreeSocialGraph {
    type Error = HashtreeSocialGraphError;

    fn get_root(&self) -> std::result::Result<String, Self::Error> {
        Ok(self.graph.get_root().to_string())
    }

    fn set_root(&mut self, root: &str) -> std::result::Result<(), Self::Error> {
        let current = self.graph.get_root().to_string();
        self.graph.set_root(root)?;
        if current != self.graph.get_root() {
            self.dirty = true;
        }
        Ok(())
    }

    fn handle_event(
        &mut self,
        event: &NostrEvent,
        allow_unknown_authors: bool,
        overmute_threshold: f64,
    ) -> std::result::Result<(), Self::Error> {
        self.graph
            .handle_event(event, allow_unknown_authors, overmute_threshold);
        if matches!(event.kind, 3 | 10_000) {
            self.dirty = true;
        }
        Ok(())
    }

    fn get_follow_distance(&self, user: &str) -> std::result::Result<u32, Self::Error> {
        Ok(self.graph.get_follow_distance(user))
    }

    fn is_following(
        &self,
        follower: &str,
        followed_user: &str,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(self.graph.is_following(follower, followed_user))
    }

    fn get_followed_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(self.graph.get_followed_by_user(user))
    }

    fn get_followers_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(self.graph.get_followers_by_user(user))
    }

    fn get_muted_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(self.graph.get_muted_by_user(user))
    }

    fn get_user_muted_by(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(self.graph.get_user_muted_by(user))
    }

    fn get_follow_list_created_at(
        &self,
        user: &str,
    ) -> std::result::Result<Option<u64>, Self::Error> {
        Ok(self.graph.get_follow_list_created_at(user))
    }

    fn get_mute_list_created_at(
        &self,
        user: &str,
    ) -> std::result::Result<Option<u64>, Self::Error> {
        Ok(self.graph.get_mute_list_created_at(user))
    }

    fn is_overmuted(&self, user: &str, threshold: f64) -> std::result::Result<bool, Self::Error> {
        Ok(self.graph.is_overmuted(user, threshold))
    }

    fn flush(&mut self) -> std::result::Result<(), Self::Error> {
        if self.dirty {
            self.write_snapshot()?;
        }
        Ok(())
    }

    fn has_unflushed_changes(&self) -> bool {
        self.dirty
    }
}

fn read_manifest(path: &Path) -> Result<Option<SnapshotManifest>> {
    if !path.exists() {
        return Ok(None);
    }
    let manifest: SnapshotManifest = serde_json::from_slice(&fs::read(path)?)?;
    manifest.validate()?;
    Ok(Some(manifest))
}

fn write_manifest_atomic(path: &Path, manifest: &SnapshotManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("json.tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(&serde_json::to_vec_pretty(manifest)?)?;
    file.sync_all()?;
    fs::rename(temp_path, path)?;
    #[cfg(unix)]
    File::open(path.parent().expect("manifest parent"))?.sync_all()?;
    Ok(())
}

fn lock_store(directory: &Path, exclusive: bool) -> Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("snapshot.lock"))?;
    if exclusive {
        lock.lock()?;
    } else {
        lock.lock_shared()?;
    }
    Ok(lock)
}

fn sync_blob_files(directory: &Path, hashes: &HashSet<[u8; 32]>) -> Result<()> {
    let base = directory.join(BLOBS_DIR);
    let mut directories = HashSet::from([base.clone()]);
    for hash in hashes {
        let hex = to_hex(hash);
        let sharded = base.join(&hex[..2]).join(&hex[2..4]).join(&hex[4..]);
        let path = if sharded.is_file() {
            sharded
        } else {
            base.join(&hex[..2]).join(&hex[2..])
        };
        File::open(&path)?.sync_all()?;
        let mut parent = path.parent();
        while let Some(path) = parent {
            if path == base {
                break;
            }
            directories.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    #[cfg(unix)]
    for directory in directories {
        File::open(directory)?.sync_all()?;
    }
    Ok(())
}
