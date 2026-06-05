use std::sync::{Arc, RwLock};

use nostr::{EventBuilder, Keys, Kind, ToBech32};
use nostr_pubsub::{
    EventBus, EventSource, EventSourceKind, Filter, InMemoryEventBus, PolicyDecision, PubsubPolicy,
    QueryOptions, RouteQuerySource, RoutedQueryOptions, SourceCandidate, SourceHealth, SourceId,
    SourcePolicyContext, SourceRoute, VerifiedEvent, query_routes_with_policy,
};
use nostr_social_graph::{NostrEvent, SocialGraph, SocialGraphBackend};
use nostr_social_graph_hashtree::HashtreeSocialGraph;
use nostr_social_graph_pubsub::{GraphDistanceAction, SocialGraphPolicy, SocialGraphPolicyConfig};
use tempfile::TempDir;

#[tokio::test]
async fn bus_prioritizes_graph_authors_and_throttles_unknown_authors() {
    let Fixture {
        graph,
        friend,
        unknown,
        ..
    } = fixture();
    let bus = InMemoryEventBus::with_policy(Arc::new(SocialGraphPolicy::new(
        graph,
        SocialGraphPolicyConfig::default(),
    )));

    let trusted = bus
        .publish(
            signed_text_note(&friend, "trusted"),
            EventSource::peer("peer"),
        )
        .await
        .unwrap();
    let unknown = bus
        .publish(
            signed_text_note(&unknown, "unknown"),
            EventSource::peer("peer"),
        )
        .await
        .unwrap();

    assert!(trusted.accepted);
    assert!(unknown.accepted);
    assert!(trusted.priority > unknown.priority);
    assert_eq!(trusted.reason, None);
    assert_eq!(
        unknown.reason.as_deref(),
        Some("author outside social graph")
    );
}

#[tokio::test]
async fn bus_can_drop_authors_outside_the_social_graph() {
    let Fixture { graph, unknown, .. } = fixture();
    let config = SocialGraphPolicyConfig {
        outside_graph_action: GraphDistanceAction::Drop,
        ..SocialGraphPolicyConfig::default()
    };
    let bus = InMemoryEventBus::with_policy(Arc::new(SocialGraphPolicy::new(graph, config)));

    let report = bus
        .publish(
            signed_text_note(&unknown, "drop"),
            EventSource::peer("peer"),
        )
        .await
        .unwrap();
    assert!(!report.accepted);
    assert_eq!(
        report.reason.as_deref(),
        Some("author outside social graph")
    );

    let queried = bus
        .query(vec![Filter::new()], QueryOptions::default())
        .await
        .unwrap();
    assert!(queried.events.is_empty());
}

#[tokio::test]
async fn source_policy_uses_candidate_author_pubkey() {
    let Fixture {
        graph,
        friend,
        unknown,
        ..
    } = fixture();
    let policy = SocialGraphPolicy::new(graph, SocialGraphPolicyConfig::default());
    let candidate = SourceCandidate {
        source: EventSource::peer("candidate"),
        priority: 0,
        reason: None,
        freshness_hint: None,
        health: SourceHealth::default(),
    };

    let trusted = policy
        .check_source(SourcePolicyContext {
            candidate: &candidate,
            author_pubkey: Some(&friend.public_key().to_hex()),
        })
        .await
        .unwrap();
    let unknown = policy
        .check_source(SourcePolicyContext {
            candidate: &candidate,
            author_pubkey: Some(&unknown.public_key().to_hex()),
        })
        .await
        .unwrap();

    assert!(matches!(trusted, PolicyDecision::Allow { .. }));
    assert!(matches!(unknown, PolicyDecision::Throttle { .. }));
}

#[tokio::test]
async fn source_policy_infers_fips_peer_npub_source_id_when_author_is_missing() {
    let Fixture { graph, friend, .. } = fixture();
    let policy = SocialGraphPolicy::new(graph, SocialGraphPolicyConfig::default());
    let friend_npub = friend.public_key().to_bech32().unwrap();
    let fips_candidate = SourceCandidate {
        source: EventSource {
            id: SourceId::new(friend_npub),
            kind: EventSourceKind::FipsEndpoint,
            url: None,
        },
        priority: 0,
        reason: None,
        freshness_hint: None,
        health: SourceHealth::default(),
    };
    let relay_candidate = SourceCandidate {
        source: EventSource::relay("wss://relay.example"),
        priority: 0,
        reason: None,
        freshness_hint: None,
        health: SourceHealth::default(),
    };

    let trusted = policy
        .check_source(SourcePolicyContext {
            candidate: &fips_candidate,
            author_pubkey: None,
        })
        .await
        .unwrap();
    let relay = policy
        .check_source(SourcePolicyContext {
            candidate: &relay_candidate,
            author_pubkey: None,
        })
        .await
        .unwrap();

    assert!(matches!(trusted, PolicyDecision::Allow { priority } if priority > 0));
    assert_eq!(relay, PolicyDecision::Allow { priority: 0 });
}

#[tokio::test]
async fn routed_query_can_apply_graph_policy_to_peer_source_ids_without_author_hint() {
    let Fixture {
        graph,
        friend,
        unknown,
        ..
    } = fixture();
    let friend_id = friend.public_key().to_hex();
    let unknown_id = unknown.public_key().to_hex();
    let friend_bus = InMemoryEventBus::new();
    let unknown_bus = InMemoryEventBus::new();
    let friend_event = signed_text_note(&friend, "trusted peer route");
    let unknown_event = signed_text_note(&unknown, "unknown peer route");
    friend_bus
        .publish(friend_event.clone(), EventSource::peer(&friend_id))
        .await
        .unwrap();
    unknown_bus
        .publish(unknown_event.clone(), EventSource::peer(&unknown_id))
        .await
        .unwrap();
    let routes = vec![
        RouteQuerySource::new(SourceRoute::fips_peer(unknown_id, 100), &unknown_bus),
        RouteQuerySource::new(SourceRoute::fips_peer(friend_id, 0), &friend_bus),
    ];
    let policy = SocialGraphPolicy::new(graph, SocialGraphPolicyConfig::default());

    let report = query_routes_with_policy(
        &routes,
        vec![Filter::new().kind(Kind::TextNote)],
        RoutedQueryOptions {
            query: QueryOptions { limit: Some(1) },
            ..RoutedQueryOptions::default()
        },
        None,
        &policy,
        None,
    )
    .await
    .unwrap();

    assert_eq!(report.events.len(), 1);
    assert_eq!(
        report.events[0].event.as_event().id,
        friend_event.as_event().id
    );
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(report.attempts[0].route.id, friend.public_key().to_hex());
}

#[tokio::test]
async fn overmuted_authors_are_dropped_before_distance_checks() {
    let Fixture {
        graph, overmuted, ..
    } = fixture();
    let bus = InMemoryEventBus::with_policy(Arc::new(SocialGraphPolicy::new(
        graph,
        SocialGraphPolicyConfig::default(),
    )));

    let report = bus
        .publish(
            signed_text_note(&overmuted, "overmuted"),
            EventSource::peer("peer"),
        )
        .await
        .unwrap();

    assert!(!report.accepted);
    assert_eq!(
        report.reason.as_deref(),
        Some("author overmuted by social graph")
    );
}

#[tokio::test]
async fn bus_policy_can_use_persisted_hashtree_graph_backend() {
    let HashtreeFixture {
        graph,
        friend,
        unknown,
        overmuted,
        ..
    } = hashtree_fixture();
    let bus = InMemoryEventBus::with_policy(Arc::new(SocialGraphPolicy::new(
        graph,
        SocialGraphPolicyConfig::default(),
    )));

    let trusted = bus
        .publish(
            signed_text_note(&friend, "trusted"),
            EventSource::peer("peer"),
        )
        .await
        .unwrap();
    let unknown = bus
        .publish(
            signed_text_note(&unknown, "unknown"),
            EventSource::peer("peer"),
        )
        .await
        .unwrap();
    let overmuted = bus
        .publish(
            signed_text_note(&overmuted, "overmuted"),
            EventSource::peer("peer"),
        )
        .await
        .unwrap();

    assert!(trusted.accepted);
    assert!(unknown.accepted);
    assert!(!overmuted.accepted);
    assert!(trusted.priority > unknown.priority);
    assert_eq!(
        unknown.reason.as_deref(),
        Some("author outside social graph")
    );
    assert_eq!(
        overmuted.reason.as_deref(),
        Some("author overmuted by social graph")
    );
}

struct Fixture {
    graph: Arc<RwLock<SocialGraph>>,
    friend: Keys,
    unknown: Keys,
    overmuted: Keys,
}

fn fixture() -> Fixture {
    let keys = graph_keys();
    let root_pk = keys.root.public_key().to_hex();
    let mut graph = SocialGraph::new(&root_pk);
    seed_graph(&mut graph, &keys).unwrap();

    Fixture {
        graph: Arc::new(RwLock::new(graph)),
        friend: keys.friend,
        unknown: keys.unknown,
        overmuted: keys.overmuted,
    }
}

struct HashtreeFixture {
    graph: Arc<RwLock<HashtreeSocialGraph>>,
    _tempdir: TempDir,
    friend: Keys,
    unknown: Keys,
    overmuted: Keys,
}

fn hashtree_fixture() -> HashtreeFixture {
    let keys = graph_keys();
    let root_pk = keys.root.public_key().to_hex();
    let tempdir = TempDir::new().unwrap();
    let mut graph = HashtreeSocialGraph::open(tempdir.path(), &root_pk).unwrap();
    seed_graph(&mut graph, &keys).unwrap();
    graph.flush().unwrap();
    drop(graph);

    let reopened =
        HashtreeSocialGraph::open(tempdir.path(), &keys.unknown.public_key().to_hex()).unwrap();
    assert_eq!(reopened.get_root().unwrap(), root_pk);

    HashtreeFixture {
        graph: Arc::new(RwLock::new(reopened)),
        _tempdir: tempdir,
        friend: keys.friend,
        unknown: keys.unknown,
        overmuted: keys.overmuted,
    }
}

struct GraphKeys {
    root: Keys,
    friend: Keys,
    friend_of_friend: Keys,
    muter: Keys,
    unknown: Keys,
    overmuted: Keys,
}

fn graph_keys() -> GraphKeys {
    let root = Keys::generate();
    let friend = Keys::generate();
    let friend_of_friend = Keys::generate();
    let muter = Keys::generate();
    let unknown = Keys::generate();
    let overmuted = Keys::generate();

    GraphKeys {
        root,
        friend,
        friend_of_friend,
        muter,
        unknown,
        overmuted,
    }
}

fn seed_graph<B>(graph: &mut B, keys: &GraphKeys) -> Result<(), B::Error>
where
    B: SocialGraphBackend,
{
    let root_pk = keys.root.public_key().to_hex();
    let friend_pk = keys.friend.public_key().to_hex();
    let friend_of_friend_pk = keys.friend_of_friend.public_key().to_hex();
    let muter_pk = keys.muter.public_key().to_hex();
    let overmuted_pk = keys.overmuted.public_key().to_hex();

    graph.handle_event(
        &follow_event(&root_pk, 1_000, vec![&friend_pk, &muter_pk]),
        true,
        1.0,
    )?;
    graph.handle_event(
        &follow_event(&friend_pk, 1_100, vec![&friend_of_friend_pk]),
        true,
        1.0,
    )?;
    graph.handle_event(
        &mute_event(&friend_pk, 1_200, vec![&overmuted_pk]),
        true,
        1.0,
    )?;
    graph.handle_event(
        &mute_event(&muter_pk, 1_201, vec![&overmuted_pk]),
        true,
        1.0,
    )?;
    Ok(())
}

fn signed_text_note(keys: &Keys, content: &str) -> VerifiedEvent {
    let event = EventBuilder::new(Kind::TextNote, content)
        .sign_with_keys(keys)
        .unwrap();
    VerifiedEvent::try_from(event).unwrap()
}

fn follow_event(pubkey: &str, created_at: u64, followed: Vec<&str>) -> NostrEvent {
    graph_event(pubkey, 3, created_at, followed)
}

fn mute_event(pubkey: &str, created_at: u64, muted: Vec<&str>) -> NostrEvent {
    graph_event(pubkey, 10_000, created_at, muted)
}

fn graph_event(pubkey: &str, kind: u32, created_at: u64, tagged: Vec<&str>) -> NostrEvent {
    NostrEvent {
        created_at,
        content: String::new(),
        tags: tagged
            .into_iter()
            .map(|pk| vec!["p".to_string(), pk.to_string()])
            .collect(),
        kind,
        pubkey: pubkey.to_string(),
        id: format!("{pubkey}:{kind}:{created_at}"),
        sig: "00".repeat(64),
    }
}
