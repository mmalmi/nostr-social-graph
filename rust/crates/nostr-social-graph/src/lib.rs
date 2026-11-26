use std::collections::{HashMap, HashSet, VecDeque};
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::{IndexMap, IndexSet};
use uuid::Uuid;

#[cfg(test)]
mod prune_tests;

#[cfg(test)]
mod alias_tests;
mod aliases;
pub use aliases::canonical_node_id;

mod mobile;
pub use mobile::{MOBILE_MAX_BYTES, MOBILE_MIN_USERS, MobileGraphError};
#[cfg(feature = "public-snapshot")]
pub mod public_snapshot;

mod revision;
pub use revision::*;

mod rating;
pub use rating::*;

mod storage;
use storage::{
    CompactRevision, DistanceMap, EdgeSet, NodeKey, NodeKeyRef, UniqueIds, accepts_compact_revision,
};

const BINARY_FORMAT_VERSION: u64 = 4;
pub const BINARY_FORMAT_VERSION_V5: u64 = 5;
const BINARY_FORMAT_VERSION_V2: u64 = 2;
const BINARY_CHUNK_SIZE: usize = 16 * 1024;
const MAX_FUTURE_EVENT_SECONDS: u64 = 10 * 60;
const UNKNOWN_FOLLOW_DISTANCE: u32 = 1000;
/// Marks a user whose path was cut while its new distance is worked out.
/// Never left in the graph once an update returns.
const DETACHED_DISTANCE: u32 = u32::MAX;
/// Users whose distance changed in one update, with the distance they had.
type Moved = IndexMap<u32, Option<u32>>;
type DecodedLists = (
    IndexMap<u32, EdgeSet>,
    IndexMap<u32, EdgeSet>,
    IndexMap<u32, u64>,
);
type RawLists = (Vec<(u32, Vec<u32>)>, Vec<(u32, u64)>);
const BINARY_NODE_ID_PUBKEY: u64 = 0;
const BINARY_NODE_ID_UUID: u64 = 1;
const BINARY_NODE_ID_STRING: u64 = 2;

#[derive(Debug, thiserror::Error)]
pub enum SocialGraphError {
    #[error("cannot store empty or whitespace-only strings")]
    EmptyString,
    #[error("invalid id {0}")]
    InvalidId(u32),
    #[error("snapshot contains conflicting latest follow lists for node {0}")]
    ConflictingFollowLists(u32),
    #[error("invalid binary version {0}")]
    InvalidVersion(u64),
    #[error("unexpected end of binary data")]
    UnexpectedEof,
    #[error("invalid binary node id type {0}")]
    InvalidBinaryNodeIdType(u64),
    #[error("invalid UTF-8 binary node id")]
    InvalidBinaryNodeIdUtf8,
    #[error("invalid UUID binary node id")]
    InvalidBinaryNodeIdUuid,
    #[error("rating range must have min_rating < max_rating (got {min_rating}..{max_rating})")]
    InvalidRatingRange { min_rating: i64, max_rating: i64 },
    #[error("rating {rating} is outside range {min_rating}..{max_rating}")]
    RatingOutOfRange {
        rating: i64,
        min_rating: i64,
        max_rating: i64,
    },
    #[error("rating window_start {window_start} is after window_end {window_end}")]
    InvalidRatingWindow { window_start: u64, window_end: u64 },
}

pub type Result<T> = std::result::Result<T, SocialGraphError>;

pub trait SocialGraphBackend {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Older backends may return no metadata; these lists require ordinary catch-up.
    fn get_list_revisions(
        &self,
    ) -> std::result::Result<Vec<(String, u32, ListRevision)>, Self::Error> {
        Ok(Vec::new())
    }

    fn get_root(&self) -> std::result::Result<String, Self::Error>;

    fn set_root(&mut self, root: &str) -> std::result::Result<(), Self::Error>;

    fn handle_event(
        &mut self,
        event: &NostrEvent,
        allow_unknown_authors: bool,
        overmute_threshold: f64,
    ) -> std::result::Result<(), Self::Error>;

    fn get_follow_distance(&self, user: &str) -> std::result::Result<u32, Self::Error>;

    fn is_following(
        &self,
        follower: &str,
        followed_user: &str,
    ) -> std::result::Result<bool, Self::Error>;

    fn get_followed_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error>;

    fn get_followers_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error>;

    fn get_muted_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error>;

    fn get_user_muted_by(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error>;

    fn get_follow_list_created_at(
        &self,
        user: &str,
    ) -> std::result::Result<Option<u64>, Self::Error>;

    fn get_mute_list_created_at(&self, user: &str)
    -> std::result::Result<Option<u64>, Self::Error>;

    fn is_overmuted(&self, user: &str, threshold: f64) -> std::result::Result<bool, Self::Error>;

    fn flush(&mut self) -> std::result::Result<(), Self::Error> {
        Ok(())
    }

    fn has_unflushed_changes(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialGraphState {
    pub list_revisions: Vec<(u32, u32, ListRevision)>,
    pub root: String,
    pub unique_ids: Vec<(String, u32)>,
    pub follow_distance_by_user: Vec<(u32, u32)>,
    pub users_by_follow_distance: Vec<(u32, Vec<u32>)>,
    pub followed_by_user: Vec<(u32, Vec<u32>)>,
    pub followers_by_user: Vec<(u32, Vec<u32>)>,
    pub follow_list_created_at: Vec<(u32, u64)>,
    pub muted_by_user: Vec<(u32, Vec<u32>)>,
    pub user_muted_by: Vec<(u32, Vec<u32>)>,
    pub mute_list_created_at: Vec<(u32, u64)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BinaryBudget {
    pub max_nodes: Option<usize>,
    pub max_edges: Option<usize>,
    pub max_distance: Option<u32>,
    pub max_edges_per_node: Option<usize>,
}

impl BinaryBudget {
    fn has_active_limits(self) -> bool {
        self.max_nodes.is_some_and(|value| value > 0)
            || self.max_edges.is_some_and(|value| value > 0)
            || self.max_distance.is_some()
            || self.max_edges_per_node.is_some_and(|value| value > 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphStats {
    pub users: usize,
    pub follows: usize,
    pub mutes: usize,
    pub size_by_distance: IndexMap<u32, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrEvent {
    pub created_at: u64,
    pub content: String,
    pub tags: Vec<Vec<String>>,
    pub kind: u32,
    pub pubkey: String,
    pub id: String,
    pub sig: String,
}

#[derive(Debug, Clone, Default)]
pub struct SocialGraph {
    list_revisions: IndexMap<(u32, u32), CompactRevision>,
    root: u32,
    root_str: String,
    follow_distance_by_user: DistanceMap,
    users_by_follow_distance: IndexMap<u32, IndexSet<u32>>,
    followed_by_user: IndexMap<u32, EdgeSet>,
    followers_by_user: IndexMap<u32, EdgeSet>,
    follow_list_created_at: IndexMap<u32, u64>,
    muted_by_user: IndexMap<u32, EdgeSet>,
    user_muted_by: IndexMap<u32, EdgeSet>,
    mute_list_created_at: IndexMap<u32, u64>,
    ids: UniqueIds,
}

#[derive(Debug)]
struct BinaryPlan {
    used_ids: IndexSet<u32>,
    follow_edge_count: IndexMap<u32, usize>,
    mute_edge_count: IndexMap<u32, usize>,
    follow_owners: Vec<u32>,
    mute_owners: Vec<u32>,
}

impl SocialGraph {
    pub fn new(root: &str) -> Self {
        let mut ids = UniqueIds::default();
        let root_id = ids.id(root).expect("root must not be empty");
        let root_str = ids.str_owned(root_id).expect("root id should always exist");
        let mut follow_distance_by_user = DistanceMap::with_dense_hint(1);
        follow_distance_by_user.insert(root_id, 0);
        let mut users_by_follow_distance = IndexMap::new();
        let mut bucket = IndexSet::new();
        bucket.insert(root_id);
        users_by_follow_distance.insert(0, bucket);
        Self {
            list_revisions: IndexMap::new(),
            root: root_id,
            root_str,
            follow_distance_by_user,
            users_by_follow_distance,
            followed_by_user: IndexMap::new(),
            followers_by_user: IndexMap::new(),
            follow_list_created_at: IndexMap::new(),
            muted_by_user: IndexMap::new(),
            user_muted_by: IndexMap::new(),
            mute_list_created_at: IndexMap::new(),
            ids,
        }
    }

    pub fn get_list_revision(&self, user: &str, kind: u32) -> Option<ListRevision> {
        self.list_revisions
            .get(&(self.ids.existing_id(user)?, kind))
            .map(CompactRevision::to_public)
    }

    pub fn get_list_revisions(&self) -> Vec<(String, u32, ListRevision)> {
        self.list_revisions
            .iter()
            .filter_map(|((owner, kind), revision)| {
                Some((
                    self.ids.str_owned(*owner).ok()?,
                    *kind,
                    revision.to_public(),
                ))
            })
            .collect()
    }

    pub fn mark_list_incomplete(&mut self, user: &str, kind: u32) {
        if let Some(owner) = self.ids.existing_id(user)
            && let Some(revision) = self.list_revisions.get_mut(&(owner, kind))
        {
            revision.complete = false;
        }
    }

    pub fn get_root(&self) -> &str {
        &self.root_str
    }

    pub fn set_root(&mut self, root: &str) -> Result<()> {
        let root_id = self.ids.id(root)?;
        if root_id == self.root {
            return Ok(());
        }
        self.root = root_id;
        self.root_str = self.ids.str_owned(root_id)?;
        self.recalculate_follow_distances();
        Ok(())
    }

    pub fn handle_event(
        &mut self,
        event: &NostrEvent,
        allow_unknown_authors: bool,
        overmute_threshold: f64,
    ) {
        if !matches!(event.kind, 3 | 10_000) {
            return;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if event.created_at > now.saturating_add(MAX_FUTURE_EVENT_SECONDS) {
            return;
        }

        let author = if allow_unknown_authors {
            let Ok(author) = self.ids.id(&event.pubkey) else {
                return;
            };
            author
        } else {
            let Some(author) = self.ids.existing_id(&event.pubkey) else {
                return;
            };
            if !self.follow_distance_by_user.contains_key(author) {
                return;
            }
            author
        };

        if self.is_overmuted_id(author, overmute_threshold) {
            return;
        }

        match event.kind {
            3 => self.handle_follow_list(author, event.created_at, &event.id, &event.tags),
            10_000 => self.handle_mute_list(author, event.created_at, &event.id, &event.tags),
            _ => {}
        }
    }

    pub fn add_positive_relation(
        &mut self,
        rater: &str,
        target: &str,
        created_at: u64,
    ) -> Result<bool> {
        self.mark_list_incomplete(rater, 3);
        let rater_id = self.ids.id(rater)?;
        let target_id = self.ids.id(target)?;
        if rater_id == target_id {
            return Ok(false);
        }

        let already_present = self
            .followed_by_user
            .get(&rater_id)
            .is_some_and(|targets| targets.contains(target_id));
        self.private_add_follower(target_id, rater_id);
        self.follow_list_created_at
            .entry(rater_id)
            .and_modify(|existing| *existing = (*existing).max(created_at))
            .or_insert(created_at);
        Ok(!already_present)
    }

    pub fn add_negative_relation(
        &mut self,
        rater: &str,
        target: &str,
        created_at: u64,
    ) -> Result<bool> {
        self.mark_list_incomplete(rater, 10_000);
        let rater_id = self.ids.id(rater)?;
        let target_id = self.ids.id(target)?;
        if rater_id == target_id {
            return Ok(false);
        }

        let entry = self.muted_by_user.entry(rater_id).or_default();
        let changed = entry.insert(target_id);
        if changed {
            self.user_muted_by
                .entry(target_id)
                .or_default()
                .insert(rater_id);
        }
        self.mute_list_created_at
            .entry(rater_id)
            .and_modify(|existing| *existing = (*existing).max(created_at))
            .or_insert(created_at);
        Ok(changed)
    }

    pub fn recalculate_follow_distances(&mut self) {
        let dense_hint = dense_hint_for(&self.ids);
        self.follow_distance_by_user
            .clear_with_dense_hint(dense_hint);
        self.users_by_follow_distance.clear();
        self.follow_distance_by_user.insert(self.root, 0);
        let mut root_bucket = IndexSet::new();
        root_bucket.insert(self.root);
        self.users_by_follow_distance.insert(0, root_bucket);

        let mut queue = VecDeque::with_capacity(self.ids.len().min(65_536));
        queue.push_back(self.root);
        while let Some(user) = queue.pop_front() {
            let Some(distance) = self.follow_distance_by_user.get(user) else {
                continue;
            };
            let next_distance = distance.saturating_add(1);
            let Some(targets) = self.followed_by_user.get(&user) else {
                continue;
            };
            for &target in targets {
                if self.follow_distance_by_user.contains_key(target) {
                    continue;
                }
                self.follow_distance_by_user.insert(target, next_distance);
                self.users_by_follow_distance
                    .entry(next_distance)
                    .or_default()
                    .insert(target);
                queue.push_back(target);
            }
        }
    }

    pub fn get_follow_distance(&self, user: &str) -> u32 {
        let Some(user_id) = self.ids.existing_id(user) else {
            return UNKNOWN_FOLLOW_DISTANCE;
        };
        self.follow_distance_by_user
            .get(user_id)
            .unwrap_or(UNKNOWN_FOLLOW_DISTANCE)
    }

    pub fn is_following(&self, follower: &str, followed_user: &str) -> bool {
        let Some(follower_id) = self.ids.existing_id(follower) else {
            return false;
        };
        let Some(followed_id) = self.ids.existing_id(followed_user) else {
            return false;
        };
        self.followed_by_user
            .get(&follower_id)
            .is_some_and(|set| set.contains(followed_id))
    }

    pub fn get_followed_by_user(&self, user: &str) -> Vec<String> {
        let Some(user_id) = self.ids.existing_id(user) else {
            return Vec::new();
        };
        self.followed_by_user
            .get(&user_id)
            .into_iter()
            .flat_map(|set| set.iter())
            .filter_map(|id| self.ids.str_owned(*id).ok())
            .collect()
    }

    pub fn get_followers_by_user(&self, user: &str) -> Vec<String> {
        let Some(user_id) = self.ids.existing_id(user) else {
            return Vec::new();
        };
        self.followers_by_user
            .get(&user_id)
            .into_iter()
            .flat_map(|set| set.iter())
            .filter_map(|id| self.ids.str_owned(*id).ok())
            .collect()
    }

    pub fn get_muted_by_user(&self, user: &str) -> Vec<String> {
        let Some(user_id) = self.ids.existing_id(user) else {
            return Vec::new();
        };
        self.muted_by_user
            .get(&user_id)
            .into_iter()
            .flat_map(|set| set.iter())
            .filter_map(|id| self.ids.str_owned(*id).ok())
            .collect()
    }

    pub fn get_user_muted_by(&self, user: &str) -> Vec<String> {
        let Some(user_id) = self.ids.existing_id(user) else {
            return Vec::new();
        };
        self.user_muted_by
            .get(&user_id)
            .into_iter()
            .flat_map(|set| set.iter())
            .filter_map(|id| self.ids.str_owned(*id).ok())
            .collect()
    }

    pub fn get_follow_list_created_at(&self, user: &str) -> Option<u64> {
        let user_id = self.ids.existing_id(user)?;
        self.follow_list_created_at.get(&user_id).copied()
    }

    pub fn get_mute_list_created_at(&self, user: &str) -> Option<u64> {
        let user_id = self.ids.existing_id(user)?;
        self.mute_list_created_at.get(&user_id).copied()
    }

    pub fn size(&self) -> GraphStats {
        let follows = self
            .followed_by_user
            .values()
            .map(EdgeSet::len)
            .sum::<usize>();
        let mutes = self.muted_by_user.values().map(EdgeSet::len).sum::<usize>();
        let size_by_distance = self
            .users_by_follow_distance
            .iter()
            .map(|(distance, users)| (*distance, users.len()))
            .collect();

        GraphStats {
            users: if self.follow_distance_by_user.is_empty() {
                self.ids.len()
            } else {
                self.follow_distance_by_user.len()
            },
            follows,
            mutes,
            size_by_distance,
        }
    }

    pub fn get_users_by_follow_distance(&self, distance: u32) -> Vec<String> {
        self.users_by_follow_distance
            .get(&distance)
            .into_iter()
            .flat_map(|users| users.iter())
            .filter_map(|id| self.ids.str_owned(*id).ok())
            .collect()
    }

    pub fn users_in_distance_order(&self, up_to_distance: Option<u32>) -> Vec<String> {
        let mut distances: Vec<u32> = self.users_by_follow_distance.keys().copied().collect();
        distances.sort_unstable();
        let mut users = Vec::new();
        for distance in distances {
            if up_to_distance.is_some_and(|max_distance| distance > max_distance) {
                break;
            }
            users.extend(self.get_users_by_follow_distance(distance));
        }
        users
    }

    pub fn remove_muted_not_followed_users(&mut self) -> usize {
        let has_followers: HashSet<u32> = self
            .followed_by_user
            .values()
            .flat_map(|users| users.iter().copied())
            .collect();
        let users_to_remove: HashSet<u32> = self
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

        // Stable bulk retention visits each entry/edge once. Repeated shift_remove
        // would compact these indexed collections once for every removed user.
        self.list_revisions.retain(|(owner, kind), _| {
            !users_to_remove.contains(owner) || !matches!(*kind, 3 | 10_000)
        });
        for (&distance, users) in &mut self.users_by_follow_distance {
            // Preserve the assigned-bucket behavior even for imported state whose
            // distance buckets and distance map have not yet been reconciled.
            users.retain(|user| {
                !users_to_remove.contains(user)
                    || self.follow_distance_by_user.get(*user) != Some(distance)
            });
        }
        self.follow_distance_by_user
            .retain(|user, _| !users_to_remove.contains(&user));
        self.followed_by_user
            .retain(|user, _| !users_to_remove.contains(user));
        self.followers_by_user
            .retain(|user, _| !users_to_remove.contains(user));
        self.follow_list_created_at
            .retain(|user, _| !users_to_remove.contains(user));
        self.muted_by_user
            .retain(|user, _| !users_to_remove.contains(user));
        self.user_muted_by
            .retain(|user, _| !users_to_remove.contains(user));
        self.mute_list_created_at
            .retain(|user, _| !users_to_remove.contains(user));
        self.ids.retain_ids(|user| !users_to_remove.contains(&user));

        for (owner, followed_users) in &mut self.followed_by_user {
            let before = followed_users.len();
            followed_users.retain(|user| !users_to_remove.contains(user));
            if followed_users.len() != before
                && let Some(revision) = self.list_revisions.get_mut(&(*owner, 3))
            {
                revision.complete = false;
            }
        }
        for followers in self.followers_by_user.values_mut() {
            followers.retain(|user| !users_to_remove.contains(user));
        }
        for (owner, muted_users) in &mut self.muted_by_user {
            let before = muted_users.len();
            muted_users.retain(|user| !users_to_remove.contains(user));
            if muted_users.len() != before
                && let Some(revision) = self.list_revisions.get_mut(&(*owner, 10_000))
            {
                revision.complete = false;
            }
        }
        for muters in self.user_muted_by.values_mut() {
            muters.retain(|user| !users_to_remove.contains(user));
        }
        users_to_remove.len()
    }

    pub fn export_state(&self) -> SocialGraphState {
        SocialGraphState {
            list_revisions: self
                .list_revisions
                .iter()
                .map(|((owner, kind), revision)| (*owner, *kind, revision.to_public()))
                .collect(),
            root: self.root_str.clone(),
            unique_ids: self
                .ids
                .iter()
                .map(|(id, key)| (key.to_string_key(), id))
                .collect(),
            follow_distance_by_user: self.follow_distance_by_user.iter(),
            users_by_follow_distance: self
                .users_by_follow_distance
                .iter()
                .map(|(distance, users)| (*distance, users.iter().copied().collect()))
                .collect(),
            followed_by_user: self
                .followed_by_user
                .iter()
                .map(|(user, followed)| (*user, followed.as_slice().to_vec()))
                .collect(),
            followers_by_user: self
                .followers_by_user
                .iter()
                .map(|(user, followers)| (*user, followers.as_slice().to_vec()))
                .collect(),
            follow_list_created_at: self
                .follow_list_created_at
                .iter()
                .map(|(user, created_at)| (*user, *created_at))
                .collect(),
            muted_by_user: self
                .muted_by_user
                .iter()
                .map(|(user, muted)| (*user, muted.as_slice().to_vec()))
                .collect(),
            user_muted_by: self
                .user_muted_by
                .iter()
                .map(|(user, muters)| (*user, muters.as_slice().to_vec()))
                .collect(),
            mute_list_created_at: self
                .mute_list_created_at
                .iter()
                .map(|(user, created_at)| (*user, *created_at))
                .collect(),
        }
    }

    pub fn from_state(state: SocialGraphState) -> Result<Self> {
        let (state, aliases_repaired) = aliases::canonicalize(state)?;
        let mut ids = UniqueIds::with_capacity(state.unique_ids.len());
        for (value, id) in state.unique_ids {
            ids.insert_with_id(&value, id);
        }
        let root = ids.id(&state.root)?;
        let root_str = ids.str_owned(root)?;
        let dense_hint = dense_hint_for(&ids);
        let mut follow_distance_by_user = DistanceMap::with_dense_hint(dense_hint);
        for (user, distance) in state.follow_distance_by_user {
            follow_distance_by_user.insert(user, distance);
        }
        let mut scratch = HashSet::new();
        let mut collect_edges = |rows: Vec<(u32, Vec<u32>)>| -> IndexMap<u32, EdgeSet> {
            rows.into_iter()
                .map(|(user, targets)| {
                    let len = targets.len();
                    (user, EdgeSet::from_iter_dedup(targets, len, &mut scratch))
                })
                .collect()
        };
        let mut graph = Self {
            list_revisions: state
                .list_revisions
                .into_iter()
                .map(|(owner, kind, revision)| {
                    ((owner, kind), CompactRevision::from_public(revision))
                })
                .collect(),
            root,
            root_str,
            follow_distance_by_user,
            users_by_follow_distance: state
                .users_by_follow_distance
                .into_iter()
                .map(|(distance, users)| (distance, users.into_iter().collect()))
                .collect(),
            followed_by_user: collect_edges(state.followed_by_user),
            followers_by_user: collect_edges(state.followers_by_user),
            follow_list_created_at: state.follow_list_created_at.into_iter().collect(),
            muted_by_user: collect_edges(state.muted_by_user),
            user_muted_by: collect_edges(state.user_muted_by),
            mute_list_created_at: state.mute_list_created_at.into_iter().collect(),
            ids,
        };

        if aliases_repaired {
            graph.recalculate_follow_distances();
        }
        Ok(graph)
    }

    pub fn to_binary(&self) -> Result<Vec<u8>> {
        self.to_binary_with_budget(BinaryBudget::default())
    }

    pub fn to_binary_with_budget(&self, budget: BinaryBudget) -> Result<Vec<u8>> {
        self.to_binary_with_budget_version(budget, BINARY_FORMAT_VERSION)
    }

    /// Explicit version export: v3 (legacy, omits revisions), v4 (default),
    /// or v5 (compact grouped dense IDs + sorted delta-encoded edges).
    pub fn to_binary_with_budget_version(
        &self,
        budget: BinaryBudget,
        version: u64,
    ) -> Result<Vec<u8>> {
        if !matches!(
            version,
            3 | BINARY_FORMAT_VERSION | BINARY_FORMAT_VERSION_V5
        ) {
            return Err(SocialGraphError::InvalidVersion(version));
        }
        let plan = self.plan_binary(budget)?;
        self.serialize_binary(&plan, version)
    }

    /// Produce a bounded discovery graph rooted at `root` without cloning the
    /// source graph or recalculating its global distances. Traversal follows the
    /// retained prefixes, so its working memory is bounded by the node budget.
    pub fn to_binary_for_root_with_budget(
        &self,
        root: &str,
        budget: BinaryBudget,
    ) -> Result<Vec<u8>> {
        self.to_binary_for_root_with_budget_version(root, budget, BINARY_FORMAT_VERSION)
    }

    /// Like `to_binary_for_root_with_budget`, with explicit format version (3, 4, or 5).
    pub fn to_binary_for_root_with_budget_version(
        &self,
        root: &str,
        budget: BinaryBudget,
        version: u64,
    ) -> Result<Vec<u8>> {
        if !matches!(
            version,
            3 | BINARY_FORMAT_VERSION | BINARY_FORMAT_VERSION_V5
        ) {
            return Err(SocialGraphError::InvalidVersion(version));
        }
        let Some(root) = self.ids.existing_id(root) else {
            return Self::new(root).to_binary_with_budget_version(budget, version);
        };
        let max_nodes = budget.max_nodes.filter(|n| *n > 0);
        let max_edges = budget.max_edges.filter(|n| *n > 0);
        let per_owner = budget.max_edges_per_node.filter(|n| *n > 0);
        let mut used_ids = IndexSet::new();
        let mut follow_edge_count = IndexMap::new();
        let mut mute_edge_count = IndexMap::new();
        let mut distances = IndexMap::from([(root, 0u32)]);
        let mut queue = VecDeque::from([root]);
        let mut edge_count = 0;
        'owners: while let Some(owner) = queue.pop_front() {
            let distance = distances[&owner];
            if budget.max_distance.is_some_and(|limit| distance > limit) {
                continue;
            }
            let mut owner_edges = 0;
            for (kind, lists, counts) in [
                (3, &self.followed_by_user, &mut follow_edge_count),
                (10_000, &self.muted_by_user, &mut mute_edge_count),
            ] {
                for &target in lists.get(&owner).into_iter().flatten() {
                    if per_owner.is_some_and(|limit| owner_edges >= limit) {
                        break;
                    }
                    if max_edges.is_some_and(|limit| edge_count >= limit) {
                        break 'owners;
                    }
                    if !self.ids.contains_id(target) {
                        continue;
                    }
                    let added = usize::from(!used_ids.contains(&owner))
                        + usize::from(!used_ids.contains(&target));
                    if max_nodes.is_some_and(|limit| used_ids.len() + added > limit) {
                        break 'owners;
                    }
                    used_ids.insert(owner);
                    used_ids.insert(target);
                    *counts.entry(owner).or_insert(0) += 1;
                    edge_count += 1;
                    owner_edges += 1;
                    if kind == 3 && !distances.contains_key(&target) {
                        distances.insert(target, distance.saturating_add(1));
                        queue.push_back(target);
                    }
                }
            }
        }
        self.retain_empty_lists(
            root,
            |owner| distances.get(&owner).copied(),
            budget,
            max_nodes,
            &mut used_ids,
            &mut follow_edge_count,
            &mut mute_edge_count,
        );
        let follow_owners = follow_edge_count.keys().copied().collect();
        let mute_owners = mute_edge_count.keys().copied().collect();
        self.serialize_binary(
            &BinaryPlan {
                used_ids,
                follow_edge_count,
                mute_edge_count,
                follow_owners,
                mute_owners,
            },
            version,
        )
    }

    pub fn to_binary_chunks(&self) -> Result<Vec<Vec<u8>>> {
        self.to_binary_chunks_with_budget(BinaryBudget::default())
    }

    pub fn to_binary_chunks_with_budget(&self, budget: BinaryBudget) -> Result<Vec<Vec<u8>>> {
        let binary = self.to_binary_with_budget(budget)?;
        Ok(binary
            .chunks(BINARY_CHUNK_SIZE)
            .map(|chunk| chunk.to_vec())
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    fn retain_empty_lists(
        &self,
        root: u32,
        distance_of: impl Fn(u32) -> Option<u32>,
        budget: BinaryBudget,
        max_nodes: Option<usize>,
        used_ids: &mut IndexSet<u32>,
        follow_edge_count: &mut IndexMap<u32, usize>,
        mute_edge_count: &mut IndexMap<u32, usize>,
    ) {
        // Retain explicit empty facts for selected, in-range owners and an
        // isolated root. A nonempty list truncated by a budget is not empty.
        let mut empty_candidates: Vec<u32> = used_ids.iter().copied().collect();
        if !used_ids.contains(&root) {
            empty_candidates.push(root);
        }
        for owner in empty_candidates {
            let Some(distance) = distance_of(owner) else {
                continue;
            };
            if budget.max_distance.is_some_and(|limit| distance > limit)
                || (!used_ids.contains(&owner)
                    && max_nodes.is_some_and(|limit| used_ids.len() >= limit))
            {
                continue;
            }
            for (lists, timestamps, counts) in [
                (
                    &self.followed_by_user,
                    &self.follow_list_created_at,
                    &mut *follow_edge_count,
                ),
                (
                    &self.muted_by_user,
                    &self.mute_list_created_at,
                    &mut *mute_edge_count,
                ),
            ] {
                if lists.get(&owner).is_none_or(EdgeSet::is_empty)
                    && (lists.contains_key(&owner) || timestamps.contains_key(&owner))
                {
                    used_ids.insert(owner);
                    counts.insert(owner, 0);
                }
            }
        }
    }

    fn write_list_revision(
        &self,
        out: &mut Vec<u8>,
        owner: u32,
        kind: u32,
        timestamp: u64,
        complete: bool,
    ) {
        match self
            .list_revisions
            .get(&(owner, kind))
            .filter(|r| r.created_at == timestamp)
            .and_then(|r| r.as_hex32().map(|bytes| (r.complete, bytes)))
        {
            Some((rev_complete, bytes)) => {
                out.push(if rev_complete && complete { 2 } else { 1 });
                out.extend_from_slice(bytes);
            }
            None => out.push(0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_edge_lists(
        &self,
        out: &mut Vec<u8>,
        version: u64,
        kind: u32,
        owners: &[u32],
        edge_counts: &IndexMap<u32, usize>,
        lists: &IndexMap<u32, EdgeSet>,
        timestamps: &IndexMap<u32, u64>,
    ) {
        write_varint(out, owners.len() as u64);
        for &owner in owners {
            let limit = edge_counts[&owner];
            let ts = timestamps.get(&owner).copied().unwrap_or(0);
            let targets = lists.get(&owner);
            write_varint(out, owner as u64);
            write_varint(out, ts);
            if version >= 4 {
                self.write_list_revision(
                    out,
                    owner,
                    kind,
                    ts,
                    limit == targets.map_or(0, EdgeSet::len),
                );
            }
            write_varint(out, limit as u64);
            if let Some(targets) = targets {
                for &target in targets.iter().take(limit) {
                    write_varint(out, target as u64);
                }
            }
        }
    }

    fn serialize_binary(&self, plan: &BinaryPlan, version: u64) -> Result<Vec<u8>> {
        if version == BINARY_FORMAT_VERSION_V5 {
            return self.serialize_binary_v5(plan);
        }
        let BinaryPlan {
            used_ids,
            follow_edge_count,
            mute_edge_count,
            follow_owners,
            mute_owners,
        } = plan;

        let total_edges: usize =
            follow_edge_count.values().sum::<usize>() + mute_edge_count.values().sum::<usize>();
        let owner_overhead =
            (follow_owners.len() + mute_owners.len()) * if version >= 4 { 40 } else { 8 };
        let estimated = used_ids
            .len()
            .saturating_mul(36)
            .saturating_add(total_edges.saturating_mul(3))
            .saturating_add(owner_overhead)
            .saturating_add(64);
        let mut out = Vec::with_capacity(estimated);

        write_varint(&mut out, version);
        write_varint(&mut out, used_ids.len() as u64);
        for &id in used_ids {
            write_varint(&mut out, id as u64);
            self.ids.key(id)?.write_typed(&mut out);
        }

        self.write_edge_lists(
            &mut out,
            version,
            3,
            follow_owners,
            follow_edge_count,
            &self.followed_by_user,
            &self.follow_list_created_at,
        );
        self.write_edge_lists(
            &mut out,
            version,
            10_000,
            mute_owners,
            mute_edge_count,
            &self.muted_by_user,
            &self.mute_list_created_at,
        );

        Ok(out)
    }

    /// v5 wire format:
    /// - Header: varint `5`
    /// - Grouped dense identity table (implicit IDs `0..N-1`), ordered within
    ///   each group by `(exported_reference_freq DESC, canonical_bytes ASC)`:
    ///   - `pubkey_count` (varint) + `pubkey_count * 32` raw bytes
    ///   - `uuid_count` (varint) + `uuid_count * 16` raw bytes
    ///   - `string_count` (varint) + repeated (`byte_len` varint + UTF-8 bytes)
    /// - Follow lists (kind 3), then Mute lists (kind 10000):
    ///   - `lists_count` (varint)
    ///   - Sorted by strictly ascending dense `owner` ID:
    ///     - `owner_delta` (`owner_0` for i=0, else `owner_i - owner_{i-1} - 1`)
    ///     - `timestamp` (varint)
    ///     - v4 revision metadata (`flag` byte + optional 32-byte event ID)
    ///     - `edge_count` (varint)
    ///     - Targets sorted by strictly ascending dense ID, gap-delta encoded
    ///       (`t_0` for j=0, else `t_j - t_{j-1} - 1`).
    fn serialize_binary_v5(&self, plan: &BinaryPlan) -> Result<Vec<u8>> {
        let BinaryPlan {
            used_ids,
            follow_edge_count,
            mute_edge_count,
            follow_owners,
            mute_owners,
        } = plan;

        let max_id = self
            .ids
            .max_id()
            .map_or(0, |m| (m as usize).saturating_add(1));
        let use_dense = max_id > 0
            && max_id <= 4_000_000
            && max_id <= self.ids.len().saturating_mul(8).saturating_add(65_536);

        let mut dense_freq = if use_dense {
            vec![0u32; max_id]
        } else {
            Vec::new()
        };
        let mut sparse_freq = if use_dense {
            HashMap::new()
        } else {
            HashMap::<u32, u32>::with_capacity(used_ids.len())
        };
        for (counts, lists) in [
            (follow_edge_count, &self.followed_by_user),
            (mute_edge_count, &self.muted_by_user),
        ] {
            for (&owner, &limit) in counts {
                if let Some(slot) = dense_freq.get_mut(owner as usize) {
                    *slot += 1;
                } else {
                    *sparse_freq.entry(owner).or_insert(0) += 1;
                }
                if limit > 0
                    && let Some(targets) = lists.get(&owner)
                {
                    for &target in targets.iter().take(limit) {
                        if let Some(slot) = dense_freq.get_mut(target as usize) {
                            *slot += 1;
                        } else {
                            *sparse_freq.entry(target).or_insert(0) += 1;
                        }
                    }
                }
            }
        }

        let mut pubkeys: Vec<(u32, u32, [u8; 32])> = Vec::with_capacity(used_ids.len());
        let mut uuids: Vec<(u32, u32, [u8; 16])> = Vec::new();
        let mut strings: Vec<(u32, u32, &str)> = Vec::new();
        for &id in used_ids {
            let count = dense_freq
                .get(id as usize)
                .copied()
                .unwrap_or_else(|| sparse_freq.get(&id).copied().unwrap_or(0));
            match self.ids.key(id)? {
                NodeKey::Pubkey(bytes) => pubkeys.push((id, count, *bytes)),
                NodeKey::Uuid(bytes) => uuids.push((id, count, *bytes)),
                NodeKey::String(s) => strings.push((id, count, s.as_ref())),
            }
        }
        drop(dense_freq);
        drop(sparse_freq);
        pubkeys.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.2.cmp(&b.2)));
        uuids.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.2.cmp(&b.2)));
        strings.sort_unstable_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.2.as_bytes().cmp(b.2.as_bytes()))
        });

        let mut dense_map = if use_dense {
            vec![u32::MAX; max_id]
        } else {
            Vec::new()
        };
        let mut sparse_map = if use_dense {
            HashMap::new()
        } else {
            HashMap::<u32, u32>::with_capacity(used_ids.len())
        };
        for (next_id, old_id) in pubkeys
            .iter()
            .map(|&(id, _, _)| id)
            .chain(uuids.iter().map(|&(id, _, _)| id))
            .chain(strings.iter().map(|&(id, _, _)| id))
            .enumerate()
        {
            let next_id = next_id as u32;
            if let Some(slot) = dense_map.get_mut(old_id as usize) {
                *slot = next_id;
            } else {
                sparse_map.insert(old_id, next_id);
            }
        }
        let map_id = |old_id: u32| -> u32 {
            dense_map
                .get(old_id as usize)
                .copied()
                .filter(|&v| v != u32::MAX)
                .unwrap_or_else(|| sparse_map[&old_id])
        };

        let total_edges: usize =
            follow_edge_count.values().sum::<usize>() + mute_edge_count.values().sum::<usize>();
        let estimated = pubkeys
            .len()
            .saturating_mul(32)
            .saturating_add(uuids.len().saturating_mul(16))
            .saturating_add(total_edges.saturating_mul(2))
            .saturating_add((follow_owners.len() + mute_owners.len()).saturating_mul(38))
            .saturating_add(64);
        let mut out = Vec::with_capacity(estimated);

        write_varint(&mut out, BINARY_FORMAT_VERSION_V5);
        write_varint(&mut out, pubkeys.len() as u64);
        for (_, _, bytes) in &pubkeys {
            out.extend_from_slice(bytes);
        }
        write_varint(&mut out, uuids.len() as u64);
        for (_, _, bytes) in &uuids {
            out.extend_from_slice(bytes);
        }
        write_varint(&mut out, strings.len() as u64);
        for (_, _, s) in &strings {
            write_varint(&mut out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }

        let mut mapped_owners = Vec::new();
        let mut mapped_targets = Vec::new();
        for (kind, owners, counts, lists, timestamps) in [
            (
                3u32,
                follow_owners,
                follow_edge_count,
                &self.followed_by_user,
                &self.follow_list_created_at,
            ),
            (
                10_000u32,
                mute_owners,
                mute_edge_count,
                &self.muted_by_user,
                &self.mute_list_created_at,
            ),
        ] {
            mapped_owners.clear();
            mapped_owners.reserve(owners.len());
            for &old_owner in owners {
                mapped_owners.push((map_id(old_owner), old_owner));
            }
            mapped_owners.sort_unstable_by_key(|&(new_owner, _)| new_owner);

            write_varint(&mut out, mapped_owners.len() as u64);
            let mut prev_owner = 0u32;
            for (idx, &(new_owner, old_owner)) in mapped_owners.iter().enumerate() {
                let owner_delta = if idx == 0 {
                    new_owner
                } else {
                    new_owner - prev_owner - 1
                };
                prev_owner = new_owner;
                write_varint(&mut out, owner_delta as u64);

                let limit = counts[&old_owner];
                let ts = timestamps.get(&old_owner).copied().unwrap_or(0);
                let targets = lists.get(&old_owner);
                write_varint(&mut out, ts);
                self.write_list_revision(
                    &mut out,
                    old_owner,
                    kind,
                    ts,
                    limit == targets.map_or(0, EdgeSet::len),
                );
                write_varint(&mut out, limit as u64);
                if limit > 0
                    && let Some(targets) = targets
                {
                    mapped_targets.clear();
                    mapped_targets.reserve(limit);
                    for &old_target in targets.iter().take(limit) {
                        mapped_targets.push(map_id(old_target));
                    }
                    mapped_targets.sort_unstable();
                    let mut prev_target = 0u32;
                    for (j, &new_target) in mapped_targets.iter().enumerate() {
                        let target_delta = if j == 0 {
                            new_target
                        } else {
                            new_target - prev_target - 1
                        };
                        prev_target = new_target;
                        write_varint(&mut out, target_delta as u64);
                    }
                }
            }
        }

        Ok(out)
    }

    fn plan_binary(&self, budget: BinaryBudget) -> Result<BinaryPlan> {
        if !budget.has_active_limits() {
            return Ok(self.plan_full_binary());
        }

        let max_nodes = budget.max_nodes.filter(|value| *value > 0);
        let max_edges = budget.max_edges.filter(|value| *value > 0);
        let max_edges_per_node = budget.max_edges_per_node.filter(|value| *value > 0);

        let mut distances: Vec<u32> = self.users_by_follow_distance.keys().copied().collect();
        distances.sort_unstable();

        let cap = max_nodes.unwrap_or(self.ids.len()).min(self.ids.len());
        let mut used_ids = IndexSet::with_capacity(cap);
        let mut follow_edge_count = IndexMap::new();
        let mut mute_edge_count = IndexMap::new();
        let mut edge_count = 0usize;

        // Stream directly in distance, owner, follow/mute order and stop as soon
        // as the export budget is reached instead of materializing all graph edges.
        'budget: for distance in distances {
            if budget
                .max_distance
                .is_some_and(|max_distance| distance > max_distance)
            {
                break;
            }
            let Some(users) = self.users_by_follow_distance.get(&distance) else {
                continue;
            };

            'owner: for &owner in users {
                let mut owner_edge_count = 0usize;
                let owner_valid = self.ids.contains_id(owner);
                for (lists, counts) in [
                    (&self.followed_by_user, &mut follow_edge_count),
                    (&self.muted_by_user, &mut mute_edge_count),
                ] {
                    let Some(targets) = lists.get(&owner) else {
                        continue;
                    };
                    for &target in targets {
                        if max_edges_per_node.is_some_and(|limit| owner_edge_count >= limit) {
                            continue 'owner;
                        }
                        owner_edge_count += 1;
                        if max_edges.is_some_and(|limit| edge_count >= limit) {
                            break 'budget;
                        }
                        if !owner_valid || !self.ids.contains_id(target) {
                            continue;
                        }
                        if let Some(limit) = max_nodes {
                            let added = usize::from(!used_ids.contains(&owner))
                                + usize::from(!used_ids.contains(&target));
                            if used_ids.len() + added > limit {
                                break 'budget;
                            }
                        }
                        used_ids.insert(owner);
                        used_ids.insert(target);
                        edge_count += 1;
                        *counts.entry(owner).or_insert(0) += 1;
                    }
                }
            }
        }

        self.retain_empty_lists(
            self.root,
            |owner| self.follow_distance_by_user.get(owner),
            budget,
            max_nodes,
            &mut used_ids,
            &mut follow_edge_count,
            &mut mute_edge_count,
        );

        let follow_owners = follow_edge_count.keys().copied().collect();
        let mute_owners = mute_edge_count.keys().copied().collect();
        Ok(BinaryPlan {
            used_ids,
            follow_edge_count,
            mute_edge_count,
            follow_owners,
            mute_owners,
        })
    }

    fn plan_full_binary(&self) -> BinaryPlan {
        let mut used_ids = IndexSet::with_capacity(self.ids.len());
        let mut follow_edge_count = IndexMap::with_capacity(self.followed_by_user.len());
        let mut mute_edge_count = IndexMap::with_capacity(self.muted_by_user.len());

        for (lists, timestamps, counts) in [
            (
                &self.followed_by_user,
                &self.follow_list_created_at,
                &mut follow_edge_count,
            ),
            (
                &self.muted_by_user,
                &self.mute_list_created_at,
                &mut mute_edge_count,
            ),
        ] {
            for (&user, targets) in lists {
                used_ids.insert(user);
                counts.insert(user, targets.len());
                for &target in targets {
                    used_ids.insert(target);
                }
            }
            // First-empty events may have a timestamp without an adjacency row.
            for &user in timestamps.keys() {
                if !counts.contains_key(&user) {
                    used_ids.insert(user);
                    counts.insert(user, 0);
                }
            }
        }

        let follow_owners = follow_edge_count.keys().copied().collect();
        let mute_owners = mute_edge_count.keys().copied().collect();

        BinaryPlan {
            used_ids,
            follow_edge_count,
            mute_edge_count,
            follow_owners,
            mute_owners,
        }
    }

    pub fn from_binary(root: &str, data: &[u8]) -> Result<Self> {
        let mut offset = 0usize;
        let version = read_varint(data, &mut offset)?;
        if !matches!(
            version,
            BINARY_FORMAT_VERSION_V5 | BINARY_FORMAT_VERSION | 3 | BINARY_FORMAT_VERSION_V2
        ) {
            return Err(SocialGraphError::InvalidVersion(version));
        }

        if version == BINARY_FORMAT_VERSION_V5 {
            let mut ids = read_v5_identities(data, &mut offset, None, None)?;
            let mut list_revisions = IndexMap::new();
            let (followed_by_user, followers_by_user, follow_list_created_at) =
                read_v5_lists(data, &mut offset, 3, &ids, &mut list_revisions, None)?;
            let (muted_by_user, user_muted_by, mute_list_created_at) =
                read_v5_lists(data, &mut offset, 10_000, &ids, &mut list_revisions, None)?;
            let root_id = ids.id(root)?;
            let root_str = ids.str_owned(root_id)?;
            let mut graph = Self {
                list_revisions,
                root: root_id,
                root_str,
                follow_distance_by_user: DistanceMap::default(),
                users_by_follow_distance: IndexMap::new(),
                followed_by_user,
                followers_by_user,
                follow_list_created_at,
                muted_by_user,
                user_muted_by,
                mute_list_created_at,
                ids,
            };
            graph.recalculate_follow_distances();
            return Ok(graph);
        }

        let ids_count = read_count(data, &mut offset)?;
        let mut ids = UniqueIds::with_capacity(ids_count);
        let mut aliased_ids: Option<Vec<(String, u32)>> = None;

        for _ in 0..ids_count {
            let (key, id) = read_binary_node_key(version, data, &mut offset)?;
            if let Some(raw_ids) = &mut aliased_ids {
                raw_ids.push((key.to_string_key(), id));
            } else if !ids.try_insert_unique_key(key.clone(), id) {
                let mut raw_ids = Vec::with_capacity(ids_count);
                for (eid, ekey) in ids.iter() {
                    raw_ids.push((ekey.to_string_key(), eid));
                }
                raw_ids.push((key.to_string_key(), id));
                aliased_ids = Some(raw_ids);
            }
        }

        if let Some(unique_ids) = aliased_ids {
            // Legacy aliased snapshot: decode tuples and canonicalize through `from_state`.
            let mut revisions = IndexMap::new();
            let mut read_raw = |kind: u32| -> Result<RawLists> {
                let count = read_count(data, &mut offset)?;
                let mut lists = Vec::with_capacity(count);
                let mut timestamps = Vec::with_capacity(count);
                for _ in 0..count {
                    let user = read_varint(data, &mut offset)? as u32;
                    let timestamp = read_varint(data, &mut offset)?;
                    if version >= 4
                        && let Some(revision) = read_list_revision(data, &mut offset, timestamp)?
                    {
                        revisions.insert((user, kind), revision.to_public());
                    }
                    let edge_count = read_count(data, &mut offset)?;
                    let mut targets = Vec::with_capacity(edge_count);
                    for _ in 0..edge_count {
                        targets.push(read_varint(data, &mut offset)? as u32);
                    }
                    lists.push((user, targets));
                    timestamps.push((user, timestamp));
                }
                Ok((lists, timestamps))
            };
            let (followed_by_user, follow_list_created_at) = read_raw(3)?;
            let (muted_by_user, mute_list_created_at) = read_raw(10_000)?;
            let state = SocialGraphState {
                root: root.to_owned(),
                unique_ids,
                list_revisions: revisions
                    .into_iter()
                    .map(|((owner, kind), r)| (owner, kind, r))
                    .collect(),
                followed_by_user,
                follow_list_created_at,
                muted_by_user,
                mute_list_created_at,
                followers_by_user: Vec::new(),
                user_muted_by: Vec::new(),
                follow_distance_by_user: Vec::new(),
                users_by_follow_distance: Vec::new(),
            };
            let mut graph = Self::from_state(state)?;
            if graph.followers_by_user.is_empty() {
                build_reverse_index(
                    &graph.followed_by_user,
                    &graph.ids,
                    &mut graph.followers_by_user,
                );
            }
            if graph.user_muted_by.is_empty() {
                build_reverse_index(&graph.muted_by_user, &graph.ids, &mut graph.user_muted_by);
            }
            if graph.follow_distance_by_user.is_empty() {
                graph.recalculate_follow_distances();
            }
            return Ok(graph);
        }

        // Alias-free fast path: decode forward lists into compact EdgeSets,
        // then build reverse indexes in a 2-pass exact-capacity sweep.
        let mut list_revisions = IndexMap::new();
        let mut seen_targets = HashSet::new();
        let mut read_direct = |kind: u32| -> Result<DecodedLists> {
            let count = read_count(data, &mut offset)?;
            let mut forward: IndexMap<u32, EdgeSet> = IndexMap::with_capacity(count);
            let mut timestamps: IndexMap<u32, u64> = IndexMap::with_capacity(count);
            for _ in 0..count {
                let owner = read_varint(data, &mut offset)? as u32;
                let timestamp = read_varint(data, &mut offset)?;
                if version >= 4
                    && let Some(revision) = read_list_revision(data, &mut offset, timestamp)?
                {
                    list_revisions.insert((owner, kind), revision);
                }
                let edge_count = read_count(data, &mut offset)?;
                let mut targets = Vec::with_capacity(edge_count);
                if edge_count <= 32 {
                    for _ in 0..edge_count {
                        let target = read_varint(data, &mut offset)? as u32;
                        if !targets.contains(&target) {
                            targets.push(target);
                        }
                    }
                } else {
                    seen_targets.clear();
                    for _ in 0..edge_count {
                        let target = read_varint(data, &mut offset)? as u32;
                        if seen_targets.insert(target) {
                            targets.push(target);
                        }
                    }
                }
                forward.insert(owner, EdgeSet::from_vec_exact(targets));
                timestamps.insert(owner, timestamp);
            }
            let mut reverse = IndexMap::new();
            build_reverse_index(&forward, &ids, &mut reverse);
            Ok((forward, reverse, timestamps))
        };

        let (followed_by_user, followers_by_user, follow_list_created_at) = read_direct(3)?;
        let (muted_by_user, user_muted_by, mute_list_created_at) = read_direct(10_000)?;
        let root_id = ids.id(root)?;
        let root_str = ids.str_owned(root_id)?;

        let mut graph = Self {
            list_revisions,
            root: root_id,
            root_str,
            follow_distance_by_user: DistanceMap::default(),
            users_by_follow_distance: IndexMap::new(),
            followed_by_user,
            followers_by_user,
            follow_list_created_at,
            muted_by_user,
            user_muted_by,
            mute_list_created_at,
            ids,
        };
        graph.recalculate_follow_distances();
        Ok(graph)
    }

    pub fn is_overmuted(&self, user: &str, threshold: f64) -> bool {
        self.ids
            .existing_id(user)
            .is_some_and(|user_id| self.is_overmuted_id(user_id, threshold))
    }

    fn is_overmuted_id(&self, user_id: u32, threshold: f64) -> bool {
        if user_id == self.root {
            return false;
        }
        let Some(muters) = self.user_muted_by.get(&user_id) else {
            return false;
        };
        if muters.is_empty() {
            return false;
        }
        if muters.contains(self.root) {
            return true;
        }

        let mut min_dist = u32::MAX;
        let mut muters_at_min = 0u32;
        for &muter in muters {
            if let Some(distance) = self.follow_distance_by_user.get(muter) {
                if distance < min_dist {
                    min_dist = distance;
                    muters_at_min = 1;
                } else if distance == min_dist {
                    muters_at_min += 1;
                }
            }
        }
        let max_followers = (muters_at_min as f64) * threshold;
        if muters_at_min == 0 || max_followers.is_nan() || max_followers <= 0.0 {
            return false;
        }

        if let Some(followers) = self.followers_by_user.get(&user_id) {
            let mut followers_at_min = 0u32;
            for &follower in followers {
                if let Some(distance) = self.follow_distance_by_user.get(follower) {
                    if distance < min_dist {
                        return false;
                    }
                    if distance == min_dist {
                        followers_at_min += 1;
                        if (followers_at_min as f64) >= max_followers {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    fn accept_and_record_revision(
        &mut self,
        author: u32,
        kind: u32,
        created_at: u64,
        id: &str,
    ) -> bool {
        let existing_at = if kind == 3 {
            self.follow_list_created_at.get(&author).copied()
        } else {
            self.mute_list_created_at.get(&author).copied()
        };
        if !accepts_compact_revision(
            existing_at,
            self.list_revisions.get(&(author, kind)),
            created_at,
            id,
        ) {
            return false;
        }
        if valid_event_id(id) {
            self.list_revisions.insert(
                (author, kind),
                CompactRevision::from_valid_hex(id, created_at, true),
            );
        } else {
            self.list_revisions.shift_remove(&(author, kind));
        }
        if kind == 3 {
            self.follow_list_created_at.insert(author, created_at);
        } else {
            self.mute_list_created_at.insert(author, created_at);
        }
        true
    }

    fn collect_pubkey_tags(&mut self, author: u32, tags: &[Vec<String>]) -> Vec<u32> {
        let mut targets = Vec::with_capacity(tags.len());
        let mut seen = (tags.len() > 32).then(|| HashSet::with_capacity(tags.len()));
        for tag in tags {
            if tag.first().is_some_and(|value| value == "p")
                && let Some(pk) = tag.get(1)
                && is_valid_pubkey(pk)
                && let Ok(target) = self.ids.id(pk)
                && target != author
            {
                let inserted = match &mut seen {
                    Some(set) => set.insert(target),
                    None => !targets.contains(&target),
                };
                if inserted {
                    targets.push(target);
                }
            }
        }
        targets
    }

    fn handle_follow_list(&mut self, author: u32, created_at: u64, id: &str, tags: &[Vec<String>]) {
        if !self.accept_and_record_revision(author, 3, created_at, id) {
            return;
        }
        let incoming = self.collect_pubkey_tags(author, tags);
        let Some((removed, added)) = replace_edge_set(&mut self.followed_by_user, author, incoming)
        else {
            return;
        };
        update_reverse_edges(&mut self.followers_by_user, author, &removed, &added);

        // Batch distance updates for all edges changed by this event so each
        // distance bucket is compacted at most once instead of once per edge.
        if let Some(author_dist) = self.follow_distance_by_user.get(author) {
            if !removed.is_empty() {
                let lost_shortest: Vec<u32> = removed
                    .into_iter()
                    .filter(|&u| {
                        self.follow_distance_by_user
                            .get(u)
                            .is_some_and(|d| d > 0 && d - 1 == author_dist)
                    })
                    .collect();
                if !lost_shortest.is_empty() {
                    self.raise_distances(&lost_shortest);
                }
            }
            if !added.is_empty() {
                let mut moved = Moved::new();
                self.lower_distances_into(&added, author_dist.saturating_add(1), &mut moved);
                if !moved.is_empty() {
                    self.rebucket(moved);
                }
            }
        }
    }

    fn handle_mute_list(&mut self, author: u32, created_at: u64, id: &str, tags: &[Vec<String>]) {
        if !self.accept_and_record_revision(author, 10_000, created_at, id) {
            return;
        }
        let incoming = self.collect_pubkey_tags(author, tags);
        let Some((removed, added)) = replace_edge_set(&mut self.muted_by_user, author, incoming)
        else {
            return;
        };
        update_reverse_edges(&mut self.user_muted_by, author, &removed, &added);
    }

    fn private_add_follower(&mut self, followed_user: u32, follower: u32) {
        self.followed_by_user
            .entry(follower)
            .or_default()
            .insert(followed_user);
        self.followers_by_user
            .entry(followed_user)
            .or_default()
            .insert(follower);

        // A new edge can only shorten paths, and only from a reachable follower.
        if let Some(distance) = self.follow_distance_by_user.get(follower) {
            let mut moved = Moved::new();
            self.lower_distances_into(&[followed_user], distance.saturating_add(1), &mut moved);
            if !moved.is_empty() {
                self.rebucket(moved);
            }
        }
    }

    /// Brings every seed in `starts` to `distance` if closer, then everyone
    /// reached through them. Buckets are settled afterwards by `rebucket`.
    fn lower_distances_into(&mut self, starts: &[u32], distance: u32, moved: &mut Moved) {
        let mut queue = VecDeque::new();
        for &start in starts {
            let current = self.follow_distance_by_user.get(start);
            if current.is_none_or(|current| current > distance) {
                moved.entry(start).or_insert(current);
                self.follow_distance_by_user.insert(start, distance);
                queue.push_back(start);
            }
        }
        while let Some(user) = queue.pop_front() {
            let next = self
                .follow_distance_by_user
                .get(user)
                .expect("queued user distance")
                .saturating_add(1);
            for &target in self.followed_by_user.get(&user).into_iter().flatten() {
                let known = self.follow_distance_by_user.get(target);
                if known.is_none_or(|known| known > next) {
                    moved.entry(target).or_insert(known);
                    self.follow_distance_by_user.insert(target, next);
                    queue.push_back(target);
                }
            }
        }
    }

    /// `seeds` all shared the same shortest-path distance and just lost a
    /// shortest-path incoming edge. Users whose every shortest path ran through
    /// those edges move farther away or become unreachable in one batched pass.
    fn raise_distances(&mut self, seeds: &[u32]) {
        let mut moved = Moved::new();
        for &user in seeds {
            let Some(distance) = self.follow_distance_by_user.get(user) else {
                continue;
            };
            if distance == DETACHED_DISTANCE || self.has_follower_at(user, distance - 1) {
                continue;
            }
            moved.insert(user, Some(distance));
            self.follow_distance_by_user.insert(user, DETACHED_DISTANCE);
        }
        if moved.is_empty() {
            return;
        }
        // Orphans are marked in place, closest first, so that they no longer
        // count as a closer follower of anyone checked after them.
        let mut index = 0;
        while let Some((&orphan, &was)) = moved.get_index(index) {
            index += 1;
            let next = was.unwrap_or(DETACHED_DISTANCE).saturating_add(1);
            let orphaned: Vec<u32> = self
                .followed_by_user
                .get(&orphan)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&target| {
                    self.follow_distance_by_user.get(target) == Some(next)
                        && !self.has_follower_at(target, next - 1)
                })
                .collect();
            for target in orphaned {
                moved.insert(target, Some(next));
                self.follow_distance_by_user
                    .insert(target, DETACHED_DISTANCE);
            }
        }

        // Reattach through the nearest follower that is still reachable.
        for index in 0..moved.len() {
            let orphan = *moved.get_index(index).expect("orphan index").0;
            let nearest = self
                .followers_by_user
                .get(&orphan)
                .into_iter()
                .flatten()
                .filter_map(|&follower| self.follow_distance_by_user.get(follower))
                .filter(|distance| *distance != DETACHED_DISTANCE)
                .min();
            if let Some(nearest) = nearest {
                self.lower_distances_into(&[orphan], nearest.saturating_add(1), &mut moved);
            }
        }
        self.rebucket(moved);
    }

    fn has_follower_at(&self, user: u32, distance: u32) -> bool {
        self.followers_by_user.get(&user).is_some_and(|followers| {
            followers
                .iter()
                .any(|&follower| self.follow_distance_by_user.get(follower) == Some(distance))
        })
    }

    /// Moves users whose distance changed into their new buckets and drops the
    /// ones left detached. Several users leaving one collection take a single
    /// stable pass rather than one compaction each.
    fn rebucket(&mut self, moved: Moved) {
        let mut leaving: IndexMap<u32, Vec<u32>> = IndexMap::new();
        let mut detached = Vec::new();
        for (user, old) in moved {
            let new = self
                .follow_distance_by_user
                .get(user)
                .filter(|distance| *distance != DETACHED_DISTANCE);
            if new.is_none() {
                detached.push(user);
            }
            if new == old {
                continue;
            }
            if let Some(old) = old {
                leaving.entry(old).or_default().push(user);
            }
            if let Some(new) = new {
                self.users_by_follow_distance
                    .entry(new)
                    .or_default()
                    .insert(user);
            }
        }
        for (distance, users) in leaving {
            let Some(bucket) = self.users_by_follow_distance.get_mut(&distance) else {
                continue;
            };
            if let [user] = users[..] {
                bucket.shift_remove(&user);
            } else {
                let users: HashSet<u32> = users.into_iter().collect();
                bucket.retain(|user| !users.contains(user));
            }
            if bucket.is_empty() {
                self.users_by_follow_distance.shift_remove(&distance);
            }
        }
        for user in detached {
            self.follow_distance_by_user.remove(user);
        }
    }
}

impl SocialGraphBackend for SocialGraph {
    type Error = SocialGraphError;
    fn get_list_revisions(
        &self,
    ) -> std::result::Result<Vec<(String, u32, ListRevision)>, Self::Error> {
        Ok(SocialGraph::get_list_revisions(self))
    }

    fn get_root(&self) -> std::result::Result<String, Self::Error> {
        Ok(SocialGraph::get_root(self).to_string())
    }

    fn set_root(&mut self, root: &str) -> std::result::Result<(), Self::Error> {
        SocialGraph::set_root(self, root)
    }

    fn handle_event(
        &mut self,
        event: &NostrEvent,
        allow_unknown_authors: bool,
        overmute_threshold: f64,
    ) -> std::result::Result<(), Self::Error> {
        SocialGraph::handle_event(self, event, allow_unknown_authors, overmute_threshold);
        Ok(())
    }

    fn get_follow_distance(&self, user: &str) -> std::result::Result<u32, Self::Error> {
        Ok(SocialGraph::get_follow_distance(self, user))
    }

    fn is_following(
        &self,
        follower: &str,
        followed_user: &str,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(SocialGraph::is_following(self, follower, followed_user))
    }

    fn get_followed_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(SocialGraph::get_followed_by_user(self, user))
    }

    fn get_followers_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(SocialGraph::get_followers_by_user(self, user))
    }

    fn get_muted_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(SocialGraph::get_muted_by_user(self, user))
    }

    fn get_user_muted_by(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        Ok(SocialGraph::get_user_muted_by(self, user))
    }

    fn get_follow_list_created_at(
        &self,
        user: &str,
    ) -> std::result::Result<Option<u64>, Self::Error> {
        Ok(SocialGraph::get_follow_list_created_at(self, user))
    }

    fn get_mute_list_created_at(
        &self,
        user: &str,
    ) -> std::result::Result<Option<u64>, Self::Error> {
        Ok(SocialGraph::get_mute_list_created_at(self, user))
    }

    fn is_overmuted(&self, user: &str, threshold: f64) -> std::result::Result<bool, Self::Error> {
        Ok(SocialGraph::is_overmuted(self, user, threshold))
    }
}

fn replace_edge_set(
    forward: &mut IndexMap<u32, EdgeSet>,
    author: u32,
    incoming: Vec<u32>,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    if let Some(existing) = forward.get_mut(&author) {
        if existing.len() <= 16 && incoming.len() <= 16 {
            if existing.len() == incoming.len() && existing.iter().all(|u| incoming.contains(u)) {
                return None;
            }
            existing.retain(|u| {
                if incoming.contains(u) {
                    true
                } else {
                    removed.push(*u);
                    false
                }
            });
            for user in incoming {
                if existing.insert(user) {
                    added.push(user);
                }
            }
        } else {
            let incoming_set: HashSet<u32> = incoming.iter().copied().collect();
            if existing.len() == incoming_set.len()
                && existing.iter().all(|u| incoming_set.contains(u))
            {
                return None;
            }
            let mut next = Vec::with_capacity(incoming.len());
            let mut kept = HashSet::with_capacity(existing.len().min(incoming.len()));
            for &u in existing.as_slice() {
                if incoming_set.contains(&u) {
                    next.push(u);
                    kept.insert(u);
                } else {
                    removed.push(u);
                }
            }
            for user in incoming {
                if !kept.contains(&user) {
                    next.push(user);
                    added.push(user);
                }
            }
            *existing = EdgeSet::from_vec_exact(next);
        }
    } else if !incoming.is_empty() {
        added.extend_from_slice(&incoming);
        forward.insert(author, EdgeSet::from_vec_exact(incoming));
    } else {
        return None;
    }
    Some((removed, added))
}

fn update_reverse_edges(
    reverse: &mut IndexMap<u32, EdgeSet>,
    author: u32,
    removed: &[u32],
    added: &[u32],
) {
    for &user in removed {
        if let Some(set) = reverse.get_mut(&user) {
            set.remove(author);
        }
    }
    for &user in added {
        reverse.entry(user).or_default().insert(author);
    }
}

/// Two-pass exact-capacity reverse index builder. Avoids per-edge hashing on
/// dense/near-dense graphs and eliminates `Vec` doubling slack across millions
/// of reverse follower lists.
fn build_reverse_index(
    forward: &IndexMap<u32, EdgeSet>,
    ids: &UniqueIds,
    reverse: &mut IndexMap<u32, EdgeSet>,
) {
    reverse.clear();
    if forward.is_empty() {
        return;
    }
    let max_id = ids.max_id().map_or(0, |m| (m as usize).saturating_add(1));
    let use_dense = max_id > 0
        && max_id <= 4_000_000
        && max_id <= ids.len().saturating_mul(8).saturating_add(65_536);

    if use_dense {
        let mut counts = vec![0u32; max_id];
        let mut non_zero = 0usize;
        let mut overflow_counts = HashMap::<u32, u32>::new();
        for targets in forward.values() {
            for &target in targets {
                if let Some(slot) = counts.get_mut(target as usize) {
                    if *slot == 0 {
                        non_zero += 1;
                    }
                    *slot += 1;
                } else {
                    *overflow_counts.entry(target).or_insert(0) += 1;
                }
            }
        }
        let mut target_to_slot = vec![u32::MAX; max_id];
        let mut buckets: Vec<(u32, EdgeSet)> = Vec::with_capacity(non_zero);
        let mut overflow_reverse: IndexMap<u32, EdgeSet> =
            IndexMap::with_capacity(overflow_counts.len());
        for (&owner, targets) in forward {
            for &target in targets {
                if let Some(slot) = target_to_slot.get_mut(target as usize) {
                    let bucket_idx = if *slot == u32::MAX {
                        let idx = buckets.len() as u32;
                        *slot = idx;
                        buckets.push((
                            target,
                            EdgeSet::with_exact_capacity(counts[target as usize] as usize),
                        ));
                        idx as usize
                    } else {
                        *slot as usize
                    };
                    buckets[bucket_idx].1.push_unchecked(owner);
                } else {
                    let cap = overflow_counts.get(&target).copied().unwrap_or(1) as usize;
                    overflow_reverse
                        .entry(target)
                        .or_insert_with(|| EdgeSet::with_exact_capacity(cap))
                        .push_unchecked(owner);
                }
            }
        }
        drop(counts);
        drop(target_to_slot);
        reverse.reserve(buckets.len() + overflow_reverse.len());
        reverse.extend(buckets);
        reverse.extend(overflow_reverse);
    } else {
        let mut counts = HashMap::<u32, u32>::new();
        for targets in forward.values() {
            for &target in targets {
                *counts.entry(target).or_insert(0) += 1;
            }
        }
        reverse.reserve(counts.len());
        for (&owner, targets) in forward {
            for &target in targets {
                let cap = counts[&target] as usize;
                reverse
                    .entry(target)
                    .or_insert_with(|| EdgeSet::with_exact_capacity(cap))
                    .push_unchecked(owner);
            }
        }
    }
}

fn dense_hint_for(ids: &UniqueIds) -> usize {
    let max_exclusive = ids.max_id().map_or(0, |m| (m as usize).saturating_add(1));
    if max_exclusive > 0 && max_exclusive <= ids.len().saturating_mul(4).saturating_add(4_096) {
        max_exclusive
    } else {
        0
    }
}

pub(crate) fn is_valid_pubkey(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn is_canonical_uuid(value: &str) -> Option<Uuid> {
    let uuid = Uuid::parse_str(value).ok()?;
    (uuid.to_string() == value).then_some(uuid)
}

#[cfg(test)]
fn write_binary_node_id(out: &mut Vec<u8>, value: &str) {
    NodeKeyRef::parse(value).to_owned().write_typed(out);
}

fn read_typed_node_key(data: &[u8], offset: &mut usize) -> Result<NodeKey> {
    match read_varint(data, offset)? {
        BINARY_NODE_ID_PUBKEY => {
            let bytes: [u8; 32] = read_bytes(data, offset, 32)?
                .try_into()
                .expect("32-byte pubkey slice");
            Ok(NodeKey::Pubkey(bytes))
        }
        BINARY_NODE_ID_UUID => {
            let bytes: [u8; 16] = read_bytes(data, offset, 16)?
                .try_into()
                .expect("16-byte uuid slice");
            Ok(NodeKey::Uuid(bytes))
        }
        BINARY_NODE_ID_STRING => {
            let len = read_varint(data, offset)? as usize;
            let raw = str::from_utf8(read_bytes(data, offset, len)?)
                .map_err(|_| SocialGraphError::InvalidBinaryNodeIdUtf8)?;
            if raw.trim().is_empty() {
                return Err(SocialGraphError::EmptyString);
            }
            Ok(NodeKeyRef::parse(raw).to_owned())
        }
        other => Err(SocialGraphError::InvalidBinaryNodeIdType(other)),
    }
}

fn read_binary_node_key(version: u64, data: &[u8], offset: &mut usize) -> Result<(NodeKey, u32)> {
    if version == BINARY_FORMAT_VERSION_V2 {
        let bytes: [u8; 32] = read_bytes(data, offset, 32)?
            .try_into()
            .expect("32-byte pubkey slice");
        let id = read_varint(data, offset)? as u32;
        return Ok((NodeKey::Pubkey(bytes), id));
    }

    let id = read_varint(data, offset)? as u32;
    Ok((read_typed_node_key(data, offset)?, id))
}

fn read_v5_identities(
    data: &[u8],
    offset: &mut usize,
    max_nodes: Option<usize>,
    min_nodes: Option<usize>,
) -> Result<UniqueIds> {
    let pubkey_count = read_count(data, offset)?;
    if max_nodes.is_some_and(|max| pubkey_count > max) {
        return Err(SocialGraphError::InvalidId(u32::MAX));
    }
    let pk_bytes_len = pubkey_count
        .checked_mul(32)
        .ok_or(SocialGraphError::UnexpectedEof)?;
    let pk_slice = read_bytes(data, offset, pk_bytes_len)?;
    let mut ids = UniqueIds::with_capacity(pubkey_count);
    for &bytes in pk_slice.as_chunks::<32>().0 {
        if !ids.push_dense_unique_key(NodeKey::Pubkey(bytes)) {
            return Err(SocialGraphError::EmptyString);
        }
    }

    let uuid_count = read_count(data, offset)?;
    if max_nodes.is_some_and(|max| ids.len().saturating_add(uuid_count) > max) {
        return Err(SocialGraphError::InvalidId(u32::MAX));
    }
    let uuid_bytes_len = uuid_count
        .checked_mul(16)
        .ok_or(SocialGraphError::UnexpectedEof)?;
    let uuid_slice = read_bytes(data, offset, uuid_bytes_len)?;
    for &bytes in uuid_slice.as_chunks::<16>().0 {
        if !ids.push_dense_unique_key(NodeKey::Uuid(bytes)) {
            return Err(SocialGraphError::EmptyString);
        }
    }

    let string_count = read_count(data, offset)?;
    if max_nodes.is_some_and(|max| ids.len().saturating_add(string_count) > max) {
        return Err(SocialGraphError::InvalidId(u32::MAX));
    }
    for _ in 0..string_count {
        let len = read_varint(data, offset)? as usize;
        let raw = str::from_utf8(read_bytes(data, offset, len)?)
            .map_err(|_| SocialGraphError::InvalidBinaryNodeIdUtf8)?;
        if raw.trim().is_empty() {
            return Err(SocialGraphError::EmptyString);
        }
        if !ids.push_dense_unique_key(NodeKeyRef::parse(raw).to_owned()) {
            return Err(SocialGraphError::EmptyString);
        }
    }

    if min_nodes.is_some_and(|min| ids.len() < min) {
        return Err(SocialGraphError::InvalidId(u32::MAX));
    }
    Ok(ids)
}

fn read_v5_lists(
    data: &[u8],
    offset: &mut usize,
    kind: u32,
    ids: &UniqueIds,
    list_revisions: &mut IndexMap<(u32, u32), CompactRevision>,
    mut remaining_edges: Option<&mut usize>,
) -> Result<DecodedLists> {
    let total_nodes = ids.len();
    let count = read_count(data, offset)?;
    if count > total_nodes {
        return Err(SocialGraphError::InvalidId(u32::MAX));
    }
    let mut forward = IndexMap::with_capacity(count);
    let mut timestamps = IndexMap::with_capacity(count);
    let mut prev_owner = 0u32;
    for i in 0..count {
        let delta = read_varint(data, offset)?;
        let owner_u64 = if i == 0 {
            delta
        } else {
            u64::from(prev_owner)
                .checked_add(1)
                .and_then(|v| v.checked_add(delta))
                .ok_or(SocialGraphError::InvalidId(u32::MAX))?
        };
        if owner_u64 >= total_nodes as u64 {
            return Err(SocialGraphError::InvalidId(
                owner_u64.min(u32::MAX as u64) as u32
            ));
        }
        let owner = owner_u64 as u32;
        prev_owner = owner;

        let timestamp = read_varint(data, offset)?;
        if let Some(revision) = read_list_revision(data, offset, timestamp)? {
            list_revisions.insert((owner, kind), revision);
        }
        let edge_count = read_count(data, offset)?;
        if edge_count > total_nodes {
            return Err(SocialGraphError::InvalidId(u32::MAX));
        }
        if let Some(rem) = remaining_edges.as_deref_mut() {
            if edge_count > *rem {
                return Err(SocialGraphError::InvalidId(u32::MAX));
            }
            *rem -= edge_count;
        }
        let mut targets = Vec::with_capacity(edge_count);
        let mut prev_target = 0u32;
        for j in 0..edge_count {
            let t_delta = read_varint(data, offset)?;
            let target_u64 = if j == 0 {
                t_delta
            } else {
                u64::from(prev_target)
                    .checked_add(1)
                    .and_then(|v| v.checked_add(t_delta))
                    .ok_or(SocialGraphError::InvalidId(u32::MAX))?
            };
            if target_u64 >= total_nodes as u64 {
                return Err(SocialGraphError::InvalidId(
                    target_u64.min(u32::MAX as u64) as u32
                ));
            }
            let target = target_u64 as u32;
            prev_target = target;
            targets.push(target);
        }
        forward.insert(owner, EdgeSet::from_vec_exact(targets));
        timestamps.insert(owner, timestamp);
    }
    let mut reverse = IndexMap::new();
    build_reverse_index(&forward, ids, &mut reverse);
    Ok((forward, reverse, timestamps))
}

fn read_list_revision(
    data: &[u8],
    offset: &mut usize,
    created_at: u64,
) -> Result<Option<CompactRevision>> {
    let flag = read_bytes(data, offset, 1)?[0];
    if flag == 0 {
        return Ok(None);
    }
    if flag > 2 {
        return Err(SocialGraphError::InvalidBinaryNodeIdType(flag as u64));
    }
    let id: [u8; 32] = read_bytes(data, offset, 32)?
        .try_into()
        .expect("32-byte event id");
    Ok(Some(CompactRevision::from_bytes(id, created_at, flag == 2)))
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(((value as u8) & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push((value & 0x7f) as u8);
}

fn read_count(data: &[u8], offset: &mut usize) -> Result<usize> {
    let value = read_varint(data, offset)?;
    // Every item needs at least one encoded byte. Reject attacker-controlled
    // capacity requests before allocating even for legacy snapshots.
    if value > data.len().saturating_sub(*offset) as u64 {
        return Err(SocialGraphError::UnexpectedEof);
    }
    Ok(value as usize)
}

fn read_varint(data: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*offset).ok_or(SocialGraphError::UnexpectedEof)?;
        *offset += 1;
        if shift >= 64 || (shift == 63 && byte > 1) {
            return Err(SocialGraphError::UnexpectedEof);
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}

fn read_bytes<'a>(data: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = offset.saturating_add(len);
    if end > data.len() {
        return Err(SocialGraphError::UnexpectedEof);
    }
    let slice = &data[*offset..end];
    *offset = end;
    Ok(slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADAM: &str = "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e";
    const FIATJAF: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";

    fn event(pubkey: &str, kind: u32, created_at: u64, tagged: Vec<&str>) -> NostrEvent {
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

    #[test]
    fn read_queries_do_not_intern_unknown_users() {
        let graph = SocialGraph::new(ADAM);
        let before = graph.export_state();

        assert_eq!(
            graph.get_follow_distance("ff".repeat(32).as_str()),
            UNKNOWN_FOLLOW_DISTANCE
        );
        assert!(!graph.is_following(ADAM, &"ee".repeat(32)));
        assert!(graph.get_followed_by_user(&"dd".repeat(32)).is_empty());
        assert!(graph.get_followers_by_user(&"cc".repeat(32)).is_empty());
        assert!(graph.get_muted_by_user(&"bb".repeat(32)).is_empty());
        assert!(graph.get_user_muted_by(&"aa".repeat(32)).is_empty());
        assert_eq!(graph.get_follow_list_created_at(&"11".repeat(32)), None);
        assert_eq!(graph.get_mute_list_created_at(&"22".repeat(32)), None);
        assert!(!graph.is_overmuted(&"33".repeat(32), 1.0));

        let after = graph.export_state();
        assert_eq!(after.unique_ids, before.unique_ids);
    }

    #[test]
    fn unique_ids_round_trip_and_reuse_existing_ids() {
        let mut ids = UniqueIds::default();

        let adam = ids.id(ADAM).unwrap();
        let fiatjaf = ids.id(FIATJAF).unwrap();

        assert_eq!(adam, 0);
        assert_eq!(fiatjaf, 1);
        assert_eq!(ids.id(ADAM).unwrap(), adam);
        assert_eq!(ids.str_owned(adam).unwrap(), ADAM);
        assert_eq!(ids.str_owned(fiatjaf).unwrap(), FIATJAF);
    }

    #[test]
    fn unique_ids_reject_empty_strings_and_invalid_ids() {
        let mut ids = UniqueIds::default();

        assert_eq!(
            ids.id("   ").unwrap_err().to_string(),
            "cannot store empty or whitespace-only strings"
        );
        assert_eq!(ids.str_owned(99).unwrap_err().to_string(), "invalid id 99");
    }

    #[test]
    fn from_binary_rejects_invalid_version() {
        let error = SocialGraph::from_binary(ADAM, &[99]).unwrap_err();
        assert!(matches!(error, SocialGraphError::InvalidVersion(99)));
    }

    #[test]
    fn from_binary_rejects_truncated_binary() {
        let error = SocialGraph::from_binary(ADAM, &[BINARY_FORMAT_VERSION as u8]).unwrap_err();
        assert!(matches!(error, SocialGraphError::UnexpectedEof));
    }

    #[test]
    fn from_binary_reads_v2_pubkey_tables() {
        let mut binary = Vec::new();
        write_varint(&mut binary, BINARY_FORMAT_VERSION_V2);
        write_varint(&mut binary, 2);
        binary.extend_from_slice(&hex::decode(ADAM).unwrap());
        write_varint(&mut binary, 0);
        binary.extend_from_slice(&hex::decode(FIATJAF).unwrap());
        write_varint(&mut binary, 1);
        write_varint(&mut binary, 1);
        write_varint(&mut binary, 0);
        write_varint(&mut binary, 1_000);
        write_varint(&mut binary, 1);
        write_varint(&mut binary, 1);
        write_varint(&mut binary, 0);

        let restored = SocialGraph::from_binary(ADAM, &binary).unwrap();
        assert!(restored.is_following(ADAM, FIATJAF));
        assert_eq!(restored.get_follow_distance(FIATJAF), 1);
    }

    #[test]
    fn binary_v3_round_trips_non_hex_authors() {
        let mut graph = SocialGraph::new(ADAM);
        graph.handle_event(
            &event("not-a-hex-pubkey", 3, 1_000, vec![FIATJAF]),
            true,
            1.0,
        );

        let binary = graph.to_binary().unwrap();
        assert_eq!(binary[0], BINARY_FORMAT_VERSION as u8);

        let restored = SocialGraph::from_binary(ADAM, &binary).unwrap();
        assert!(restored.is_following("not-a-hex-pubkey", FIATJAF));
        assert_eq!(
            restored.get_follow_distance(FIATJAF),
            UNKNOWN_FOLLOW_DISTANCE
        );

        let restored_from_author = SocialGraph::from_binary("not-a-hex-pubkey", &binary).unwrap();
        assert_eq!(restored_from_author.get_follow_distance(FIATJAF), 1);
    }

    #[test]
    fn binary_v3_round_trips_uuid_and_string_nodes() {
        let uuid = "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a";
        let external = "external:nvpn-peer:exit-a";

        let mut graph = SocialGraph::new(ADAM);
        let root = graph.ids.existing_id(ADAM).unwrap();
        let uuid_id = graph.ids.id(uuid).unwrap();
        let external_id = graph.ids.id(external).unwrap();
        graph.private_add_follower(uuid_id, root);
        graph.private_add_follower(external_id, uuid_id);

        let binary = graph.to_binary().unwrap();
        assert_eq!(binary[0], BINARY_FORMAT_VERSION as u8);

        let restored = SocialGraph::from_binary(ADAM, &binary).unwrap();
        assert!(restored.is_following(ADAM, uuid));
        assert!(restored.is_following(uuid, external));
        assert_eq!(restored.get_follow_distance(uuid), 1);
        assert_eq!(restored.get_follow_distance(external), 2);

        // v5 round-trip preserves pubkeys, UUIDs, strings, distances, and shrinks bytes.
        let v5 = graph
            .to_binary_with_budget_version(BinaryBudget::default(), BINARY_FORMAT_VERSION_V5)
            .unwrap();
        assert_eq!(v5[0], BINARY_FORMAT_VERSION_V5 as u8);
        assert!(v5.len() < binary.len());
        let restored_v5 = SocialGraph::from_binary(ADAM, &v5).unwrap();
        assert!(restored_v5.is_following(ADAM, uuid));
        assert!(restored_v5.is_following(uuid, external));
        assert_eq!(restored_v5.get_follow_distance(uuid), 1);
        assert_eq!(restored_v5.get_follow_distance(external), 2);
        // Re-encoding a v5 graph produces identical v5 bytes.
        assert_eq!(
            restored_v5
                .to_binary_with_budget_version(BinaryBudget::default(), BINARY_FORMAT_VERSION_V5)
                .unwrap(),
            v5
        );
    }

    #[test]
    fn binary_v5_rejects_out_of_range_and_duplicate_identities() {
        // Out-of-range owner index (1 node, owner delta = 1).
        let mut bad_owner = vec![5, 1];
        bad_owner.extend([0x11; 32]);
        bad_owner.extend([0, 0, 1, 1, 100, 0, 0, 0]);
        assert!(SocialGraph::from_binary(ADAM, &bad_owner).is_err());

        // Out-of-range target delta (2 nodes, owner 0 follows target 2).
        let mut bad_target = vec![5, 2];
        bad_target.extend([0x11; 32]);
        bad_target.extend([0x22; 32]);
        bad_target.extend([0, 0, 1, 0, 100, 0, 1, 2, 0]);
        assert!(SocialGraph::from_binary(ADAM, &bad_target).is_err());

        // Duplicate pubkey inside v5 identity table.
        let mut dup_pk = vec![5, 2];
        dup_pk.extend([0x11; 32]);
        dup_pk.extend([0x11; 32]);
        dup_pk.extend([0, 0, 0, 0]);
        assert!(SocialGraph::from_binary(ADAM, &dup_pk).is_err());
    }
}
