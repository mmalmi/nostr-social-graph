use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::{Result, SocialGraph, SocialGraphError, UNKNOWN_FOLLOW_DISTANCE};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rating {
    pub id: String,
    pub rater: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub rating: i64,
    pub min_rating: i64,
    pub max_rating: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub created_at: u64,
}

impl Rating {
    pub fn new(
        rater: impl Into<String>,
        subject: impl Into<String>,
        rating: i64,
        min_rating: i64,
        max_rating: i64,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            rater: rater.into(),
            subject: subject.into(),
            scope: None,
            rating,
            min_rating,
            max_rating,
            sample_count: None,
            window_start: None,
            window_end: None,
            evidence: Vec::new(),
            reason: None,
            tags: Vec::new(),
            created_at: now_unix(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.min_rating >= self.max_rating {
            return Err(SocialGraphError::InvalidRatingRange {
                min_rating: self.min_rating,
                max_rating: self.max_rating,
            });
        }
        if self.rating < self.min_rating || self.rating > self.max_rating {
            return Err(SocialGraphError::RatingOutOfRange {
                rating: self.rating,
                min_rating: self.min_rating,
                max_rating: self.max_rating,
            });
        }
        if let (Some(window_start), Some(window_end)) = (self.window_start, self.window_end)
            && window_start > window_end
        {
            return Err(SocialGraphError::InvalidRatingWindow {
                window_start,
                window_end,
            });
        }
        Ok(())
    }

    pub fn normalized_score(&self) -> Result<i64> {
        self.validate()?;
        let rating = i128::from(self.rating);
        let min = i128::from(self.min_rating);
        let max = i128::from(self.max_rating);
        let width = max - min;
        let centered = rating.saturating_mul(2) - min - max;
        Ok(((centered.saturating_mul(100)) / width) as i64)
    }

    pub fn is_positive(&self) -> bool {
        self.normalized_score().is_ok_and(|score| score > 0)
    }

    pub fn is_negative(&self) -> bool {
        self.normalized_score().is_ok_and(|score| score < 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingGraphConfig {
    pub scopes: IndexSet<String>,
    pub max_rater_distance: u32,
    pub min_positive_score: i64,
    pub max_negative_score: i64,
    pub min_sample_count: Option<u64>,
}

impl RatingGraphConfig {
    pub fn for_scopes(scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            scopes: scopes.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    fn accepts_rating(&self, rating: &Rating) -> bool {
        if !self.scopes.is_empty()
            && !rating
                .scope
                .as_ref()
                .is_some_and(|scope| self.scopes.contains(scope))
        {
            return false;
        }
        if let Some(min_sample_count) = self.min_sample_count
            && rating.sample_count.unwrap_or(0) < min_sample_count
        {
            return false;
        }
        true
    }
}

impl Default for RatingGraphConfig {
    fn default() -> Self {
        Self {
            scopes: IndexSet::new(),
            max_rater_distance: 3,
            min_positive_score: 1,
            max_negative_score: -1,
            min_sample_count: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RatingGraphProjection {
    pub passes: u32,
    pub accepted_ratings: usize,
    pub ignored_ratings: usize,
    pub positive_ratings: usize,
    pub negative_ratings: usize,
    pub neutral_ratings: usize,
    pub positive_edges_added: usize,
    pub negative_edges_added: usize,
}

impl SocialGraph {
    pub fn apply_ratings(
        &mut self,
        ratings: &[Rating],
        config: &RatingGraphConfig,
    ) -> Result<RatingGraphProjection> {
        for rating in ratings {
            rating.validate()?;
        }

        let mut projection = RatingGraphProjection::default();
        let mut accepted = IndexSet::<String>::new();
        let root = self.get_root().to_owned();

        // Root ratings define the local authority boundary. Apply them before
        // discovering transitive raters so input order cannot let a
        // root-muted identity influence the projection through another path.
        for rating in ratings {
            if rating.rater != root
                || !config.accepts_rating(rating)
                || !accepted.insert(rating.id.clone())
            {
                continue;
            }
            self.apply_accepted_rating(rating, config, &mut projection)?;
        }
        let root_muted_raters = self
            .get_muted_by_user(&root)
            .into_iter()
            .collect::<IndexSet<_>>();

        loop {
            projection.passes = projection.passes.saturating_add(1);
            let mut graph_changed = false;

            for rating in ratings {
                if rating.rater == root
                    || accepted.contains(&rating.id)
                    || !config.accepts_rating(rating)
                    || root_muted_raters.contains(&rating.rater)
                {
                    continue;
                }

                let rater_distance = self.get_follow_distance(&rating.rater);
                if rater_distance >= UNKNOWN_FOLLOW_DISTANCE
                    || rater_distance > config.max_rater_distance
                {
                    continue;
                }

                accepted.insert(rating.id.clone());
                graph_changed |= self.apply_accepted_rating(rating, config, &mut projection)?;
            }

            if !graph_changed {
                break;
            }
        }

        projection.ignored_ratings = ratings.len().saturating_sub(projection.accepted_ratings);
        Ok(projection)
    }

    fn apply_accepted_rating(
        &mut self,
        rating: &Rating,
        config: &RatingGraphConfig,
        projection: &mut RatingGraphProjection,
    ) -> Result<bool> {
        let normalized_score = rating.normalized_score()?;
        projection.accepted_ratings += 1;
        if normalized_score >= config.min_positive_score {
            projection.positive_ratings += 1;
            let changed =
                self.add_positive_relation(&rating.rater, &rating.subject, rating.created_at)?;
            projection.positive_edges_added += usize::from(changed);
            Ok(changed)
        } else if normalized_score <= config.max_negative_score {
            projection.negative_ratings += 1;
            let changed =
                self.add_negative_relation(&rating.rater, &rating.subject, rating.created_at)?;
            projection.negative_edges_added += usize::from(changed);
            Ok(changed)
        } else {
            projection.neutral_ratings += 1;
            Ok(false)
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rating(rater: &str, subject: &str, scope: &str, value: i64) -> Rating {
        let mut rating = Rating::new(rater, subject, value, 0, 100);
        rating.scope = Some(scope.to_owned());
        rating.created_at = 1_000;
        rating
    }

    #[test]
    fn normalized_score_uses_integer_range() {
        assert_eq!(
            Rating::new("alice", "bob", 100, 0, 100)
                .normalized_score()
                .unwrap(),
            100
        );
        assert_eq!(
            Rating::new("alice", "bob", 50, 0, 100)
                .normalized_score()
                .unwrap(),
            0
        );
        assert_eq!(
            Rating::new("alice", "bob", 0, 0, 100)
                .normalized_score()
                .unwrap(),
            -100
        );
        assert_eq!(
            Rating::new("alice", "bob", 4, 1, 5)
                .normalized_score()
                .unwrap(),
            50
        );
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        assert!(matches!(
            Rating::new("alice", "bob", 5, 5, 5).validate(),
            Err(SocialGraphError::InvalidRatingRange { .. })
        ));
        assert!(matches!(
            Rating::new("alice", "bob", 6, 0, 5).validate(),
            Err(SocialGraphError::RatingOutOfRange { .. })
        ));

        let mut rating = Rating::new("alice", "bob", 5, 0, 10);
        rating.window_start = Some(20);
        rating.window_end = Some(10);
        assert!(matches!(
            rating.validate(),
            Err(SocialGraphError::InvalidRatingWindow { .. })
        ));
    }

    #[test]
    fn scope_serializes_canonically() {
        let rating: Rating = serde_json::from_str(
            r#"{
                "id": "rating-1",
                "rater": "alice",
                "subject": "bob",
                "scope": "peer",
                "rating": 80,
                "min_rating": 0,
                "max_rating": 100,
                "created_at": 1000
            }"#,
        )
        .unwrap();

        assert_eq!(rating.scope.as_deref(), Some("peer"));
        let serialized = serde_json::to_string(&rating).unwrap();
        assert!(serialized.contains("\"scope\":\"peer\""));
        assert!(!serialized.contains("\"context\""));
    }

    #[test]
    fn context_is_not_a_rating_scope_alias() {
        let rating: Rating = serde_json::from_str(
            r#"{
                "id": "rating-1",
                "rater": "alice",
                "subject": "bob",
                "context": "peer",
                "rating": 80,
                "min_rating": 0,
                "max_rating": 100,
                "created_at": 1000
            }"#,
        )
        .unwrap();

        assert_eq!(rating.scope, None);
        let serialized = serde_json::to_string(&rating).unwrap();
        assert!(!serialized.contains("\"scope\""));
        assert!(!serialized.contains("\"context\""));
    }

    #[test]
    fn positive_ratings_expand_graph_iteratively() {
        let mut graph = SocialGraph::new("local:root");
        let ratings = vec![
            rating("local:root", "peer:a", "peer", 80),
            rating("peer:a", "peer:b", "peer", 75),
        ];

        let projection = graph
            .apply_ratings(&ratings, &RatingGraphConfig::for_scopes(["peer"]))
            .unwrap();

        assert_eq!(projection.accepted_ratings, 2);
        assert_eq!(projection.positive_edges_added, 2);
        assert_eq!(graph.get_follow_distance("peer:a"), 1);
        assert_eq!(graph.get_follow_distance("peer:b"), 2);
    }

    #[test]
    fn unknown_raters_cannot_introduce_subjects() {
        let mut graph = SocialGraph::new("local:root");
        let ratings = vec![
            rating("spammer", "peer:bad", "peer", 100),
            rating("local:root", "peer:a", "peer", 80),
        ];

        let projection = graph
            .apply_ratings(&ratings, &RatingGraphConfig::for_scopes(["peer"]))
            .unwrap();

        assert_eq!(projection.accepted_ratings, 1);
        assert_eq!(projection.ignored_ratings, 1);
        assert_eq!(
            graph.get_follow_distance("peer:bad"),
            UNKNOWN_FOLLOW_DISTANCE
        );
        assert_eq!(graph.get_follow_distance("peer:a"), 1);
    }

    #[test]
    fn negative_ratings_do_not_expand_reachability() {
        let mut graph = SocialGraph::new("local:root");
        let ratings = vec![
            rating("local:root", "peer:a", "peer", 80),
            rating("peer:a", "peer:bad", "peer", 0),
            rating("peer:bad", "peer:c", "peer", 100),
        ];

        let projection = graph
            .apply_ratings(&ratings, &RatingGraphConfig::for_scopes(["peer"]))
            .unwrap();

        assert_eq!(projection.accepted_ratings, 2);
        assert_eq!(projection.negative_edges_added, 1);
        assert_eq!(
            graph.get_follow_distance("peer:bad"),
            UNKNOWN_FOLLOW_DISTANCE
        );
        assert_eq!(
            graph.get_user_muted_by("peer:bad"),
            vec!["peer:a".to_owned()]
        );
        assert_eq!(graph.get_follow_distance("peer:c"), UNKNOWN_FOLLOW_DISTANCE);
    }

    #[test]
    fn scope_filter_keeps_app_policy_outside_the_format() {
        let mut graph = SocialGraph::new("local:root");
        let ratings = vec![
            rating("local:root", "peer:a", "peer", 80),
            rating("local:root", "peer:b", "other", 80),
        ];

        let projection = graph
            .apply_ratings(&ratings, &RatingGraphConfig::for_scopes(["peer"]))
            .unwrap();

        assert_eq!(projection.accepted_ratings, 1);
        assert_eq!(graph.get_follow_distance("peer:a"), 1);
        assert_eq!(graph.get_follow_distance("peer:b"), UNKNOWN_FOLLOW_DISTANCE);
    }

    #[test]
    fn sample_count_filter_is_optional_and_concrete() {
        let mut graph = SocialGraph::new("local:root");
        let mut weak = rating("local:root", "peer:weak", "peer", 80);
        weak.sample_count = Some(1);
        let mut strong = rating("local:root", "peer:strong", "peer", 80);
        strong.sample_count = Some(3);
        let mut config = RatingGraphConfig::for_scopes(["peer"]);
        config.min_sample_count = Some(2);

        let projection = graph.apply_ratings(&[weak, strong], &config).unwrap();

        assert_eq!(projection.accepted_ratings, 1);
        assert_eq!(
            graph.get_follow_distance("peer:weak"),
            UNKNOWN_FOLLOW_DISTANCE
        );
        assert_eq!(graph.get_follow_distance("peer:strong"), 1);
    }

    #[test]
    fn root_mute_suppresses_reachable_raters_influence() {
        let mut graph = SocialGraph::new("local:root");
        let projection = graph
            .apply_ratings(
                &poisoning_ratings(0),
                &RatingGraphConfig::for_scopes(["peer"]),
            )
            .unwrap();

        assert_root_revocation_projection(&graph, &projection);
    }

    #[test]
    fn root_mute_projection_is_input_order_independent() {
        let ratings = poisoning_ratings(0);
        let mut reversed = ratings.clone();
        reversed.reverse();
        let config = RatingGraphConfig::for_scopes(["peer"]);
        let mut forward_graph = SocialGraph::new("local:root");
        let mut reversed_graph = SocialGraph::new("local:root");

        let forward = forward_graph.apply_ratings(&ratings, &config).unwrap();
        let reversed = reversed_graph.apply_ratings(&reversed, &config).unwrap();

        assert_root_revocation_projection(&forward_graph, &forward);
        assert_root_revocation_projection(&reversed_graph, &reversed);
        assert_eq!(projection_counts(&forward), projection_counts(&reversed));
    }

    #[test]
    fn root_recovery_reactivates_retained_ratings_on_rebuild() {
        let mut graph = SocialGraph::new("local:root");
        let projection = graph
            .apply_ratings(
                &poisoning_ratings(100),
                &RatingGraphConfig::for_scopes(["peer"]),
            )
            .unwrap();

        assert_eq!(projection.accepted_ratings, 5);
        assert_eq!(projection.ignored_ratings, 0);
        assert_eq!(graph.get_follow_distance("peer:poisoner"), 1);
        assert!(graph.get_user_muted_by("peer:poisoner").is_empty());
        for target in ["peer:target-one", "peer:target-two"] {
            assert_eq!(
                graph.get_user_muted_by(target),
                vec!["peer:poisoner".to_owned()]
            );
        }
    }

    fn poisoning_ratings(root_to_poisoner: i64) -> Vec<Rating> {
        vec![
            rating("local:root", "peer:origin", "peer", 100),
            rating("peer:origin", "peer:poisoner", "peer", 100),
            rating("peer:poisoner", "peer:target-one", "peer", 0),
            rating("peer:poisoner", "peer:target-two", "peer", 0),
            rating("local:root", "peer:poisoner", "peer", root_to_poisoner),
        ]
    }

    fn assert_root_revocation_projection(graph: &SocialGraph, projection: &RatingGraphProjection) {
        assert_eq!(projection.accepted_ratings, 3);
        assert_eq!(projection.ignored_ratings, 2);
        assert_eq!(graph.get_follow_distance("peer:poisoner"), 2);
        assert_eq!(
            graph.get_user_muted_by("peer:poisoner"),
            vec!["local:root".to_owned()]
        );
        assert!(graph.get_user_muted_by("peer:target-one").is_empty());
        assert!(graph.get_user_muted_by("peer:target-two").is_empty());
    }

    fn projection_counts(projection: &RatingGraphProjection) -> (usize, usize, usize, usize) {
        (
            projection.accepted_ratings,
            projection.ignored_ratings,
            projection.positive_edges_added,
            projection.negative_edges_added,
        )
    }
}
