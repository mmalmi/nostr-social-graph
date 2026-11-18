//! Optional anonymous bootstrap from Vader's signed public directory. Call this
//! only for an empty public-discovery cache. Dropping the future cancels I/O.
use crate::{MOBILE_MAX_BYTES, MobileGraphError, SocialGraph};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt, future::select_ok};
use hashtree_core::{
    Cid, Hash, HashTree, HashTreeConfig, LinkType, ObjectKey, Store, StoreError, TreeEntry, sha256,
    to_hex,
};
use nostr::{Event, PublicKey};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

const PUBLISHER: &str = "npub1yscmnnadrmvape0mvgzfx9uyl267klnhzy8znsr6ysl8wtxngpqsnx6dm0";
const DIRECTORY: &str = "social-graph";
const RELAYS: [&str; 2] = ["wss://temp.iris.to", "wss://relay.primal.net"];
const BLOCKS: &str = "https://cdn.iris.to";
const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_BLOCK: usize = MOBILE_MAX_BYTES + 128 * 1024;
const MAX_WIRE: usize = 2 * MAX_BLOCK;
const MAX_BLOCK_READS: usize = 128;
const MAX_FRAME: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("public graph snapshot unavailable: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Graph(#[from] MobileGraphError),
    #[error(transparent)]
    Tree(#[from] hashtree_core::HashTreeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub struct MobileSnapshot {
    /// Strictly decoded from the same bytes as the checksum in the signed directory.
    pub graph: SocialGraph,
    pub bytes: Vec<u8>,
    pub announcement_id: String,
    pub completed_at: u64,
}

/// Fetch only the manifest and the requested mobile projection; no account key,
/// viewer-specific relay filter, or authentication header is sent. Applications
/// keep rendering their embedded graph while this future is pending.
pub async fn fetch(root: &str, min_users: usize) -> Result<MobileSnapshot, SnapshotError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SnapshotError::Invalid("clock"))?
        .as_secs();
    let publisher = PublicKey::parse(PUBLISHER).map_err(|_| SnapshotError::Invalid("publisher"))?;
    tokio::time::timeout(Duration::from_secs(8), async {
        let requests = RELAYS
            .iter()
            .map(|relay| Box::pin(query_name(relay, publisher, now)));
        let ((cid, id), remaining) = select_ok(requests).await?;
        drop(remaining);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| SnapshotError::Invalid("HTTP client"))?;
        let store = Arc::new(Blocks {
            client,
            requests: 0.into(),
            bytes: 0.into(),
        });
        read_named(store, cid, id, now, root, min_users).await
    })
    .await
    .map_err(|_| SnapshotError::Invalid("deadline"))?
}

async fn query_name(
    relay: &str,
    publisher: PublicKey,
    now: u64,
) -> Result<(Cid, String), SnapshotError> {
    let config = WebSocketConfig::default()
        .max_frame_size(Some(MAX_FRAME))
        .max_message_size(Some(MAX_FRAME));
    let (mut socket, _) = connect_async_with_config(relay, Some(config), false)
        .await
        .map_err(|_| SnapshotError::Invalid("relay unavailable"))?;
    let request = json!(["REQ","graph-name",{"authors":[publisher.to_hex()],"kinds":[30064],"#d":[DIRECTORY],"#l":["hashtree"],"limit":1}]);
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .map_err(|_| SnapshotError::Invalid("relay send"))?;
    for _ in 0..16 {
        let Some(Ok(frame)) = socket.next().await else {
            break;
        };
        let Message::Text(raw) = frame else { continue };
        let Ok(Value::Array(mut frame)) = serde_json::from_str(&raw) else {
            continue;
        };
        if frame.get(1).and_then(Value::as_str) != Some("graph-name") {
            continue;
        }
        match frame.first().and_then(Value::as_str) {
            Some("EOSE" | "CLOSED") => break,
            Some("EVENT") if frame.len() == 3 => {
                if let Ok(event) = serde_json::from_value::<Event>(frame.swap_remove(2)) {
                    if let Some(cid) = public_root(&event, publisher, now) {
                        return Ok((cid, event.id.to_hex()));
                    }
                }
            }
            _ => {}
        }
    }
    Err(SnapshotError::Invalid("no valid signed directory"))
}

fn public_root(event: &Event, publisher: PublicKey, now: u64) -> Option<Cid> {
    if event.pubkey != publisher
        || event.kind.as_u16() != 30064
        || event.created_at.as_secs() > now.saturating_add(300)
        || event.verify().is_err()
    {
        return None;
    }
    let mut name = None;
    let mut hash = None;
    let mut key = None;
    let mut labeled = false;
    for tag in event.tags.iter().map(|t| t.as_slice()) {
        match tag.first()?.as_str() {
            "d" if name.is_none() => name = Some(tag.get(1)?.as_str()),
            "hash" if hash.is_none() => hash = Some(hashtree_core::from_hex(tag.get(1)?).ok()?),
            "key" if key.is_none() && tag.get(1)?.len() == 64 => {
                key = Some(ObjectKey::from_hex(tag.get(1)?).ok()?)
            }
            "d" | "hash" | "key" | "selfEncryptedKey" | "encryptedKey" | "encrypted_key" => {
                return None;
            }
            "l" if tag.get(1).map(String::as_str) == Some("hashtree") => labeled = true,
            _ => {}
        }
    }
    if name != Some(DIRECTORY) || !labeled {
        return None;
    }
    Some(Cid {
        hash: hash?,
        key: Some(key?),
    })
}

#[derive(Deserialize)]
struct Manifest {
    version: u32,
    root: String,
    graph_since: u64,
    graph_completed_at: u64,
    generated_at: u64,
    files: HashMap<String, File>,
    #[serde(default)]
    mobile_roots: HashMap<String, Projection>,
}
#[derive(Deserialize)]
struct File {
    size: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct Projection {
    file: String,
    #[serde(flatten)]
    info: File,
}

async fn read_named<S: Store>(
    store: Arc<S>,
    cid: Cid,
    id: String,
    now: u64,
    root: &str,
    min_users: usize,
) -> Result<MobileSnapshot, SnapshotError> {
    use SnapshotError::Invalid;
    let tree = HashTree::new(HashTreeConfig::new(store).public());
    let mut entries = HashMap::new();
    for entry in tree.list_directory_required(&cid).await? {
        if entries.insert(entry.name.clone(), entry).is_some() {
            return Err(Invalid("duplicate directory entry"));
        }
    }
    let manifest_entry = entries
        .get("manifest.json")
        .ok_or(Invalid("missing manifest"))?;
    let manifest: Manifest =
        serde_json::from_slice(&read_entry(&tree, manifest_entry, MAX_MANIFEST).await?)?;
    if manifest.version != 1
        || !crate::is_valid_pubkey(&manifest.root)
        || manifest.graph_completed_at == 0
        || manifest.graph_since > manifest.graph_completed_at
        || manifest.graph_completed_at > manifest.generated_at
        || manifest.generated_at > now.saturating_add(300)
    {
        return Err(Invalid("manifest"));
    }
    for name in [
        "social-graph.bin",
        "social-graph-v4.bin",
        "mobile.bin",
        "allowlist.txt",
        "profile-data.json",
        "profile-index.json",
    ] {
        if !manifest
            .files
            .get(name)
            .is_some_and(|f| crate::is_valid_pubkey(&f.sha256))
        {
            return Err(Invalid("manifest file"));
        }
    }
    if manifest.files["mobile.bin"].size > MOBILE_MAX_BYTES as u64 {
        return Err(Invalid("oversized mobile file"));
    }
    for (key, projection) in &manifest.mobile_roots {
        if !crate::is_valid_pubkey(key)
            || projection.file != format!("mobile-{key}.bin")
            || projection.info.size > MOBILE_MAX_BYTES as u64
            || !crate::is_valid_pubkey(&projection.info.sha256)
        {
            return Err(Invalid("mobile projection"));
        }
    }
    let (name, file) = if root == manifest.root {
        ("mobile.bin", &manifest.files["mobile.bin"])
    } else {
        let alt = manifest
            .mobile_roots
            .get(root)
            .ok_or(Invalid("no projection for root"))?;
        (alt.file.as_str(), &alt.info)
    };
    let entry = entries.get(name).ok_or(Invalid("missing mobile file"))?;
    if entry.size != file.size {
        return Err(Invalid("mobile size mismatch"));
    }
    let bytes = read_entry(&tree, entry, MOBILE_MAX_BYTES as u64).await?;
    if to_hex(&sha256(&bytes)) != file.sha256 {
        return Err(Invalid("mobile digest mismatch"));
    }
    let graph = SocialGraph::from_mobile_binary(root, &bytes, min_users)?;
    Ok(MobileSnapshot {
        graph,
        bytes,
        announcement_id: id,
        completed_at: manifest.graph_completed_at,
    })
}

async fn read_entry<S: Store>(
    tree: &HashTree<S>,
    entry: &TreeEntry,
    limit: u64,
) -> Result<Vec<u8>, SnapshotError> {
    if !matches!(entry.link_type, LinkType::File | LinkType::Blob) || entry.size > limit {
        return Err(SnapshotError::Invalid("file size or type"));
    }
    let bytes = tree
        .get(
            &Cid {
                hash: entry.hash,
                key: entry.key,
            },
            Some(entry.size),
        )
        .await?
        .ok_or(SnapshotError::Invalid("missing file"))?;
    if bytes.len() as u64 != entry.size {
        return Err(SnapshotError::Invalid("file length mismatch"));
    }
    Ok(bytes)
}

struct Blocks {
    client: reqwest::Client,
    requests: AtomicUsize,
    bytes: AtomicUsize,
}

#[async_trait]
impl Store for Blocks {
    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        if self.requests.fetch_add(1, Ordering::Relaxed) >= MAX_BLOCK_READS {
            return Err(StoreError::Other(
                "snapshot request budget exhausted".into(),
            ));
        }
        let url = format!("{BLOCKS}/{}.bin", to_hex(hash));
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| StoreError::Other(e.to_string()))?;
        if !response.status().is_success() {
            return Ok(None);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BLOCK as u64)
        {
            return Err(StoreError::Other("oversized block".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| StoreError::Other(e.to_string()))?
        {
            if bytes.len() + chunk.len() > MAX_BLOCK
                || self
                    .bytes
                    .fetch_add(chunk.len(), Ordering::Relaxed)
                    .saturating_add(chunk.len())
                    > MAX_WIRE
            {
                return Err(StoreError::Other("snapshot byte budget exhausted".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        if sha256(&bytes) != *hash {
            return Err(StoreError::Other("snapshot block digest mismatch".into()));
        }
        Ok(Some(bytes))
    }
    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        Ok(self.get(hash).await?.is_some())
    }
    async fn put(&self, _: Hash, _: Vec<u8>) -> Result<bool, StoreError> {
        Err(StoreError::Other("read-only snapshot".into()))
    }
    async fn delete(&self, _: &Hash) -> Result<bool, StoreError> {
        Err(StoreError::Other("read-only snapshot".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{DirEntry, MemoryStore};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn sign(keys: &Keys, cid: &Cid, name: &str, now: u64) -> Event {
        EventBuilder::new(Kind::Custom(30064), "")
            .tags(
                [
                    vec!["d".into(), name.into()],
                    vec!["l".into(), "hashtree".into()],
                    vec!["hash".into(), to_hex(&cid.hash)],
                    vec!["key".into(), cid.key.unwrap().to_hex()],
                ]
                .into_iter()
                .map(|parts: Vec<String>| Tag::parse(parts).unwrap()),
            )
            .custom_created_at(Timestamp::from(now))
            .sign_with_keys(keys)
            .unwrap()
    }

    #[tokio::test]
    async fn signed_directory_and_named_file_are_verified_together() {
        let keys = Keys::generate();
        let (root, mobile) = crate::mobile::tests::fixture(50, 50);
        let store = Arc::new(MemoryStore::new());
        let tree = HashTree::new(HashTreeConfig::new(store.clone()))
            .with_encryption_mode(hashtree_core::EncryptionMode::LegacyChk);
        let (file, size) = tree.put(&mobile).await.unwrap();
        let mut manifest = json!({"version":1,"root":root,"graph_since":0,"graph_completed_at":9,"generated_at":10,"files":{}});
        for name in [
            "social-graph.bin",
            "social-graph-v4.bin",
            "mobile.bin",
            "allowlist.txt",
            "profile-data.json",
            "profile-index.json",
        ] {
            manifest["files"][name] = json!({"size":size,"sha256":to_hex(&sha256(&mobile))});
        }
        let (info, size_info) = tree
            .put(&serde_json::to_vec(&manifest).unwrap())
            .await
            .unwrap();
        let directory = tree
            .put_directory(vec![
                DirEntry::from_cid("manifest.json", &info).with_size(size_info),
                DirEntry::from_cid("mobile.bin", &file).with_size(size),
            ])
            .await
            .unwrap();
        let announcement = sign(&keys, &directory, DIRECTORY, 11);
        let verified = public_root(&announcement, keys.public_key(), 11).unwrap();
        let result = read_named(
            store.clone(),
            verified.clone(),
            announcement.id.to_hex(),
            11,
            &root,
            50,
        )
        .await
        .unwrap();
        assert_eq!(result.bytes, mobile);
        assert_eq!(result.completed_at, 9);
        assert!(
            read_named(
                store.clone(),
                verified.clone(),
                announcement.id.to_hex(),
                11,
                &root,
                51
            )
            .await
            .is_err()
        );
        assert!(
            read_named(
                store,
                verified,
                announcement.id.to_hex(),
                11,
                &"1".repeat(64),
                1
            )
            .await
            .is_err()
        );
        assert!(public_root(&announcement, Keys::generate().public_key(), 11).is_none());
        assert!(
            public_root(
                &sign(&keys, &directory, "another-tree", 11),
                keys.public_key(),
                11
            )
            .is_none()
        );
        assert!(
            public_root(
                &sign(&keys, &directory, DIRECTORY, 400),
                keys.public_key(),
                11
            )
            .is_none()
        );
        let mut altered = announcement;
        altered.content = "changed".into();
        assert!(public_root(&altered, keys.public_key(), 11).is_none());
    }
}
