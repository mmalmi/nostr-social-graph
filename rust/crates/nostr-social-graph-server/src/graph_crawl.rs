//! Schedule graph requests within a single distance. Only a full sweep can
//! combine new and known authors into the same filter.

use super::{
    Forwarder, GraphFreshness, NostrClient, Result, SocialGraph, apply_graph_events,
    fetch_graph_batch, fetch_graph_events, graph_filter, parse_public_keys, unix_timestamp,
};
use nostr_sdk::Event;
use std::sync::{Arc, RwLock};
use tokio::task::JoinHandle;

const FULL_REQUEST_WINDOW: usize = 4;

pub(super) async fn fetch_distance(
    client: &NostrClient,
    authors: &[String],
    batch_size: usize,
    since: Option<u64>,
    graph: &Arc<RwLock<SocialGraph>>,
    freshness: &Arc<RwLock<GraphFreshness>>,
    forwarder: Option<&Forwarder>,
) -> Result<()> {
    if let Some(since) = since {
        for chunk in authors.chunks(batch_size) {
            mark_progress(freshness);
            // Newly reachable authors need unrestricted history even during an
            // incremental sweep. A stored graph ID is not a follow-list head.
            let (new, known): (Vec<_>, Vec<_>) = {
                let guard = graph.read().expect("graph lock");
                chunk
                    .iter()
                    .cloned()
                    .partition(|author| guard.get_follow_list_created_at(author).is_none())
            };
            if !new.is_empty() {
                fetch_graph_events(client, &new, None, graph, forwarder).await?;
            }
            if !known.is_empty() {
                fetch_graph_events(client, &known, Some(since), graph, forwarder).await?;
            }
        }
        return Ok(());
    }

    for window in authors.chunks(batch_size * FULL_REQUEST_WINDOW) {
        mark_progress(freshness);
        let mut requests = Vec::with_capacity(FULL_REQUEST_WINDOW);
        for chunk in window.chunks(batch_size) {
            let keys = parse_public_keys(chunk);
            let client = client.clone();
            requests.push(tokio::spawn(async move {
                if keys.is_empty() {
                    return Ok(Vec::new());
                }
                fetch_graph_batch(&client, graph_filter(keys, None)).await
            }));
        }

        // Finish and unsubscribe every started request even when a sibling
        // fails. Apply only the successful prefix, in original chunk order.
        for (batch, chunk) in collect_in_order(requests)
            .await
            .into_iter()
            .zip(window.chunks(batch_size))
        {
            let mut events = batch?;
            mark_progress(freshness);
            unknown_authors_first(&mut events, &graph.read().expect("graph lock"));
            apply_graph_events(events, chunk.len(), graph, forwarder).await;
        }
    }
    Ok(())
}

async fn collect_in_order<T>(requests: Vec<JoinHandle<Result<T>>>) -> Vec<Result<T>> {
    let mut batches = Vec::with_capacity(requests.len());
    for request in requests {
        let result = match request.await {
            Ok(batch) => batch,
            Err(error) => Err(std::io::Error::other(error).into()),
        };
        batches.push(result);
    }
    batches
}

// The old pair of fetches applied new authors before known authors, with each
// group in (author, kind) order. Preserve that order for admission decisions.
fn unknown_authors_first(events: &mut [Event], graph: &SocialGraph) {
    events.sort_by_cached_key(|event| {
        graph
            .get_follow_list_created_at(&event.pubkey.to_hex())
            .is_some()
    });
}

fn mark_progress(freshness: &RwLock<GraphFreshness>) {
    freshness
        .write()
        .expect("freshness lock")
        .last_graph_progress_at = Some(unix_timestamp());
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Kind};
    use tokio::sync::oneshot;

    #[test]
    fn merged_full_batch_keeps_new_authors_first_and_both_kinds_in_order() {
        let known = Keys::generate();
        let new = Keys::generate();
        let list = |keys: &Keys, kind: u16| {
            EventBuilder::new(Kind::from(kind), "")
                .sign_with_keys(keys)
                .unwrap()
        };
        let known_follow = list(&known, 3);
        let known_mute = list(&known, 10_000);
        let new_follow = list(&new, 3);
        let new_mute = list(&new, 10_000);
        let mut graph = SocialGraph::new(&known.public_key().to_hex());
        graph.handle_event(
            &crate::sdk_event_to_social_graph_event(&known_follow).unwrap(),
            true,
            1.0,
        );
        let mut batch = vec![
            known_follow.clone(),
            known_mute.clone(),
            new_follow.clone(),
            new_mute.clone(),
        ];
        unknown_authors_first(&mut batch, &graph);
        assert_eq!(batch, [new_follow, new_mute, known_follow, known_mute]);
    }

    #[tokio::test]
    async fn waits_for_every_started_request_and_keeps_error_in_chunk_order() {
        let (send, wait) = oneshot::channel();
        let handles = vec![
            tokio::spawn(async move {
                wait.await.unwrap();
                Ok(1u8)
            }),
            tokio::spawn(async { Err(std::io::Error::other("batch failed").into()) }),
            tokio::spawn(async move {
                send.send(()).unwrap();
                Ok(3u8)
            }),
        ];
        let [first, failed, last]: [_; 3] = collect_in_order(handles).await.try_into().unwrap();
        assert_eq!(first.unwrap(), 1);
        assert!(failed.is_err());
        assert_eq!(last.unwrap(), 3);
    }
}
