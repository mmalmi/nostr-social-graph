use std::sync::{Arc, RwLock};

use nostr::{EventBuilder, Keys, Kind};
use nostr_pubsub::{
    EventBus, EventSource, Filter, InMemoryEventBus, PolicyDecision, PubsubPolicy, QueryOptions,
    SourceCandidate, SourceHealth, SourcePolicyContext, VerifiedEvent,
};
use nostr_social_graph::{NostrEvent, SocialGraph};
use nostr_social_graph_pubsub::{GraphDistanceAction, SocialGraphPolicy, SocialGraphPolicyConfig};

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

struct Fixture {
    graph: Arc<RwLock<SocialGraph>>,
    friend: Keys,
    unknown: Keys,
    overmuted: Keys,
}

fn fixture() -> Fixture {
    let root = Keys::generate();
    let friend = Keys::generate();
    let friend_of_friend = Keys::generate();
    let muter = Keys::generate();
    let unknown = Keys::generate();
    let overmuted = Keys::generate();

    let root_pk = root.public_key().to_hex();
    let friend_pk = friend.public_key().to_hex();
    let friend_of_friend_pk = friend_of_friend.public_key().to_hex();
    let muter_pk = muter.public_key().to_hex();
    let overmuted_pk = overmuted.public_key().to_hex();

    let mut graph = SocialGraph::new(&root_pk);
    graph.handle_event(
        &follow_event(&root_pk, 1_000, vec![&friend_pk, &muter_pk]),
        true,
        1.0,
    );
    graph.handle_event(
        &follow_event(&friend_pk, 1_100, vec![&friend_of_friend_pk]),
        true,
        1.0,
    );
    graph.handle_event(
        &mute_event(&friend_pk, 1_200, vec![&overmuted_pk]),
        true,
        1.0,
    );
    graph.handle_event(
        &mute_event(&muter_pk, 1_201, vec![&overmuted_pk]),
        true,
        1.0,
    );

    Fixture {
        graph: Arc::new(RwLock::new(graph)),
        friend,
        unknown,
        overmuted,
    }
}

fn signed_text_note(keys: &Keys, content: &str) -> VerifiedEvent {
    let event = EventBuilder::new(Kind::TextNote, content, [])
        .to_event(keys)
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
