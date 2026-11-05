//! Optional bounded forwarding into a replaceable-event relay. No second event DB.
use crate::{GraphFreshness, Result, ServerError};
use nostr_sdk::{Client, Event, Keys};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub(crate) struct Forwarder {
    sender: mpsc::Sender<Message>,
    freshness: Arc<RwLock<GraphFreshness>>,
}
enum Message {
    Event(Box<Event>),
    Barrier(oneshot::Sender<()>),
}

impl Forwarder {
    pub async fn from_env(
        data_dir: &Path,
        freshness: Arc<RwLock<GraphFreshness>>,
    ) -> Result<Option<Self>> {
        let Ok(url) = std::env::var("GRAPH_FORWARD_RELAY_URL") else {
            return Ok(None);
        };
        if url.trim().is_empty() {
            return Ok(None);
        }
        let reserve = std::env::var("GRAPH_FORWARD_MIN_FREE_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5 * 1024 * 1024 * 1024);
        let client = Client::new(Keys::generate());
        client
            .add_relay(url)
            .await
            .map_err(|e| ServerError::Io(std::io::Error::other(e.to_string())))?;
        client.connect().await;
        let (sender, mut receiver) = mpsc::channel(256);
        let state = Arc::clone(&freshness);
        let path = data_dir.to_path_buf();
        freshness
            .write()
            .expect("freshness lock")
            .forwarding_enabled = true;
        tokio::spawn(async move {
            let mut disk_check: Option<(Instant, u64)> = None;
            while let Some(message) = receiver.recv().await {
                match message {
                    Message::Barrier(done) => {
                        let _ = done.send(());
                    }
                    Message::Event(event) => {
                        if disk_check
                            .is_none_or(|(checked, _)| checked.elapsed() >= Duration::from_secs(30))
                        {
                            disk_check = Some((
                                Instant::now(),
                                free_bytes(&path).unwrap_or(0).saturating_sub(reserve),
                            ));
                        }
                        // Budget conservatively for LMDB page/index growth between
                        // disk probes; never enqueue an unbounded burst into reserve.
                        let cost = serde_json::to_vec(&*event)
                            .map(|bytes| bytes.len() as u64 * 4 + 4096)
                            .unwrap_or(u64::MAX);
                        if !disk_check.is_some_and(|(_, available)| available >= cost) {
                            state.write().expect("freshness lock").forwarding_failures += 1;
                            continue;
                        }
                        if let Some((_, available)) = disk_check.as_mut() {
                            *available = available.saturating_sub(cost);
                        }
                        let accepted = matches!(tokio::time::timeout(Duration::from_secs(5), client.send_event(&event)).await,
                            Ok(Ok(output)) if !output.success.is_empty());
                        let mut status = state.write().expect("freshness lock");
                        if accepted {
                            status.last_forwarded_at = Some(crate::unix_timestamp());
                        } else {
                            status.forwarding_failures += 1;
                        }
                    }
                }
            }
        });
        Ok(Some(Self { sender, freshness }))
    }

    pub async fn enqueue(&self, event: &Event) {
        if forwardable(event)
            && self
                .sender
                .send(Message::Event(Box::new(event.clone())))
                .await
                .is_err()
        {
            self.freshness
                .write()
                .expect("freshness lock")
                .forwarding_failures += 1;
        }
    }

    pub fn try_enqueue(&self, event: &Event) {
        if forwardable(event)
            && self
                .sender
                .try_send(Message::Event(Box::new(event.clone())))
                .is_err()
        {
            self.freshness
                .write()
                .expect("freshness lock")
                .forwarding_failures += 1;
        }
    }

    pub async fn drain(&self) {
        let (send, receive) = oneshot::channel();
        if self.sender.send(Message::Barrier(send)).await.is_err() || receive.await.is_err() {
            self.freshness
                .write()
                .expect("freshness lock")
                .forwarding_failures += 1;
        }
    }
}

fn forwardable(event: &Event) -> bool {
    matches!(event.kind.as_u16(), 3 | 10000)
        && event.verify().is_ok()
        && !event.is_protected()
        && event.tags.len() <= 25_000
        && serde_json::to_vec(event).is_ok_and(|bytes| bytes.len() <= 524_288)
}

// POSIX df is present in supported server images. Fail closed if its output is
// unavailable; forwarding must never consume the configured filesystem reserve.
fn free_bytes(path: &Path) -> Option<u64> {
    let output = std::process::Command::new("df")
        .args(["-Pk"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_free_bytes(&String::from_utf8(output.stdout).ok()?)
}
fn parse_free_bytes(output: &str) -> Option<u64> {
    output
        .lines()
        .last()?
        .split_whitespace()
        .nth(3)?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

pub(crate) fn refresh_allowlist(graph: &nostr_social_graph::SocialGraph) -> Result<()> {
    let Some(path) = std::env::var_os("GRAPH_FORWARD_ALLOWLIST_PATH").map(PathBuf::from) else {
        return Ok(());
    };
    let temporary = path.with_extension("tmp");
    let mut contents = graph.users_in_distance_order(None).join("\n");
    contents.push('\n');
    std::fs::write(&temporary, contents)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_reserve_parser_fails_closed() {
        assert_eq!(
            parse_free_bytes(
                "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda 100 50 40 55% /data\n"
            ),
            Some(40 * 1024)
        );
        assert_eq!(parse_free_bytes("invalid"), None);
        assert_eq!(parse_free_bytes("/dev/sda 100 50 nope 55% /data"), None);
    }
}
