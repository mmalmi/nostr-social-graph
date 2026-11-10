mod support;

use std::collections::{BTreeMap, BTreeSet};

use nostr_social_graph::SocialGraph;
use support::event;

fn key(id: u64) -> String {
    format!("{id:064x}")
}

fn list(graph: &mut SocialGraph, author: u64, follows: &[u64], created_at: u64) {
    let follows: Vec<String> = follows.iter().copied().map(key).collect();
    let tagged = follows.iter().map(String::as_str).collect();
    graph.handle_event(&event(&key(author), 3, created_at, tagged), true, 1.0);
}

fn distances(graph: &SocialGraph, users: &[u64]) -> Vec<u32> {
    users
        .iter()
        .map(|user| graph.get_follow_distance(&key(*user)))
        .collect()
}

fn bucket(graph: &SocialGraph, distance: u32) -> BTreeSet<String> {
    graph
        .get_users_by_follow_distance(distance)
        .into_iter()
        .collect()
}

/// Distances and buckets as a from-scratch breadth-first search sees them.
fn snapshot(graph: &SocialGraph, users: u64) -> (Vec<u32>, BTreeMap<u32, BTreeSet<String>>, usize) {
    let all: Vec<u64> = (0..users).collect();
    let buckets = (0..users as u32)
        .map(|distance| (distance, bucket(graph, distance)))
        .filter(|(_, users)| !users.is_empty())
        .collect();
    (distances(graph, &all), buckets, graph.size().users)
}

fn assert_exact(graph: &mut SocialGraph, users: u64) {
    let live = snapshot(graph, users);
    graph.recalculate_follow_distances();
    assert_eq!(live, snapshot(graph, users));
}

#[test]
fn reaches_the_follows_of_a_newly_followed_user_whose_list_was_already_known() {
    let mut graph = SocialGraph::new(&key(0));
    list(&mut graph, 1, &[2, 3], 10);
    list(&mut graph, 2, &[4], 10);
    assert_eq!(graph.get_follow_distance(&key(4)), 1000);

    list(&mut graph, 0, &[1], 10);

    assert_eq!(distances(&graph, &[1, 2, 3, 4]), [1, 2, 2, 3]);
    assert_eq!(bucket(&graph, 2), BTreeSet::from([key(2), key(3)]));
}

#[test]
fn moves_an_unfollowed_user_and_everyone_reached_through_them() {
    let mut graph = SocialGraph::new(&key(0));
    list(&mut graph, 0, &[1, 5], 10);
    for (author, followed) in [(1, 2), (2, 3), (5, 6), (6, 7), (7, 3)] {
        list(&mut graph, author, &[followed], 10);
    }
    assert_eq!(graph.get_follow_distance(&key(3)), 3);

    list(&mut graph, 0, &[5], 20);

    assert_eq!(distances(&graph, &[1, 2, 3]), [1000, 1000, 4]);
    assert_eq!(bucket(&graph, 1), BTreeSet::from([key(5)]));
    assert_eq!(bucket(&graph, 4), BTreeSet::from([key(3)]));
    assert_eq!(graph.size().users, 5);
}

#[test]
fn keeps_a_distance_that_another_equally_short_path_still_supports() {
    let mut graph = SocialGraph::new(&key(0));
    list(&mut graph, 0, &[1, 2], 10);
    list(&mut graph, 1, &[3], 10);
    list(&mut graph, 2, &[3], 10);
    list(&mut graph, 3, &[4], 10);

    list(&mut graph, 1, &[], 20);

    assert_eq!(distances(&graph, &[3, 4]), [2, 3]);
}

#[test]
fn matches_a_full_recalculation_after_random_follows_and_unfollows() {
    let mut seed = 7u64;
    let mut random = |bound: u64| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) % bound
    };
    for (users, steps) in [(8, 400), (40, 1500), (150, 2500)] {
        let mut graph = SocialGraph::new(&key(0));
        for step in 1..=steps {
            let author = random(users);
            if random(10) < 4 {
                graph
                    .add_positive_relation(&key(author), &key(random(users)), step)
                    .unwrap();
            } else {
                // A replacement list both follows and unfollows.
                let follows: Vec<u64> = (0..random(6)).map(|_| random(users)).collect();
                list(&mut graph, author, &follows, step);
            }
            if step % 50 == 0 {
                assert_exact(&mut graph, users);
            }
        }
        assert_exact(&mut graph, users);
    }
}
