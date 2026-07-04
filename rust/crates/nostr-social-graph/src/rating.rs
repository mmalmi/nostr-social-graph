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
    pub context: Option<String>,
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
            context: None,
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
    pub contexts: IndexSet<String>,
    pub max_rater_distance: u32,
    pub min_positive_score: i64,
    pub max_negative_score: i64,
    pub min_sample_count: Option<u64>,
}

impl RatingGraphConfig {
    pub fn for_contexts(contexts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            contexts: contexts.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    fn accepts_rating(&self, rating: &Rating) -> bool {
        if !self.contexts.is_empty()
            && !rating
                .context
                .as_ref()
                .is_some_and(|context| self.contexts.contains(context))
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
            contexts: IndexSet::new(),
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

        loop {
            projection.passes = projection.passes.saturating_add(1);
            let mut graph_changed = false;

            for rating in ratings {
                if accepted.contains(&rating.id) || !config.accepts_rating(rating) {
                    continue;
                }

                let rater_distance = self.get_follow_distance(&rating.rater);
                if rater_distance >= UNKNOWN_FOLLOW_DISTANCE
                    || rater_distance > config.max_rater_distance
                {
                    continue;
                }

                let normalized_score = rating.normalized_score()?;
                accepted.insert(rating.id.clone());
                projection.accepted_ratings += 1;

                if normalized_score >= config.min_positive_score {
                    projection.positive_ratings += 1;
                    if self.add_positive_relation(
                        &rating.rater,
                        &rating.subject,
                        rating.created_at,
                    )? {
                        graph_changed = true;
                        projection.positive_edges_added += 1;
                    }
                } else if normalized_score <= config.max_negative_score {
                    projection.negative_ratings += 1;
                    if self.add_negative_relation(
                        &rating.rater,
                        &rating.subject,
                        rating.created_at,
                    )? {
                        graph_changed = true;
                        projection.negative_edges_added += 1;
                    }
                } else {
                    projection.neutral_ratings += 1;
                }
            }

            if !graph_changed {
                break;
            }
        }

        projection.ignored_ratings = ratings.len().saturating_sub(projection.accepted_ratings);
        Ok(projection)
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

    fn rating(rater: &str, subject: &str, context: &str, value: i64) -> Rating {
        let mut rating = Rating::new(rater, subject, value, 0, 100);
        rating.context = Some(context.to_owned());
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
    fn positive_ratings_expand_graph_iteratively() {
        let mut graph = SocialGraph::new("local:root");
        let ratings = vec![
            rating("local:root", "peer:a", "peer", 80),
            rating("peer:a", "peer:b", "peer", 75),
        ];

        let projection = graph
            .apply_ratings(&ratings, &RatingGraphConfig::for_contexts(["peer"]))
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
            .apply_ratings(&ratings, &RatingGraphConfig::for_contexts(["peer"]))
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
            .apply_ratings(&ratings, &RatingGraphConfig::for_contexts(["peer"]))
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
    fn context_filter_keeps_app_policy_outside_the_format() {
        let mut graph = SocialGraph::new("local:root");
        let ratings = vec![
            rating("local:root", "peer:a", "peer", 80),
            rating("local:root", "peer:b", "other", 80),
        ];

        let projection = graph
            .apply_ratings(&ratings, &RatingGraphConfig::for_contexts(["peer"]))
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
        let mut config = RatingGraphConfig::for_contexts(["peer"]);
        config.min_sample_count = Some(2);

        let projection = graph.apply_ratings(&[weak, strong], &config).unwrap();

        assert_eq!(projection.accepted_ratings, 1);
        assert_eq!(
            graph.get_follow_distance("peer:weak"),
            UNKNOWN_FOLLOW_DISTANCE
        );
        assert_eq!(graph.get_follow_distance("peer:strong"), 1);
    }
}
