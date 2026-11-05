//! Optional bounded forwarding into a replaceable-event relay. No second event DB.
use crate::{GraphFreshness, Result, ServerError};
use nostr_sdk::{Client, Event, Keys};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc, oneshot};
type PendingHeads = BTreeMap<(String, u16), Box<Event>>;

#[derive(Clone)]
pub(crate) struct Forwarder {
    sender: mpsc::Sender<Message>,
    freshness: Arc<RwLock<GraphFreshness>>,
    pending: Arc<Mutex<PendingHeads>>,
    wake: Arc<Notify>,
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
        client.wait_for_connection(Duration::from_secs(5)).await;
        let (sender, mut receiver) = mpsc::channel(256);
        let pending = Arc::new(Mutex::new(PendingHeads::new()));
        let pending_worker = Arc::clone(&pending);
        let wake = Arc::new(Notify::new());
        let wake_worker = Arc::clone(&wake);
        let state = Arc::clone(&freshness);
        let path = data_dir.to_path_buf();
        freshness
            .write()
            .expect("freshness lock")
            .forwarding_enabled = true;
        tokio::spawn(async move {
            let mut disk_check: Option<(Instant, u64)> = None;
            let mut retry_after: Option<Instant> = None;
            loop {
                let pending_event = pending_worker
                    .lock()
                    .expect("pending heads lock")
                    .pop_first()
                    .map(|(_, event)| event);
                let message = if let Some(event) = pending_event {
                    Message::Event(event)
                } else {
                    tokio::select! {
                        message = receiver.recv() => match message { Some(message) => message, None => break },
                        _ = wake_worker.notified() => continue,
                    }
                };
                match message {
                    Message::Barrier(done) => {
                        let _ = done.send(());
                    }
                    Message::Event(event) => {
                        // A failed relay must not stall crawling for five seconds
                        // per event. Record the missed delivery for full repair.
                        if retry_after.is_some_and(|until| Instant::now() < until) {
                            state.write().expect("freshness lock").forwarding_failures += 1;
                            continue;
                        }
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
                        let (accepted, transport_failure) = match tokio::time::timeout(
                            Duration::from_secs(5),
                            client.send_event(&event),
                        )
                        .await
                        {
                            Ok(Ok(output)) => classify_delivery(
                                !output.success.is_empty(),
                                output.failed.values().map(String::as_str),
                            ),
                            _ => (false, true),
                        };
                        let mut status = state.write().expect("freshness lock");
                        if accepted {
                            status.last_forwarded_at = Some(crate::unix_timestamp());
                        } else {
                            status.forwarding_failures += 1;
                            if transport_failure {
                                retry_after = Some(Instant::now() + Duration::from_secs(30));
                            }
                        }
                    }
                }
            }
        });
        Ok(Some(Self {
            sender,
            freshness,
            pending,
            wake,
        }))
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
        if !forwardable(event) {
            return;
        }
        match self
            .sender
            .try_send(Message::Event(Box::new(event.clone())))
        {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(Message::Event(event))) => {
                if coalesce_head(&mut self.pending.lock().expect("pending heads lock"), event) {
                    self.wake.notify_one();
                } else {
                    self.freshness
                        .write()
                        .expect("freshness lock")
                        .forwarding_failures += 1;
                    tracing::warn!("graph forwarding overflow exceeded bounded pending heads");
                }
            }
            Err(_) => {
                self.freshness
                    .write()
                    .expect("freshness lock")
                    .forwarding_failures += 1;
            }
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

// Coalesce latest live heads while the crawl uses channel backpressure. Keep
// ingestion nonblocking even across a checkpoint pause, without unbounded tasks.
fn coalesce_head(pending: &mut PendingHeads, event: Box<Event>) -> bool {
    let key = (event.pubkey.to_hex(), event.kind.as_u16());
    if let Some(previous) = pending.get(&key) {
        if event.created_at < previous.created_at
            || (event.created_at == previous.created_at && event.id >= previous.id)
        {
            return true;
        }
    } else if pending.len() >= 256 {
        return false;
    }
    pending.insert(key, event);
    true
}

// A relay that already holds a newer replaceable head is synchronized. A
// per-event policy rejection must not pause delivery for unrelated authors.
fn classify_delivery<'a>(accepted: bool, failures: impl Iterator<Item = &'a str>) -> (bool, bool) {
    if accepted {
        return (true, false);
    }
    let failures: Vec<_> = failures.collect();
    if !failures.is_empty()
        && failures
            .iter()
            .all(|failure| failure.contains("replaced: have newer event"))
    {
        return (true, false);
    }
    let transport_failure = failures.is_empty()
        || failures.iter().any(|failure| {
            failure.contains("not connected")
                || failure.contains("timeout")
                || failure.contains("Timeout")
        });
    (false, transport_failure)
}

fn forwardable(event: &Event) -> bool {
    matches!(event.kind.as_u16(), 3 | 10000)
        && event.verify().is_ok()
        && crate::relay_mirror::local_relay_reject_reason(event).is_none()
}

// POSIX df is present in supported server images. Fail closed if its output is
// unavailable; forwarding must never consume the configured filesystem reserve.
pub(crate) fn free_bytes(path: &Path) -> Option<u64> {
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
    fn live_overflow_coalesces_latest_heads_with_a_fixed_memory_bound() {
        use nostr_sdk::{EventBuilder, Kind, Timestamp};
        let keys = Keys::generate();
        let old = EventBuilder::new(Kind::from(3), "")
            .custom_created_at(Timestamp::from(10))
            .sign_with_keys(&keys)
            .unwrap();
        let new = EventBuilder::new(Kind::from(3), "")
            .custom_created_at(Timestamp::from(20))
            .sign_with_keys(&keys)
            .unwrap();
        let mut pending = PendingHeads::new();
        assert!(coalesce_head(&mut pending, Box::new(new.clone())));
        assert!(coalesce_head(&mut pending, Box::new(old)));
        assert_eq!(pending.values().next().unwrap().id, new.id);
        for _ in 1..256 {
            let event = EventBuilder::new(Kind::from(3), "")
                .sign_with_keys(&Keys::generate())
                .unwrap();
            assert!(coalesce_head(&mut pending, Box::new(event)));
        }
        let overflow = EventBuilder::new(Kind::from(3), "")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(!coalesce_head(&mut pending, Box::new(overflow)));
        assert_eq!(pending.len(), 256);
        assert!(coalesce_head(&mut pending, Box::new(new)));
    }

    #[test]
    fn newer_heads_and_individual_rejections_do_not_pause_other_authors() {
        assert_eq!(
            classify_delivery(
                false,
                ["event not published: replaced: have newer event"].into_iter()
            ),
            (true, false)
        );
        assert_eq!(
            classify_delivery(
                false,
                ["event not published: blocked: author outside graph"].into_iter()
            ),
            (false, false)
        );
        assert_eq!(
            classify_delivery(false, ["relay not connected"].into_iter()),
            (false, true)
        );
        assert_eq!(classify_delivery(true, [].into_iter()), (true, false));
    }

    #[test]
    fn forwarding_only_admits_verified_public_graph_lists() {
        use nostr_sdk::{EventBuilder, Kind, Tag};
        let keys = Keys::generate();
        let follow = EventBuilder::new(Kind::from(3), "")
            .sign_with_keys(&keys)
            .unwrap();
        let mute = EventBuilder::new(Kind::from(10000), "")
            .sign_with_keys(&keys)
            .unwrap();
        let note = EventBuilder::new(Kind::from(1), "")
            .sign_with_keys(&keys)
            .unwrap();
        let protected = EventBuilder::new(Kind::from(3), "")
            .tags([Tag::parse(["-"]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let mut invalid = follow.clone();
        invalid.content = "changed after signing".to_string();
        assert!(forwardable(&follow));
        assert!(forwardable(&mute));
        assert!(!forwardable(&note));
        assert!(!forwardable(&protected));
        assert!(!forwardable(&invalid));
    }

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
