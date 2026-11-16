use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use nostr_sdk::hashes::{Hash, sha256};
use nostr_sdk::{FromBech32, Keys, PublicKey};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

const TREE_NAME: &str = "social-graph";
const OUTPUT_LIMIT: u64 = 16 * 1024;

pub(super) struct NamedTree {
    binary: PathBuf,
    config_dir: PathBuf,
    data_dir: PathBuf,
    publisher: PublicKey,
    blossom: String,
    receipt_path: Option<PathBuf>,
}

#[derive(Deserialize, Serialize)]
struct RelayConfig {
    nostr: Relays,
}

#[derive(Deserialize, Serialize)]
struct Relays {
    relays: Vec<String>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("named graph publisher requires {name}")))
}

fn absolute_path(name: &str, directory: bool) -> io::Result<PathBuf> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() || (directory && !path.is_dir()) || (!directory && !path.is_file()) {
        return Err(invalid(format!(
            "{name} must identify an existing absolute path"
        )));
    }
    Ok(path)
}

fn private_directory(parent: &Path, prefix: &str) -> io::Result<TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir_in(parent)
}

fn store_space(directory: &Path) -> io::Result<u64> {
    let mut bytes = 0u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(invalid(
                "named graph bundle must contain only regular files",
            ));
        }
        bytes = bytes.saturating_add(entry.metadata()?.len());
    }
    // The native store writes encrypted chunks, directory nodes, and LMDB
    // indexes. Include room for temporary pack writes and database pages.
    Ok(bytes.saturating_mul(2).saturating_add(32 * 1024 * 1024))
}

fn endpoint(value: &str, schemes: &[&str]) -> bool {
    Url::parse(value).is_ok_and(|url| {
        schemes.contains(&url.scheme())
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn read_setting(path: &Path) -> io::Result<String> {
    const MAX_BYTES: u64 = 256 * 1024;
    if !path.is_file() {
        return Err(invalid(
            "Hashtree configuration requires existing config.toml and keys files",
        ));
    }
    let mut content = String::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut content)?;
    if content.len() as u64 > MAX_BYTES {
        return Err(invalid("Hashtree configuration file exceeds size limit"));
    }
    Ok(content)
}

fn signer_token(keys_file: &str, publisher: PublicKey) -> io::Result<&str> {
    // htree 0.4.5 signs with the first non-comment entry. Its optional alias
    // is separated by a literal space.
    let token = keys_file
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .and_then(|line| line.split(' ').next())
        .ok_or_else(|| invalid("Hashtree signer is missing"))?;
    if Keys::parse(token).map(|keys| keys.public_key()).ok() != Some(publisher) {
        return Err(invalid("Hashtree signer does not match GRAPH_PUBLISH_NPUB"));
    }
    Ok(token)
}

fn public_cid(output: &[u8]) -> io::Result<(&str, &str)> {
    let text = std::str::from_utf8(output).map_err(|_| invalid("invalid htree add output"))?;
    fn field<'a>(output: &'a str, name: &str) -> io::Result<&'a str> {
        let mut fields = output
            .lines()
            .filter_map(|line| line.trim().strip_prefix(name))
            .map(str::trim);
        let value = fields.next().unwrap_or_default();
        if fields.next().is_some()
            || value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "htree add must return one 32-byte hash and one 32-byte CHK key",
            ));
        }
        Ok(value)
    }
    Ok((field(text, "hash:")?, field(text, "key:")?))
}

fn handoff_path() -> io::Result<Option<PathBuf>> {
    let Some(raw) = env::var_os("GRAPH_PUBLISH_RECEIPT_PATH") else {
        return Ok(None);
    };
    let path = PathBuf::from(raw);
    if !path.is_absolute() || path.file_name().is_none() || path.is_dir() {
        return Err(invalid(
            "GRAPH_PUBLISH_RECEIPT_PATH must be an absolute file path",
        ));
    }
    let parent = path.parent().expect("absolute file").canonicalize()?;
    Ok(Some(parent.join(path.file_name().expect("file name"))))
}

fn write_upload_receipt(path: &Path, directory: &Path, hash: &str, key: &str) -> io::Result<()> {
    let bytes = fs::read(directory.join("manifest.json"))?;
    let manifest: crate::SnapshotManifest = serde_json::from_slice(&bytes)?;
    let receipt = serde_json::json!({
        "version": 1,
        "hash": hash,
        "key": key,
        "manifest_sha256": sha256::Hash::hash(&bytes).to_string(),
        "graph_completed_at": manifest.graph_completed_at,
    });
    let parent = path
        .parent()
        .ok_or_else(|| invalid("missing receipt parent"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(temp.as_file_mut(), &receipt)?;
    temp.as_file_mut().write_all(b"\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

impl NamedTree {
    pub(super) fn from_env() -> io::Result<Option<Self>> {
        match env::var("GRAPH_PUBLISH_HASHTREE") {
            Err(env::VarError::NotPresent) => return Ok(None),
            Ok(value) if value == "false" => return Ok(None),
            Ok(value) if value == "true" => (),
            _ => return Err(invalid("GRAPH_PUBLISH_HASHTREE must be true or false")),
        }
        let blossom = required("GRAPH_PUBLISH_BLOSSOM_URL")?;
        if !endpoint(&blossom, &["https", "http"]) {
            return Err(invalid(
                "GRAPH_PUBLISH_BLOSSOM_URL requires an HTTP(S) endpoint",
            ));
        }
        let tree = Self {
            publisher: PublicKey::from_bech32(&required("GRAPH_PUBLISH_NPUB")?)
                .map_err(|_| invalid("GRAPH_PUBLISH_NPUB requires a valid npub"))?,
            binary: absolute_path("GRAPH_PUBLISH_HTREE_BIN", false)?.canonicalize()?,
            config_dir: absolute_path("HTREE_CONFIG_DIR", true)?,
            data_dir: absolute_path("HTREE_DATA_DIR", true)?,
            blossom,
            receipt_path: handoff_path()?,
        };
        let private_dir = tree.config_dir.canonicalize()?;
        if tree
            .receipt_path
            .as_ref()
            .is_some_and(|path| path.starts_with(&private_dir))
        {
            return Err(invalid(
                "public upload receipt must be outside the private Hashtree config directory",
            ));
        }
        tree.settings()?;
        Ok(Some(tree))
    }

    fn settings(&self) -> io::Result<(String, String)> {
        let source = read_setting(&self.config_dir.join("keys"))?;
        let signer = signer_token(&source, self.publisher)?.to_owned();
        let settings: RelayConfig =
            toml::from_str(&read_setting(&self.config_dir.join("config.toml"))?).map_err(|_| {
                invalid("Hashtree config.toml requires an explicit [nostr].relays list")
            })?;
        if settings.nostr.relays.is_empty()
            || !settings
                .nostr
                .relays
                .iter()
                .all(|relay| endpoint(relay, &["wss", "ws"]))
        {
            return Err(invalid(
                "Hashtree requires at least one valid configured Nostr relay",
            ));
        }
        let mut config =
            toml::to_string(&settings).map_err(|_| invalid("invalid Hashtree relay settings"))?;
        let server = toml::Value::String(self.blossom.clone());
        config.push_str(&format!(
            "\n[blossom]\nservers = [{server}]\nread_servers = []\nwrite_servers = []\n"
        ));
        config.push_str("\n[updater]\nauto_check = false\nauto_install = false\n");
        Ok((signer, config))
    }

    fn private_config(&self) -> io::Result<TempDir> {
        let (signer, config) = self.settings()?;
        // Pin the validated first key for all three opens by htree. A service
        // credential can rotate atomically during a large upload. The CLI
        // otherwise silently creates an identity if its keys file disappears.
        let private = private_directory(&env::temp_dir(), "graph-publisher-")?;
        let mut open = fs::OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600);
        }
        open.open(private.path().join("keys"))?
            .write_all(signer.as_bytes())?;
        open.open(private.path().join("config.toml"))?
            .write_all(config.as_bytes())?;
        Ok(private)
    }

    async fn run(
        &self,
        config: &Path,
        store: &Path,
        step: &'static str,
        args: &[&OsStr],
        limit: Duration,
    ) -> io::Result<Vec<u8>> {
        let mut child = Command::new(&self.binary)
            .arg(step)
            .args(args)
            .env_clear()
            .env("HOME", config)
            .env("HTREE_CONFIG_DIR", config)
            .env("HTREE_DATA_DIR", store)
            .env("NOSTR_PREFER_LOCAL", "0")
            .current_dir(config)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(if step == "add" {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .kill_on_drop(true)
            .spawn()?;
        let result = timeout(limit, async {
            let mut output = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                stdout
                    .take(OUTPUT_LIMIT + 1)
                    .read_to_end(&mut output)
                    .await?;
                if output.len() as u64 > OUTPUT_LIMIT {
                    return Err(invalid("htree add output exceeds size limit"));
                }
            }
            let status = child.wait().await?;
            if !status.success() {
                return Err(io::Error::other(format!("htree {step} failed ({status})")));
            }
            Ok(output)
        })
        .await
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("htree {step} timed out"),
            ))
        });
        if result.is_err() {
            let _ = child.start_kill();
            let _ = timeout(Duration::from_secs(5), child.wait()).await;
        }
        result
    }

    pub(super) async fn publish(&self, directory: &Path) -> io::Result<()> {
        let directory = directory.canonicalize()?;
        let data_dir = self.data_dir.canonicalize()?;
        if !directory.is_dir()
            || self.config_dir.canonicalize()?.starts_with(&directory)
            || data_dir.starts_with(&directory)
            || env::temp_dir().canonicalize()?.starts_with(&directory)
        {
            return Err(invalid(
                "named graph input must be a dedicated snapshot directory",
            ));
        }
        let store = private_directory(&data_dir, "graph-cas-")?;
        super::check_stage_capacity(store.path(), store_space(&directory)?)?;
        let config = self.private_config()?;
        let result = self
            .upload_and_announce(&directory, config.path(), store.path())
            .await;
        let store_cleanup = store.close();
        let config_cleanup = config.close();
        result.and(store_cleanup).and(config_cleanup)
    }

    async fn upload_and_announce(
        &self,
        directory: &Path,
        config: &Path,
        store: &Path,
    ) -> io::Result<()> {
        let output = self
            .run(
                config,
                store,
                "add",
                &[
                    directory.as_os_str(),
                    "--local".as_ref(),
                    "--legacy-keys".as_ref(),
                    "--no-ignore".as_ref(),
                ],
                Duration::from_secs(300),
            )
            .await?;
        let (hash, key) = public_cid(&output)?;
        let cid = format!("{hash}:{key}");
        self.run(
            config,
            store,
            "push",
            &[cid.as_ref(), "--server".as_ref(), self.blossom.as_ref()],
            Duration::from_secs(900),
        )
        .await?;
        if let Some(path) = &self.receipt_path {
            return write_upload_receipt(path, directory, hash, key);
        }
        self.run(
            config,
            store,
            "publish",
            &[
                TREE_NAME.as_ref(),
                hash.as_ref(),
                "--key".as_ref(),
                key.as_ref(),
            ],
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::ToBech32;

    #[test]
    fn measure_regular_snapshot_files_before_creating_blob_data() {
        let stage = tempfile::tempdir().unwrap();
        fs::write(stage.path().join("one"), b"abc").unwrap();
        fs::write(stage.path().join("two"), b"de").unwrap();
        assert_eq!(store_space(stage.path()).unwrap(), 32 * 1024 * 1024 + 10);
        fs::create_dir(stage.path().join("extra")).unwrap();
        assert!(store_space(stage.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn checked_signer_survives_credential_rotation_for_one_attempt() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let source = tempfile::tempdir().unwrap();
        let keys = Keys::generate();
        let key_path = source.path().join("credential");
        fs::write(&key_path, keys.secret_key().to_bech32().unwrap()).unwrap();
        symlink(&key_path, source.path().join("keys")).unwrap();
        fs::write(
            source.path().join("config.toml"),
            "[nostr]\nrelays = ['ws://localhost:7777']\n",
        )
        .unwrap();
        let tree = NamedTree {
            publisher: keys.public_key(),
            binary: PathBuf::from("/unused"),
            receipt_path: None,
            blossom: "http://localhost:8080".into(),
            config_dir: source.path().to_owned(),
            data_dir: source.path().to_owned(),
        };
        let pinned = tree.private_config().unwrap();
        fs::remove_file(key_path).unwrap();
        assert!(tree.private_config().is_err());
        assert!(!source.path().join("keys").exists());
        let pinned_keys = pinned.path().join("keys");
        assert_eq!(
            fs::metadata(pinned.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&pinned_keys).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(signer_token(&read_setting(&pinned_keys).unwrap(), keys.public_key()).is_ok());
        let config: toml::Table = read_setting(&pinned.path().join("config.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            config["blossom"]["servers"][0].as_str(),
            Some(tree.blossom.as_str())
        );
        drop(pinned);
        assert!(!pinned_keys.exists());
    }

    #[test]
    fn require_the_first_existing_signer() {
        let first = Keys::generate();
        let second = Keys::generate();
        let nsec = first.secret_key().to_bech32().unwrap();
        let source = format!(
            "\n # ignored\n{nsec} server\n{}",
            second.secret_key().to_bech32().unwrap()
        );
        assert_eq!(signer_token(&source, first.public_key()).unwrap(), nsec);
        assert!(signer_token(&source, second.public_key()).is_err());
        assert!(signer_token("# empty\n", first.public_key()).is_err());
    }

    #[test]
    fn require_one_hash_and_legacy_chk_key() {
        let hash = "ab".repeat(32);
        let key = "12".repeat(32);
        let good = format!("added files\n  hash:  {hash}\n  key:   {key}\n");
        assert_eq!(
            public_cid(good.as_bytes()).unwrap(),
            (hash.as_str(), key.as_str())
        );
        for bad in [
            format!("hash: {hash}"),
            format!("{good}key: {key}"),
            format!("hash: {hash}\nkey: 01{key}"),
            format!("hash: {hash}\nkey: invalid"),
        ] {
            assert!(public_cid(bad.as_bytes()).is_err());
        }
    }
}
