use super::*;
use std::time::Instant;

// Frozen pre-optimization implementation from dc81464. This intentionally slow
// oracle checks observable state/order against the independent bulk algorithm.
impl SocialGraph {
    fn legacy_prune(&mut self) -> usize {
        let mut has_followers = IndexSet::new();
        for followed_users in self.followed_by_user.values() {
            for user in followed_users {
                has_followers.insert(*user);
            }
        }

        let users_to_remove: Vec<u32> = self
            .user_muted_by
            .iter()
            .filter_map(|(user, muters)| {
                if *user != self.root && !muters.is_empty() && !has_followers.contains(user) {
                    Some(*user)
                } else {
                    None
                }
            })
            .collect();

        if users_to_remove.is_empty() {
            return 0;
        }

        for user in users_to_remove.iter().copied() {
            self.list_revisions.shift_remove(&(user, 3));
            self.list_revisions.shift_remove(&(user, 10_000));
            if let Some(distance) = self.follow_distance_by_user.shift_remove(&user)
                && let Some(bucket) = self.users_by_follow_distance.get_mut(&distance)
            {
                bucket.shift_remove(&user);
            }
            self.followed_by_user.shift_remove(&user);
            self.followers_by_user.shift_remove(&user);
            self.follow_list_created_at.shift_remove(&user);
            self.muted_by_user.shift_remove(&user);
            self.user_muted_by.shift_remove(&user);
            self.mute_list_created_at.shift_remove(&user);
            if let Some(value) = self.ids.unique_id_to_str.shift_remove(&user) {
                self.ids.str_to_unique_id.shift_remove(&value);
            }
        }

        for (owner, followed_users) in &mut self.followed_by_user {
            let mut changed = false;
            for user in &users_to_remove {
                changed |= followed_users.shift_remove(user);
            }
            if changed && let Some(revision) = self.list_revisions.get_mut(&(*owner, 3)) {
                revision.complete = false;
            }
        }
        for followers in self.followers_by_user.values_mut() {
            for user in &users_to_remove {
                followers.shift_remove(user);
            }
        }
        for (owner, muted_users) in &mut self.muted_by_user {
            let mut changed = false;
            for user in &users_to_remove {
                changed |= muted_users.shift_remove(user);
            }
            if changed && let Some(revision) = self.list_revisions.get_mut(&(*owner, 10_000)) {
                revision.complete = false;
            }
        }
        for muters in self.user_muted_by.values_mut() {
            for user in &users_to_remove {
                muters.shift_remove(user);
            }
        }

        users_to_remove.len()
    }
}

fn key(user: u32) -> String {
    format!("{user:064x}")
}

fn revision(user: u32, kind: u32) -> ListRevision {
    ListRevision {
        id: format!("{:064x}", u64::from(user) * 100_001 + u64::from(kind)),
        created_at: 100,
        complete: true,
    }
}

// Construct indexed state directly: graph updates/BFS are outside this pruning
// benchmark. Reverse edges, IDs, timestamps and revision heads are all present.
fn fixture(kept: u32, removed: u32, fanout: u32) -> SocialGraph {
    let mut graph = SocialGraph::new(&key(0));
    let total = kept + removed;
    for user in 1..total {
        assert_eq!(graph.ids.id(&key(user)).unwrap(), user);
    }
    for user in (0..total).rev() {
        let distance = if user == 0 {
            0
        } else if user < kept {
            1
        } else {
            1000
        };
        graph.follow_distance_by_user.insert(user, distance);
        graph
            .users_by_follow_distance
            .entry(distance)
            .or_default()
            .insert(user);
        graph.followed_by_user.insert(user, IndexSet::new());
        graph.followers_by_user.insert(user, IndexSet::new());
        graph.muted_by_user.insert(user, IndexSet::new());
        graph.user_muted_by.insert(user, IndexSet::new());
        graph.follow_list_created_at.insert(user, 100);
        graph.mute_list_created_at.insert(user, 100);
        for kind in [3, 10_000] {
            graph
                .list_revisions
                .insert((user, kind), revision(user, kind));
        }
    }
    for owner in 0..total {
        for offset in 1..=fanout.min(kept.saturating_sub(1)) {
            let target = (owner + offset) % kept;
            graph
                .followed_by_user
                .get_mut(&owner)
                .unwrap()
                .insert(target);
            graph
                .followers_by_user
                .get_mut(&target)
                .unwrap()
                .insert(owner);
        }
    }
    for target in kept..total {
        let owner = target % kept;
        graph.muted_by_user.get_mut(&owner).unwrap().insert(target);
        graph.user_muted_by.get_mut(&target).unwrap().insert(owner);
        // A removed author's own mute list must disappear from the inverse map.
        graph.muted_by_user.get_mut(&target).unwrap().insert(0);
        graph.user_muted_by.get_mut(&0).unwrap().insert(target);
    }
    graph
}

fn assert_same(actual: &SocialGraph, expected: &SocialGraph) {
    assert_eq!(actual.export_state(), expected.export_state());
    assert_eq!(actual.ids.current_unique_id, expected.ids.current_unique_id);
    assert_eq!(
        actual.ids.str_to_unique_id.iter().collect::<Vec<_>>(),
        expected.ids.str_to_unique_id.iter().collect::<Vec<_>>()
    );
}

#[test]
fn bulk_pruning_matches_legacy_state_order_and_stable_ids() {
    for seed in 0..96u32 {
        let kept = 2 + seed % 13;
        let mut graph = fixture(kept, seed % 11, seed % kept);
        // Imported state may contain stale buckets or other revision kinds.
        let candidate = kept + (seed % 11).saturating_sub(1);
        graph
            .users_by_follow_distance
            .entry(7)
            .or_default()
            .insert(candidate);
        graph
            .list_revisions
            .insert((candidate, 99), revision(candidate, 99));
        if let Some(revision) = graph.list_revisions.get_mut(&(0, 3)) {
            revision.complete = seed % 2 == 0;
        }
        let mut expected = graph.clone();
        assert_eq!(
            graph.remove_muted_not_followed_users(),
            expected.legacy_prune()
        );
        assert_same(&graph, &expected);
        assert_eq!(graph.get_root(), key(0));
        let next = graph.ids.current_unique_id;
        assert_eq!(graph.ids.id("after-pruning").unwrap(), next);
        assert_eq!(expected.ids.id("after-pruning").unwrap(), next);
        assert_same(&graph, &expected);
    }
}

#[test]
fn bulk_pruning_only_invalidates_lists_whose_edges_were_removed() {
    let mut graph = fixture(8, 3, 2);
    let before = graph.clone();
    let mut expected = graph.clone();
    assert_eq!(graph.remove_muted_not_followed_users(), 3);
    assert_eq!(expected.legacy_prune(), 3);
    assert_same(&graph, &expected);
    assert_eq!(graph.to_binary().unwrap(), expected.to_binary().unwrap());
    for owner in 0..8 {
        assert_eq!(
            graph.get_list_revision(&key(owner), 3),
            before.get_list_revision(&key(owner), 3)
        );
        let changed = before.muted_by_user[&owner]
            .iter()
            .any(|target| *target >= 8);
        let mut revision = before.get_list_revision(&key(owner), 10_000).unwrap();
        if changed {
            revision.complete = false;
        }
        assert_eq!(graph.get_list_revision(&key(owner), 10_000), Some(revision));
        assert_eq!(graph.ids.existing_id(&key(owner)), Some(owner));
    }
    for removed in 8..11 {
        assert_eq!(graph.ids.existing_id(&key(removed)), None);
        assert_eq!(graph.get_list_revision(&key(removed), 3), None);
        assert_eq!(graph.get_list_revision(&key(removed), 10_000), None);
    }
    // Root was muted only by the removed authors, but must always survive.
    assert_eq!(graph.get_root(), key(0));
    assert!(graph.get_user_muted_by(&key(0)).is_empty());
}

#[test]
fn bulk_pruning_preserves_legacy_removal_for_duplicate_imported_id_values() {
    let mut state = fixture(8, 3, 2).export_state();
    // An imported duplicate string can make the two ID indexes asymmetric.
    state
        .unique_ids
        .iter_mut()
        .find(|(_, id)| *id == 8)
        .unwrap()
        .0 = key(1);
    let mut graph = SocialGraph::from_state(state).unwrap();
    let mut expected = graph.clone();
    assert_eq!(
        graph.remove_muted_not_followed_users(),
        expected.legacy_prune()
    );
    assert_same(&graph, &expected);
}

#[test]
fn bulk_pruning_is_a_noop_without_candidates_and_after_completion() {
    let mut no_candidates = fixture(8, 0, 2);
    let before = no_candidates.clone();
    assert_eq!(no_candidates.remove_muted_not_followed_users(), 0);
    assert_same(&no_candidates, &before);
    let mut graph = fixture(8, 3, 2);
    assert_eq!(graph.remove_muted_not_followed_users(), 3);
    let once = graph.clone();
    assert_eq!(graph.remove_muted_not_followed_users(), 0);
    assert_same(&graph, &once);
}

#[test]
#[ignore = "bounded performance fixture; run serially in release mode with --nocapture"]
fn bulk_pruning_sparse_graph_benchmark() {
    assert!(!cfg!(debug_assertions), "Use --release for this benchmark");
    let mut small = fixture(4_000, 2_000, 24);
    let mut legacy = small.clone();
    let start = Instant::now();
    let legacy_removed = legacy.legacy_prune();
    let legacy_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let removed = small.remove_muted_not_followed_users();
    let bulk_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(removed, legacy_removed);
    assert_same(&small, &legacy);
    println!(
        "prune-small users=6000 edges=144000 removed={removed} legacy_ms={legacy_ms:.3} bulk_ms={bulk_ms:.3}"
    );
    drop(small);
    drop(legacy);

    let mut large = fixture(100_000, 50_000, 32);
    let edges_before = large.size().follows;
    let start = Instant::now();
    assert_eq!(large.remove_muted_not_followed_users(), 50_000);
    let elapsed = start.elapsed();
    assert_eq!(large.ids.unique_id_to_str.len(), 100_000);
    assert_eq!(large.size().follows, 3_200_000);
    assert_eq!(large.ids.current_unique_id, 150_000);
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "bulk prune exceeded bounded fixture latency: {elapsed:?}"
    );
    println!(
        "prune-large users=150000 edges={edges_before} removed=50000 elapsed_ms={:.3}",
        elapsed.as_secs_f64() * 1000.0
    );
}
