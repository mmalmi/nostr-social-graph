use crate::rating::Rating;

/// Provides follow distance from the user's perspective.
/// Consumer implements this — social-memory doesn't depend on nostr-social-graph.
pub trait FollowGraph {
    /// Follow distance from the user to a given npub.
    /// 0 = self, 1 = direct follow, 2 = follow-of-follow, etc.
    /// None = unknown / not in graph.
    fn follow_distance(&self, npub: &str) -> Option<u32>;
}

/// Configurable weights for reputation computation.
#[derive(Debug, Clone)]
pub struct TrustConfig {
    /// Base trust for each follow distance level.
    /// Index 0 = self (distance 0), index 1 = direct follow, etc.
    /// Beyond the vec length, trust is 0.
    pub distance_trust: Vec<f64>,
    /// How much a rater's signal is multiplied per hop of distance.
    /// e.g. 0.5 means a distance-2 rater's signal is worth 0.25x.
    pub attester_decay: f64,
    /// Weight multiplier for positive ratings.
    pub positive_weight: f64,
    /// Weight multiplier for negative ratings.
    pub negative_weight: f64,
    /// Maximum follow distance to consider for raters.
    /// Raters beyond this distance are ignored.
    pub max_attester_distance: u32,
}

impl Default for TrustConfig {
    fn default() -> Self {
        Self {
            distance_trust: vec![1.0, 0.8, 0.5, 0.25, 0.1],
            attester_decay: 0.5,
            positive_weight: 1.0,
            negative_weight: 1.5,
            max_attester_distance: 4,
        }
    }
}

/// Breakdown of how a reputation score was computed.
#[derive(Debug, Clone)]
pub struct ReputationScore {
    /// Overall score from -1.0 to 1.0.
    pub score: f64,
    /// Base trust from follow distance alone (0.0 to 1.0).
    pub follow_trust: f64,
    /// Weighted sum of positive rating signals before squashing.
    pub positive_signal: f64,
    /// Weighted sum of negative rating signals before squashing.
    pub negative_signal: f64,
    /// Number of ratings that contributed to the score.
    pub rating_count: u32,
}

/// Compute a reputation score for an entity/npub.
///
/// - `ratings`: all ratings about this entity
/// - `npubs`: the npubs associated with this entity (used for follow distance)
/// - `follow_graph`: provides follow distance from user's perspective
/// - `config`: weight configuration
pub fn compute_reputation(
    ratings: &[Rating],
    npubs: &[&str],
    follow_graph: &dyn FollowGraph,
    config: &TrustConfig,
) -> ReputationScore {
    // Base trust from follow distance — take the best (closest) npub
    let follow_trust = npubs
        .iter()
        .filter_map(|npub| follow_graph.follow_distance(npub))
        .min()
        .and_then(|d| config.distance_trust.get(d as usize).copied())
        .unwrap_or(0.0);

    let mut positive_signal = 0.0f64;
    let mut negative_signal = 0.0f64;
    let mut rating_count = 0u32;

    for r in ratings {
        let rater_distance = follow_graph.follow_distance(&r.rater);

        // Skip raters beyond max distance or unknown
        let distance = match rater_distance {
            Some(d) if d <= config.max_attester_distance => d,
            _ => continue,
        };

        // Weight decays exponentially with rater distance
        let weight = config.attester_decay.powi(distance as i32);

        let Ok(rating_score) = r.normalized_score() else {
            continue;
        };
        let rating_weight = (rating_score.unsigned_abs() as f64) / 100.0;

        if rating_score > 0 {
            positive_signal += weight * config.positive_weight * rating_weight;
            rating_count += 1;
        } else if rating_score < 0 {
            negative_signal += weight * config.negative_weight * rating_weight;
            rating_count += 1;
        }
        // Neutral ratings don't affect score
    }

    // Combine: follow trust + rating signal, squashed via tanh
    let raw = follow_trust + positive_signal - negative_signal;
    let score = raw.tanh();

    ReputationScore {
        score,
        follow_trust,
        positive_signal,
        negative_signal,
        rating_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockGraph {
        distances: HashMap<String, u32>,
    }

    impl MockGraph {
        fn new() -> Self {
            Self {
                distances: HashMap::new(),
            }
        }

        fn set(&mut self, npub: &str, distance: u32) -> &mut Self {
            self.distances.insert(npub.into(), distance);
            self
        }
    }

    impl FollowGraph for MockGraph {
        fn follow_distance(&self, npub: &str) -> Option<u32> {
            self.distances.get(npub).copied()
        }
    }

    fn make_positive(rater: &str) -> Rating {
        Rating::new(rater, "target", 100, 0, 100)
    }

    fn make_negative(rater: &str) -> Rating {
        Rating::new(rater, "target", 0, 0, 100)
    }

    fn make_neutral(rater: &str) -> Rating {
        Rating::new(rater, "target", 50, 0, 100)
    }

    #[test]
    fn follow_distance_only_direct_follow() {
        let mut graph = MockGraph::new();
        graph.set("npub1alice", 1);

        let score = compute_reputation(&[], &["npub1alice"], &graph, &TrustConfig::default());

        assert_eq!(score.follow_trust, 0.8);
        assert_eq!(score.positive_signal, 0.0);
        assert_eq!(score.negative_signal, 0.0);
        assert_eq!(score.rating_count, 0);
        assert!((score.score - 0.8_f64.tanh()).abs() < 0.001);
    }

    #[test]
    fn follow_distance_only_self() {
        let mut graph = MockGraph::new();
        graph.set("npub1self", 0);

        let score = compute_reputation(&[], &["npub1self"], &graph, &TrustConfig::default());
        assert_eq!(score.follow_trust, 1.0);
    }

    #[test]
    fn follow_distance_only_fof() {
        let mut graph = MockGraph::new();
        graph.set("npub1fof", 2);

        let score = compute_reputation(&[], &["npub1fof"], &graph, &TrustConfig::default());
        assert_eq!(score.follow_trust, 0.5);
    }

    #[test]
    fn follow_distance_unknown() {
        let graph = MockGraph::new();
        let score = compute_reputation(&[], &["npub1stranger"], &graph, &TrustConfig::default());
        assert_eq!(score.follow_trust, 0.0);
        assert_eq!(score.score, 0.0);
    }

    #[test]
    fn multiple_npubs_uses_closest() {
        let mut graph = MockGraph::new();
        graph.set("npub1far", 3);
        graph.set("npub1close", 1);

        let score = compute_reputation(
            &[],
            &["npub1far", "npub1close"],
            &graph,
            &TrustConfig::default(),
        );
        assert_eq!(score.follow_trust, 0.8);
    }

    #[test]
    fn positive_ratings_raise_score() {
        let mut graph = MockGraph::new();
        graph.set("npub1alice", 2);
        graph.set("npub1bob", 1);

        let pos = make_positive("npub1bob");
        let config = TrustConfig::default();

        let without = compute_reputation(&[], &["npub1alice"], &graph, &config);
        let with = compute_reputation(&[pos], &["npub1alice"], &graph, &config);

        assert!(with.score > without.score);
        assert!(with.positive_signal > 0.0);
        assert_eq!(with.rating_count, 1);
    }

    #[test]
    fn negative_ratings_lower_score() {
        let mut graph = MockGraph::new();
        graph.set("npub1alice", 2);
        graph.set("npub1bob", 1);

        let neg = make_negative("npub1bob");
        let config = TrustConfig::default();

        let without = compute_reputation(&[], &["npub1alice"], &graph, &config);
        let with = compute_reputation(&[neg], &["npub1alice"], &graph, &config);

        assert!(with.score < without.score);
        assert!(with.negative_signal > 0.0);
        assert_eq!(with.rating_count, 1);
    }

    #[test]
    fn neutral_ratings_dont_affect_score() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);
        graph.set("npub1rater", 1);

        let neutral = make_neutral("npub1rater");
        let config = TrustConfig::default();

        let without = compute_reputation(&[], &["npub1target"], &graph, &config);
        let with = compute_reputation(&[neutral], &["npub1target"], &graph, &config);

        assert_eq!(with.score, without.score);
        assert_eq!(with.rating_count, 0);
    }

    #[test]
    fn rater_distance_decay() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 2);
        graph.set("npub1close", 1);
        graph.set("npub1far", 3);

        let pos_close = make_positive("npub1close");
        let pos_far = make_positive("npub1far");
        let config = TrustConfig::default();

        let with_close = compute_reputation(&[pos_close], &["npub1target"], &graph, &config);
        let with_far = compute_reputation(&[pos_far], &["npub1target"], &graph, &config);

        assert!(with_close.positive_signal > with_far.positive_signal);
    }

    #[test]
    fn unknown_rater_ignored() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);

        let pos = make_positive("npub1unknown_rater");
        let config = TrustConfig::default();

        let score = compute_reputation(&[pos], &["npub1target"], &graph, &config);
        assert_eq!(score.positive_signal, 0.0);
        assert_eq!(score.rating_count, 0);
    }

    #[test]
    fn rater_beyond_max_distance_ignored() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);
        graph.set("npub1far", 10);

        let pos = make_positive("npub1far");
        let config = TrustConfig::default();

        let score = compute_reputation(&[pos], &["npub1target"], &graph, &config);
        assert_eq!(score.positive_signal, 0.0);
        assert_eq!(score.rating_count, 0);
    }

    #[test]
    fn score_bounded_by_tanh() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);
        graph.set("npub1a", 0);
        graph.set("npub1b", 0);
        graph.set("npub1c", 0);
        graph.set("npub1d", 0);
        graph.set("npub1e", 0);

        let positives: Vec<_> = ["npub1a", "npub1b", "npub1c", "npub1d", "npub1e"]
            .iter()
            .map(|r| make_positive(r))
            .collect();
        let config = TrustConfig::default();

        let score = compute_reputation(&positives, &["npub1target"], &graph, &config);
        assert!(score.score > 0.0);
        assert!(score.score <= 1.0);
    }

    #[test]
    fn heavily_negative_still_bounded() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 3);
        graph.set("npub1a", 0);
        graph.set("npub1b", 0);
        graph.set("npub1c", 0);

        let negatives: Vec<_> = ["npub1a", "npub1b", "npub1c"]
            .iter()
            .map(|r| make_negative(r))
            .collect();
        let config = TrustConfig::default();

        let score = compute_reputation(&negatives, &["npub1target"], &graph, &config);
        assert!(score.score < 0.0);
        assert!(score.score >= -1.0);
    }

    #[test]
    fn custom_config_weights() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);
        graph.set("npub1rater", 1);

        let pos = make_positive("npub1rater");

        let default_config = TrustConfig::default();
        let boosted_config = TrustConfig {
            positive_weight: 3.0,
            ..TrustConfig::default()
        };

        let normal = compute_reputation(
            std::slice::from_ref(&pos),
            &["npub1target"],
            &graph,
            &default_config,
        );
        let boosted = compute_reputation(&[pos], &["npub1target"], &graph, &boosted_config);

        assert!(boosted.positive_signal > normal.positive_signal);
        assert!(boosted.score > normal.score);
    }

    #[test]
    fn custom_distance_trust() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);

        let harsh_config = TrustConfig {
            distance_trust: vec![1.0, 0.1],
            ..TrustConfig::default()
        };

        let score = compute_reputation(&[], &["npub1target"], &graph, &harsh_config);
        assert_eq!(score.follow_trust, 0.1);
    }

    #[test]
    fn custom_max_rater_distance() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);
        graph.set("npub1rater", 3);

        let pos = make_positive("npub1rater");

        let strict_config = TrustConfig {
            max_attester_distance: 2,
            ..TrustConfig::default()
        };

        let score = compute_reputation(&[pos], &["npub1target"], &graph, &strict_config);
        assert_eq!(score.rating_count, 0);
    }

    #[test]
    fn no_npubs_no_graph_data() {
        let graph = MockGraph::new();
        let score = compute_reputation(&[], &[], &graph, &TrustConfig::default());
        assert_eq!(score.score, 0.0);
        assert_eq!(score.follow_trust, 0.0);
    }

    #[test]
    fn mixed_positive_and_negative() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 2);
        graph.set("npub1friend", 1);
        graph.set("npub1foe", 1);

        let pos = make_positive("npub1friend");
        let neg = make_negative("npub1foe");
        let config = TrustConfig::default();

        let score = compute_reputation(&[pos, neg], &["npub1target"], &graph, &config);
        assert_eq!(score.rating_count, 2);
        assert!(score.positive_signal > 0.0);
        assert!(score.negative_signal > 0.0);
    }

    #[test]
    fn distance_beyond_trust_vec_gives_zero() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 10);

        let score = compute_reputation(&[], &["npub1target"], &graph, &TrustConfig::default());
        assert_eq!(score.follow_trust, 0.0);
    }

    #[test]
    fn rater_at_distance_zero() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 2);
        graph.set("npub1self", 0);

        let pos = make_positive("npub1self");
        let config = TrustConfig::default();

        let score = compute_reputation(&[pos], &["npub1target"], &graph, &config);
        assert_eq!(score.positive_signal, config.positive_weight);
    }

    #[test]
    fn rating_magnitude_affects_signal() {
        let mut graph = MockGraph::new();
        graph.set("npub1target", 1);
        graph.set("npub1rater", 0);

        let weak = Rating::new("npub1rater", "target", 60, 0, 100);
        let strong = Rating::new("npub1rater", "target", 100, 0, 100);
        let config = TrustConfig::default();

        let weak_score = compute_reputation(&[weak], &["npub1target"], &graph, &config);
        let strong_score = compute_reputation(&[strong], &["npub1target"], &graph, &config);
        assert!(strong_score.positive_signal > weak_score.positive_signal);
    }
}
