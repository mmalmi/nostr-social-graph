mod support;

use nostr_social_graph::SocialGraph;
use support::*;

#[test]
fn follow_lists_are_replaceable_and_ignore_stale_updates() {
    let mut graph = SocialGraph::new(ADAM);

    graph.handle_event(&event(ADAM, 3, 2_000, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![SNOWDEN]), true, 1.0);

    assert!(graph.is_following(ADAM, FIATJAF));
    assert!(!graph.is_following(ADAM, SNOWDEN));
    assert_eq!(graph.get_follow_list_created_at(ADAM), Some(2_000));

    graph.handle_event(&event(ADAM, 3, 3_000, vec![SNOWDEN]), true, 1.0);

    assert!(!graph.is_following(ADAM, FIATJAF));
    assert!(graph.is_following(ADAM, SNOWDEN));
    assert_eq!(graph.get_follow_list_created_at(ADAM), Some(3_000));
}

#[test]
fn mute_lists_are_replaceable_and_can_unmute_everyone() {
    let mut graph = SocialGraph::new(ADAM);

    graph.handle_event(&event(ADAM, 10_000, 1_000, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(ADAM, 10_000, 900, vec![SNOWDEN]), true, 1.0);

    assert_eq!(
        sorted(graph.get_muted_by_user(ADAM)),
        vec![FIATJAF.to_string()]
    );
    assert_eq!(
        sorted(graph.get_user_muted_by(FIATJAF)),
        vec![ADAM.to_string()]
    );
    assert_eq!(graph.get_mute_list_created_at(ADAM), Some(1_000));

    graph.handle_event(&event(ADAM, 10_000, 2_000, vec![]), true, 1.0);

    assert!(graph.get_muted_by_user(ADAM).is_empty());
    assert!(graph.get_user_muted_by(FIATJAF).is_empty());
    assert_eq!(graph.get_mute_list_created_at(ADAM), Some(2_000));
}

#[test]
fn set_root_recalculates_follow_distances() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(FIATJAF, 3, 2_000, vec![SNOWDEN]), true, 1.0);

    assert_eq!(graph.get_follow_distance(ADAM), 0);
    assert_eq!(graph.get_follow_distance(FIATJAF), 1);
    assert_eq!(graph.get_follow_distance(SNOWDEN), 2);

    graph.set_root(SNOWDEN).unwrap();
    assert_eq!(graph.get_follow_distance(SNOWDEN), 0);
    assert_eq!(graph.get_follow_distance(FIATJAF), 1000);
    assert_eq!(graph.get_follow_distance(ADAM), 1000);

    graph.set_root(FIATJAF).unwrap();
    assert_eq!(graph.get_follow_distance(FIATJAF), 0);
    assert_eq!(graph.get_follow_distance(SNOWDEN), 1);
    assert_eq!(graph.get_follow_distance(ADAM), 1000);

    graph.set_root(ADAM).unwrap();
    assert_eq!(graph.get_follow_distance(ADAM), 0);
    assert_eq!(graph.get_follow_distance(FIATJAF), 1);
    assert_eq!(graph.get_follow_distance(SNOWDEN), 2);
}

#[test]
fn alternate_root_can_link_into_existing_graph_after_recalc() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(FIATJAF, 3, 2_000, vec![SNOWDEN]), true, 1.0);

    let binary = graph.to_binary().unwrap();
    let mut new_graph = SocialGraph::from_binary(SIRIUS, &binary).unwrap();

    assert!(new_graph.is_following(ADAM, FIATJAF));
    assert!(new_graph.is_following(FIATJAF, SNOWDEN));
    assert_eq!(new_graph.get_follow_distance(SIRIUS), 0);
    assert_eq!(new_graph.get_follow_distance(ADAM), 1000);
    assert_eq!(new_graph.get_follow_distance(FIATJAF), 1000);
    assert_eq!(new_graph.get_follow_distance(SNOWDEN), 1000);

    new_graph.handle_event(&event(SIRIUS, 3, 3_000, vec![ADAM]), true, 1.0);

    assert_eq!(new_graph.get_follow_distance(ADAM), 1);
    assert_eq!(new_graph.get_follow_distance(FIATJAF), 1000);

    new_graph.recalculate_follow_distances();

    assert!(new_graph.is_following(SIRIUS, ADAM));
    assert_eq!(new_graph.get_follow_distance(SIRIUS), 0);
    assert_eq!(new_graph.get_follow_distance(ADAM), 1);
    assert_eq!(new_graph.get_follow_distance(FIATJAF), 2);
    assert_eq!(new_graph.get_follow_distance(SNOWDEN), 3);
}

#[test]
fn root_mute_always_marks_user_overmuted() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(
        &event(ADAM, 3, 1_000, vec![SNOWDEN, SIRIUS, FIATJAF]),
        true,
        1.0,
    );
    graph.handle_event(&event(ADAM, 10_000, 1_001, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(SNOWDEN, 10_000, 1_002, vec![FIATJAF]), true, 1.0);

    assert!(graph.is_overmuted(FIATJAF, 2.0));
    assert!(graph.is_overmuted(FIATJAF, 1.0));
}

#[test]
fn closest_distance_opinions_determine_overmuted_state() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![FIATJAF, SNOWDEN]), true, 1.0);
    graph.handle_event(&event(FIATJAF, 3, 1_001, vec![SIRIUS]), true, 1.0);
    graph.handle_event(&event(SNOWDEN, 10_000, 1_002, vec![SIRIUS]), true, 1.0);
    graph.handle_event(&event(SIRIUS, 3, 1_003, vec![CHARLIE]), true, 1.0);
    graph.handle_event(&event(SIRIUS, 3, 1_004, vec![CHARLIE, DIANA]), true, 1.0);

    assert!(!graph.is_overmuted(SIRIUS, 1.0));
    assert!(graph.is_overmuted(SIRIUS, 2.0));
}

#[test]
fn zero_threshold_is_not_overmuted_without_root_mute() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 3, 1_000, vec![SNOWDEN]), true, 1.0);
    graph.handle_event(&event(SNOWDEN, 3, 1_001, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(SNOWDEN, 10_000, 1_002, vec![FIATJAF]), true, 1.0);

    assert!(!graph.is_overmuted(FIATJAF, 0.0));
    assert!(!graph.is_overmuted(FIATJAF, 1.0));
    assert!(graph.is_overmuted(FIATJAF, 1.1));
}

#[test]
fn unknown_authors_are_ignored_when_disallowed() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(FIATJAF, 3, 1_000, vec![SNOWDEN]), false, 1.0);

    assert!(!graph.is_following(FIATJAF, SNOWDEN));
    assert!(graph.get_followed_by_user(FIATJAF).is_empty());
    assert!(graph.get_followers_by_user(SNOWDEN).is_empty());
    assert_eq!(graph.get_follow_distance(FIATJAF), 1000);
    assert_eq!(graph.get_follow_distance(SNOWDEN), 1000);
}

#[test]
fn future_and_irrelevant_events_are_ignored() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(&event(ADAM, 1, 1_000, vec![FIATJAF]), true, 1.0);
    graph.handle_event(&event(ADAM, 3, u64::MAX, vec![FIATJAF]), true, 1.0);

    assert!(!graph.is_following(ADAM, FIATJAF));
    assert!(graph.get_followed_by_user(ADAM).is_empty());
    assert!(graph.get_followers_by_user(FIATJAF).is_empty());
    assert_eq!(graph.get_follow_list_created_at(ADAM), None);
}

#[test]
fn invalid_tags_and_self_references_are_ignored() {
    let mut graph = SocialGraph::new(ADAM);
    graph.handle_event(
        &nostr_social_graph::NostrEvent {
            created_at: 1_000,
            content: String::new(),
            tags: vec![
                vec!["p".to_string(), ADAM.to_string()],
                vec!["e".to_string(), FIATJAF.to_string()],
                vec!["p".to_string(), "short".to_string()],
                vec!["p".to_string(), FIATJAF.to_string()],
            ],
            kind: 3,
            pubkey: ADAM.to_string(),
            id: "mixed-tags".to_string(),
            sig: "00".repeat(64),
        },
        true,
        1.0,
    );

    assert_eq!(
        sorted(graph.get_followed_by_user(ADAM)),
        vec![FIATJAF.to_string()]
    );
    assert_eq!(
        sorted(graph.get_followers_by_user(FIATJAF)),
        vec![ADAM.to_string()]
    );
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}
