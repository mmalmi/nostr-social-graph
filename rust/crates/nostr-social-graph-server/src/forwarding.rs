//! Optional bounded forwarding into a replaceable-event relay. No second event DB.
mod diagnostics;
use crate::{GraphFreshness, Result, ServerError};
use diagnostics::{Diagnostics, Outcome};
use nostr_sdk::{Client, Event, Keys};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
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
    allowlist_dirty: Arc<AtomicBool>,
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
            let mut diagnostics = Diagnostics::default();
            let mut report_tick = tokio::time::interval(Duration::from_secs(30));
            report_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
                        _ = report_tick.tick() => {
                            report_diagnostics(&mut diagnostics);
                            continue;
                        },
                    }
                };
                match message {
                    Message::Barrier(done) => {
                        report_diagnostics(&mut diagnostics);
                        let _ = done.send(());
                    }
                    Message::Event(event) => {
                        // A failed relay must not stall crawling for five seconds
                        // per event. Record the missed delivery for full repair.
                        if retry_after.is_some_and(|until| Instant::now() < until) {
                            state.write().expect("freshness lock").forwarding_failures += 1;
                            diagnostics.record(Outcome::CooldownSkip);
                            report_diagnostics(&mut diagnostics);
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
                            // The debit is only a conservative estimate. Replaceable
                            // heads often reuse LMDB pages: measure again before
                            // declaring an actual capacity failure and skipping data.
                            disk_check = Some((
                                Instant::now(),
                                free_bytes(&path).unwrap_or(0).saturating_sub(reserve),
                            ));
                        }
                        if !disk_check.is_some_and(|(_, available)| available >= cost) {
                            let mut status = state.write().expect("freshness lock");
                            status.forwarding_failures += 1;
                            status.forwarding_last_failure = Some("disk reserve".into());
                            drop(status);
                            diagnostics.record(Outcome::DiskReserve);
                            report_diagnostics(&mut diagnostics);
                            continue;
                        }
                        if let Some((_, available)) = disk_check.as_mut() {
                            *available = available.saturating_sub(cost);
                        }
                        let outcome = match tokio::time::timeout(
                            Duration::from_secs(5),
                            client.send_event(&event),
                        )
                        .await
                        {
                            Ok(Ok(output)) => delivery_outcome(
                                !output.success.is_empty(),
                                output.failed.values().map(String::as_str),
                            ),
                            Ok(Err(_)) => Outcome::SendError,
                            Err(_) => Outcome::SendTimeout,
                        };
                        let mut status = state.write().expect("freshness lock");
                        if outcome.accepted() {
                            status.last_forwarded_at = Some(crate::unix_timestamp());
                        } else {
                            status.forwarding_failures += 1;
                            status.forwarding_last_failure = Some(outcome.label().into());
                            if outcome.starts_cooldown() {
                                retry_after = Some(Instant::now() + Duration::from_secs(30));
                            }
                        }
                        drop(status);
                        diagnostics.record(outcome);
                        report_diagnostics(&mut diagnostics);
                    }
                }
            }
        });
        Ok(Some(Self {
            sender,
            freshness,
            pending,
            wake,
            allowlist_dirty: Arc::new(AtomicBool::new(false)),
        }))
    }

    pub fn mark_allowlist_dirty(&self) {
        self.allowlist_dirty.store(true, Ordering::Release);
    }

    // Called only by the existing sync-loop owner, alongside its crawl and
    // checkpoint writes. Live ingestion requests a refresh; it never writes it.
    pub fn refresh_allowlist_if_dirty(&self, graph: &nostr_social_graph::SocialGraph) {
        if let Err(error) =
            refresh_dirty_allowlist(&self.allowlist_dirty, || refresh_allowlist(graph))
        {
            let mut status = self.freshness.write().expect("freshness lock");
            status.forwarding_failures = status.forwarding_failures.saturating_add(1);
            status.forwarding_last_failure = Some("allowlist persistence".into());
            tracing::warn!("live allowlist refresh remains pending: {error}");
        }
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
                    self.freshness
                        .write()
                        .expect("freshness lock")
                        .forwarding_last_failure = Some("live backlog overflow".into());
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

fn refresh_dirty_allowlist(dirty: &AtomicBool, write: impl FnOnce() -> Result<()>) -> Result<()> {
    if dirty.swap(false, Ordering::AcqRel) {
        if let Err(error) = write() {
            dirty.store(true, Ordering::Release);
            return Err(error);
        }
    }
    Ok(())
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

// The SDK flattens relay errors to strings. Preserve the existing delivery and
// cooldown decisions; only count recognizable NIP-20 reason prefixes as reported
// rejections. Never print a remote reason, URL, event ID, author or event body.
fn delivery_outcome<'a>(accepted: bool, failures: impl Iterator<Item = &'a str>) -> Outcome {
    let failures: Vec<_> = failures.collect();
    let (accepted, transport) = classify_delivery(accepted, failures.iter().copied());
    if accepted {
        Outcome::Accepted
    } else if transport {
        Outcome::TransportFailure
    } else if failures.iter().all(|failure| {
        let reason = failure
            .strip_prefix("event not published: ")
            .unwrap_or(failure);
        [
            "blocked:",
            "invalid:",
            "pow:",
            "duplicate:",
            "rate-limited:",
            "restricted:",
        ]
        .iter()
        .any(|prefix| reason.starts_with(prefix))
    }) {
        Outcome::ReportedRejection
    } else {
        Outcome::OtherDeliveryFailure
    }
}

fn report_diagnostics(diagnostics: &mut Diagnostics) {
    if let Some(counts) = diagnostics.take_report(Instant::now()) {
        tracing::warn!(
            send_attempts = counts.send_attempts,
            accepted = counts.accepted,
            send_timeouts = counts.send_timeouts,
            send_errors = counts.send_errors,
            transport_failures = counts.transport_failures,
            reported_rejections = counts.reported_rejections,
            other_delivery_failures = counts.other_delivery_failures,
            cooldown_skips = counts.cooldown_skips,
            disk_reserve_skips = counts.disk_reserve_skips,
            "graph forwarding cumulative diagnostics; skipped heads are not send attempts"
        );
    }
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
    fn allowlist_updates_coalesce_and_failed_writes_remain_pending() {
        let dirty = AtomicBool::new(false);
        let writes = std::cell::Cell::new(0);
        let write = || {
            writes.set(writes.get() + 1);
            Ok(())
        };
        refresh_dirty_allowlist(&dirty, write).unwrap();
        assert_eq!(writes.get(), 0);
        for _ in 0..100 {
            dirty.store(true, Ordering::Release);
        }
        refresh_dirty_allowlist(&dirty, write).unwrap();
        refresh_dirty_allowlist(&dirty, write).unwrap();
        assert_eq!(writes.get(), 1);
        dirty.store(true, Ordering::Release);
        assert!(
            refresh_dirty_allowlist(&dirty, || Err(std::io::Error::other("fixture").into()))
                .is_err()
        );
        assert!(dirty.load(Ordering::Acquire));
        refresh_dirty_allowlist(&dirty, || {
            // A notification arriving while the write runs must not be erased.
            dirty.store(true, Ordering::Release);
            Ok(())
        })
        .unwrap();
        assert!(dirty.load(Ordering::Acquire));
        refresh_dirty_allowlist(&dirty, write).unwrap();
        assert!(!dirty.load(Ordering::Acquire));
        assert_eq!(writes.get(), 2);
    }

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
    fn delivery_diagnostics_do_not_guess_unknown_errors_or_change_admission() {
        assert_eq!(
            delivery_outcome(false, ["blocked: author outside graph"].into_iter()),
            Outcome::ReportedRejection
        );
        assert_eq!(
            delivery_outcome(false, ["unexpected SDK failure"].into_iter()),
            Outcome::OtherDeliveryFailure
        );
        assert_eq!(
            delivery_outcome(false, ["relay not connected"].into_iter()),
            Outcome::TransportFailure
        );
        assert_eq!(
            delivery_outcome(false, ["replaced: have newer event"].into_iter()),
            Outcome::Accepted
        );
        assert_eq!(
            delivery_outcome(true, ["blocked: other relay"].into_iter()),
            Outcome::Accepted
        );
        assert_eq!(
            delivery_outcome(false, [].into_iter()),
            Outcome::TransportFailure
        );
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
