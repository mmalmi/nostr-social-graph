use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SNAPSHOT_FILES: [&str; 6] = [
    "social-graph.bin",
    "social-graph-v4.bin",
    "mobile.bin",
    "allowlist.txt",
    "profile-data.json",
    "profile-index.json",
];

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub version: u32,
    pub root: String,
    pub graph_completed_at: u64,
    pub graph_since: u64,
    pub generated_at: u64,
    pub files: BTreeMap<String, SnapshotFile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mobile_roots: BTreeMap<String, MobileProjection>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MobileProjection {
    pub file: String,
    #[serde(flatten)]
    pub metadata: SnapshotFile,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub size: u64,
    pub sha256: String,
}
