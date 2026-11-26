use std::collections::{HashSet, VecDeque};
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

const BINARY_FORMAT_VERSION: u64 = 4;
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
    IndexMap<u32, IndexSet<u32>>,
    IndexMap<u32, IndexSet<u32>>,
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
struct UniqueIds {
    str_to_unique_id: IndexMap<String, u32>,
    unique_id_to_str: IndexMap<u32, String>,
    current_unique_id: u32,
}

impl UniqueIds {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            str_to_unique_id: IndexMap::with_capacity(capacity),
            unique_id_to_str: IndexMap::with_capacity(capacity),
            current_unique_id: 0,
        }
    }

    fn id(&mut self, value: &str) -> Result<u32> {
        if let Some(&id) = self.str_to_unique_id.get(value) {
            return Ok(id);
        }
        let value = aliases::canonical_node_id(value);
        let value = value.as_ref();
        if value.trim().is_empty() {
            return Err(SocialGraphError::EmptyString);
        }
        if let Some(&id) = self.str_to_unique_id.get(value) {
            return Ok(id);
        }
        let id = self.current_unique_id;
        self.current_unique_id = self.current_unique_id.saturating_add(1);
        self.str_to_unique_id.insert(value.to_owned(), id);
        self.unique_id_to_str.insert(id, value.to_owned());
        Ok(id)
    }

    fn existing_id(&self, value: &str) -> Option<u32> {
        if let Some(&id) = self.str_to_unique_id.get(value) {
            return Some(id);
        }
        let canonical = aliases::canonical_node_id(value);
        (canonical.as_ref() != value)
            .then(|| self.str_to_unique_id.get(canonical.as_ref()).copied())
            .flatten()
    }

    fn contains_id(&self, id: u32) -> bool {
        self.unique_id_to_str.contains_key(&id)
    }

    fn str(&self, id: u32) -> Result<&str> {
        self.unique_id_to_str
            .get(&id)
            .map(String::as_str)
            .ok_or(SocialGraphError::InvalidId(id))
    }

    fn insert_with_id(&mut self, value: String, id: u32) {
        self.current_unique_id = self.current_unique_id.max(id.saturating_add(1));
        self.str_to_unique_id.insert(value.clone(), id);
        self.unique_id_to_str.insert(id, value);
    }

    fn try_insert_unique(&mut self, value: String, id: u32) -> bool {
        use indexmap::map::Entry;
        let Entry::Vacant(id_slot) = self.unique_id_to_str.entry(id) else {
            return false;
        };
        let Entry::Vacant(str_slot) = self.str_to_unique_id.entry(value.clone()) else {
            return false;
        };
        id_slot.insert(value);
        str_slot.insert(id);
        self.current_unique_id = self.current_unique_id.max(id.saturating_add(1));
        true
    }
}

#[derive(Debug, Clone, Default)]
pub struct SocialGraph {
    list_revisions: IndexMap<(u32, u32), ListRevision>,
    root: u32,
    follow_distance_by_user: IndexMap<u32, u32>,
    users_by_follow_distance: IndexMap<u32, IndexSet<u32>>,
    followed_by_user: IndexMap<u32, IndexSet<u32>>,
    followers_by_user: IndexMap<u32, IndexSet<u32>>,
    follow_list_created_at: IndexMap<u32, u64>,
    muted_by_user: IndexMap<u32, IndexSet<u32>>,
    user_muted_by: IndexMap<u32, IndexSet<u32>>,
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
        let mut follow_distance_by_user = IndexMap::new();
        follow_distance_by_user.insert(root_id, 0);
        let mut users_by_follow_distance = IndexMap::new();
        let mut bucket = IndexSet::new();
        bucket.insert(root_id);
        users_by_follow_distance.insert(0, bucket);
        Self {
            list_revisions: IndexMap::new(),
            root: root_id,
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
            .cloned()
    }

    pub fn get_list_revisions(&self) -> Vec<(String, u32, ListRevision)> {
        self.list_revisions
            .iter()
            .filter_map(|((owner, kind), revision)| {
                Some((
                    self.ids.str(*owner).ok()?.to_string(),
                    *kind,
                    revision.clone(),
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
        self.ids
            .str(self.root)
            .expect("root id should always exist")
    }

    pub fn set_root(&mut self, root: &str) -> Result<()> {
        let root_id = self.ids.id(root)?;
        if root_id == self.root {
            return Ok(());
        }
        self.root = root_id;
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
            if !self.follow_distance_by_user.contains_key(&author) {
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
            .is_some_and(|targets| targets.contains(&target_id));
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
        self.follow_distance_by_user.clear();
        self.users_by_follow_distance.clear();
        self.follow_distance_by_user.insert(self.root, 0);
        let mut root_bucket = IndexSet::new();
        root_bucket.insert(self.root);
        self.users_by_follow_distance.insert(0, root_bucket);

        let mut queue = VecDeque::from([self.root]);
        while let Some(user) = queue.pop_front() {
            let Some(distance) = self.follow_distance_by_user.get(&user).copied() else {
                continue;
            };
            let next_distance = distance.saturating_add(1);
            for &target in self.followed_by_user.get(&user).into_iter().flatten() {
                if self.follow_distance_by_user.contains_key(&target) {
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
            .get(&user_id)
            .copied()
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
            .is_some_and(|set| set.contains(&followed_id))
    }

    pub fn get_followed_by_user(&self, user: &str) -> Vec<String> {
        let Some(user_id) = self.ids.existing_id(user) else {
            return Vec::new();
        };
        self.followed_by_user
            .get(&user_id)
            .into_iter()
            .flat_map(|set| set.iter())
            .filter_map(|id| self.ids.str(*id).ok().map(ToOwned::to_owned))
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
            .filter_map(|id| self.ids.str(*id).ok().map(ToOwned::to_owned))
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
            .filter_map(|id| self.ids.str(*id).ok().map(ToOwned::to_owned))
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
            .filter_map(|id| self.ids.str(*id).ok().map(ToOwned::to_owned))
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
            .map(IndexSet::len)
            .sum::<usize>();
        let mutes = self
            .muted_by_user
            .values()
            .map(IndexSet::len)
            .sum::<usize>();
        let size_by_distance = self
            .users_by_follow_distance
            .iter()
            .map(|(distance, users)| (*distance, users.len()))
            .collect();

        GraphStats {
            users: if self.follow_distance_by_user.is_empty() {
                self.ids.unique_id_to_str.len()
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
            .filter_map(|id| self.ids.str(*id).ok().map(ToOwned::to_owned))
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
        let has_followers: IndexSet<u32> = self
            .followed_by_user
            .values()
            .flat_map(|users| users.iter().copied())
            .collect();
        let users_to_remove: IndexSet<u32> = self
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
        for (distance, users) in &mut self.users_by_follow_distance {
            // Preserve the assigned-bucket behavior even for imported state whose
            // distance buckets and distance map have not yet been reconciled.
            users.retain(|user| {
                !users_to_remove.contains(user)
                    || self.follow_distance_by_user.get(user) != Some(distance)
            });
        }
        self.follow_distance_by_user
            .retain(|user, _| !users_to_remove.contains(user));
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
        // Remove the same string keys as the prior per-ID operation, including
        // imported states that contain duplicate string values for distinct IDs.
        let removed_values: IndexSet<&str> = self
            .ids
            .unique_id_to_str
            .iter()
            .filter(|(user, _)| users_to_remove.contains(*user))
            .map(|(_, value)| value.as_str())
            .collect();
        self.ids
            .str_to_unique_id
            .retain(|value, _| !removed_values.contains(value.as_str()));
        self.ids
            .unique_id_to_str
            .retain(|user, _| !users_to_remove.contains(user));

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
                .map(|((owner, kind), revision)| (*owner, *kind, revision.clone()))
                .collect(),
            root: self.get_root().to_string(),
            unique_ids: self
                .ids
                .unique_id_to_str
                .iter()
                .map(|(id, value)| (value.clone(), *id))
                .collect(),
            follow_distance_by_user: self
                .follow_distance_by_user
                .iter()
                .map(|(id, distance)| (*id, *distance))
                .collect(),
            users_by_follow_distance: self
                .users_by_follow_distance
                .iter()
                .map(|(distance, users)| (*distance, users.iter().copied().collect()))
                .collect(),
            followed_by_user: self
                .followed_by_user
                .iter()
                .map(|(user, followed)| (*user, followed.iter().copied().collect()))
                .collect(),
            followers_by_user: self
                .followers_by_user
                .iter()
                .map(|(user, followers)| (*user, followers.iter().copied().collect()))
                .collect(),
            follow_list_created_at: self
                .follow_list_created_at
                .iter()
                .map(|(user, created_at)| (*user, *created_at))
                .collect(),
            muted_by_user: self
                .muted_by_user
                .iter()
                .map(|(user, muted)| (*user, muted.iter().copied().collect()))
                .collect(),
            user_muted_by: self
                .user_muted_by
                .iter()
                .map(|(user, muters)| (*user, muters.iter().copied().collect()))
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
            ids.insert_with_id(value, id);
        }
        let root = ids.id(&state.root)?;
        let mut graph = Self {
            list_revisions: state
                .list_revisions
                .into_iter()
                .map(|(owner, kind, revision)| ((owner, kind), revision))
                .collect(),
            root,
            follow_distance_by_user: state.follow_distance_by_user.into_iter().collect(),
            users_by_follow_distance: state
                .users_by_follow_distance
                .into_iter()
                .map(|(distance, users)| (distance, users.into_iter().collect()))
                .collect(),
            followed_by_user: state
                .followed_by_user
                .into_iter()
                .map(|(user, followed)| (user, followed.into_iter().collect()))
                .collect(),
            followers_by_user: state
                .followers_by_user
                .into_iter()
                .map(|(user, followers)| (user, followers.into_iter().collect()))
                .collect(),
            follow_list_created_at: state.follow_list_created_at.into_iter().collect(),
            muted_by_user: state
                .muted_by_user
                .into_iter()
                .map(|(user, muted)| (user, muted.into_iter().collect()))
                .collect(),
            user_muted_by: state
                .user_muted_by
                .into_iter()
                .map(|(user, muters)| (user, muters.into_iter().collect()))
                .collect(),
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

    /// Explicit legacy v3 export for rolling upgrades; v3 omits revision metadata.
    pub fn to_binary_with_budget_version(
        &self,
        budget: BinaryBudget,
        version: u64,
    ) -> Result<Vec<u8>> {
        if !matches!(version, 3 | BINARY_FORMAT_VERSION) {
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
        let Some(root) = self.ids.existing_id(root) else {
            return Self::new(root).to_binary_with_budget(budget);
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
                for target in lists.get(&owner).into_iter().flatten().copied() {
                    if per_owner.is_some_and(|limit| owner_edges >= limit) {
                        break;
                    }
                    if max_edges.is_some_and(|limit| edge_count >= limit) {
                        break 'owners;
                    }
                    if self.ids.str(target).is_err() {
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
            &distances,
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
            BINARY_FORMAT_VERSION,
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
        distances: &IndexMap<u32, u32>,
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
            let Some(distance) = distances.get(&owner) else {
                continue;
            };
            if budget.max_distance.is_some_and(|limit| *distance > limit)
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
                if lists.get(&owner).is_none_or(IndexSet::is_empty)
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
            .filter(|r| r.created_at == timestamp && valid_event_id(&r.id))
        {
            Some(revision) => {
                out.push(if revision.complete && complete { 2 } else { 1 });
                let mut bytes = [0u8; 32];
                hex::decode_to_slice(&revision.id, &mut bytes).expect("validated event ID");
                out.extend_from_slice(&bytes);
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
        lists: &IndexMap<u32, IndexSet<u32>>,
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
                    limit == targets.map_or(0, IndexSet::len),
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
            write_binary_node_id(&mut out, self.ids.str(id)?);
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

    fn plan_binary(&self, budget: BinaryBudget) -> Result<BinaryPlan> {
        if !budget.has_active_limits() {
            return Ok(self.plan_full_binary());
        }

        let max_nodes = budget.max_nodes.filter(|value| *value > 0);
        let max_edges = budget.max_edges.filter(|value| *value > 0);
        let max_edges_per_node = budget.max_edges_per_node.filter(|value| *value > 0);

        let mut distances: Vec<u32> = self.users_by_follow_distance.keys().copied().collect();
        distances.sort_unstable();

        let cap = max_nodes
            .unwrap_or(self.ids.unique_id_to_str.len())
            .min(self.ids.unique_id_to_str.len());
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
            &self.follow_distance_by_user,
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
        let mut used_ids = IndexSet::with_capacity(self.ids.unique_id_to_str.len());
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
            BINARY_FORMAT_VERSION | 3 | BINARY_FORMAT_VERSION_V2
        ) {
            return Err(SocialGraphError::InvalidVersion(version));
        }

        let ids_count = read_count(data, &mut offset)?;
        let mut ids = UniqueIds::with_capacity(ids_count);
        let mut aliased_ids: Option<Vec<(String, u32)>> = None;

        for _ in 0..ids_count {
            let (value, id) = read_binary_node_id(version, data, &mut offset)?;
            if let Some(raw_ids) = &mut aliased_ids {
                raw_ids.push((value, id));
            } else if !ids.try_insert_unique(value.clone(), id) {
                let mut raw_ids = Vec::with_capacity(ids_count);
                for (&eid, eval) in &ids.unique_id_to_str {
                    raw_ids.push((eval.clone(), eid));
                }
                raw_ids.push((value, id));
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
                        revisions.insert((user, kind), revision);
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
                build_reverse_index(&graph.followed_by_user, &mut graph.followers_by_user);
            }
            if graph.user_muted_by.is_empty() {
                build_reverse_index(&graph.muted_by_user, &mut graph.user_muted_by);
            }
            if graph.follow_distance_by_user.is_empty() {
                graph.recalculate_follow_distances();
            }
            return Ok(graph);
        }

        // Alias-free fast path: decode lists directly into graph maps without intermediate Vecs.
        let mut list_revisions = IndexMap::new();
        let mut read_direct = |kind: u32| -> Result<DecodedLists> {
            let count = read_count(data, &mut offset)?;
            let mut forward: IndexMap<u32, IndexSet<u32>> = IndexMap::with_capacity(count);
            let mut reverse: IndexMap<u32, IndexSet<u32>> =
                IndexMap::with_capacity(if kind == 3 { ids_count } else { 0 });
            let mut timestamps: IndexMap<u32, u64> = IndexMap::with_capacity(count);
            let mut had_duplicate_owner = false;
            for _ in 0..count {
                let owner = read_varint(data, &mut offset)? as u32;
                let timestamp = read_varint(data, &mut offset)?;
                if version >= 4
                    && let Some(revision) = read_list_revision(data, &mut offset, timestamp)?
                {
                    list_revisions.insert((owner, kind), revision);
                }
                let edge_count = read_count(data, &mut offset)?;
                let mut targets = IndexSet::with_capacity(edge_count);
                for _ in 0..edge_count {
                    let target = read_varint(data, &mut offset)? as u32;
                    if targets.insert(target) && !had_duplicate_owner {
                        reverse.entry(target).or_default().insert(owner);
                    }
                }
                if forward.insert(owner, targets).is_some() {
                    had_duplicate_owner = true;
                }
                timestamps.insert(owner, timestamp);
            }
            if had_duplicate_owner {
                reverse.clear();
                build_reverse_index(&forward, &mut reverse);
            }
            Ok((forward, reverse, timestamps))
        };

        let (followed_by_user, followers_by_user, follow_list_created_at) = read_direct(3)?;
        let (muted_by_user, user_muted_by, mute_list_created_at) = read_direct(10_000)?;
        let root_id = ids.id(root)?;

        let mut graph = Self {
            list_revisions,
            root: root_id,
            follow_distance_by_user: IndexMap::with_capacity(ids_count),
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
        if muters.contains(&self.root) {
            return true;
        }

        let mut min_dist = u32::MAX;
        let mut muters_at_min = 0u32;
        for muter in muters {
            if let Some(&distance) = self.follow_distance_by_user.get(muter) {
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
            for follower in followers {
                if let Some(&distance) = self.follow_distance_by_user.get(follower) {
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
        if !accepts_list_revision(
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
                ListRevision {
                    id: id.to_owned(),
                    created_at,
                    complete: true,
                },
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

    fn collect_pubkey_tags(&mut self, author: u32, tags: &[Vec<String>]) -> IndexSet<u32> {
        let mut targets = IndexSet::with_capacity(tags.len());
        for tag in tags {
            if tag.first().is_some_and(|value| value == "p")
                && let Some(pk) = tag.get(1)
                && is_valid_pubkey(pk)
                && let Ok(target) = self.ids.id(pk)
                && target != author
            {
                targets.insert(target);
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
        if let Some(author_dist) = self.follow_distance_by_user.get(&author).copied() {
            if !removed.is_empty() {
                let lost_shortest: Vec<u32> = removed
                    .into_iter()
                    .filter(|u| {
                        self.follow_distance_by_user
                            .get(u)
                            .is_some_and(|&d| d > 0 && d - 1 == author_dist)
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
        if let Some(distance) = self.follow_distance_by_user.get(&follower).copied() {
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
            let current = self.follow_distance_by_user.get(&start).copied();
            if current.is_none_or(|current| current > distance) {
                moved.entry(start).or_insert(current);
                self.follow_distance_by_user.insert(start, distance);
                queue.push_back(start);
            }
        }
        while let Some(user) = queue.pop_front() {
            let next = self.follow_distance_by_user[&user].saturating_add(1);
            for &target in self.followed_by_user.get(&user).into_iter().flatten() {
                let known = self.follow_distance_by_user.get(&target).copied();
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
            let Some(&distance) = self.follow_distance_by_user.get(&user) else {
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
                .filter(|target| {
                    self.follow_distance_by_user.get(target) == Some(&next)
                        && !self.has_follower_at(*target, next - 1)
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
                .filter_map(|follower| self.follow_distance_by_user.get(follower).copied())
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
                .any(|follower| self.follow_distance_by_user.get(follower) == Some(&distance))
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
                .get(&user)
                .copied()
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
        if let [user] = detached[..] {
            self.follow_distance_by_user.shift_remove(&user);
        } else if !detached.is_empty() {
            let detached: HashSet<u32> = detached.into_iter().collect();
            self.follow_distance_by_user
                .retain(|user, _| !detached.contains(user));
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
    forward: &mut IndexMap<u32, IndexSet<u32>>,
    author: u32,
    incoming: IndexSet<u32>,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    if let Some(existing) = forward.get_mut(&author) {
        if existing.len() == incoming.len() && existing.iter().all(|u| incoming.contains(u)) {
            return None;
        }
        existing.retain(|&u| {
            if incoming.contains(&u) {
                true
            } else {
                removed.push(u);
                false
            }
        });
        for user in incoming {
            if existing.insert(user) {
                added.push(user);
            }
        }
    } else if !incoming.is_empty() {
        added.extend(incoming.iter().copied());
        forward.insert(author, incoming);
    } else {
        return None;
    }
    Some((removed, added))
}

fn update_reverse_edges(
    reverse: &mut IndexMap<u32, IndexSet<u32>>,
    author: u32,
    removed: &[u32],
    added: &[u32],
) {
    for &user in removed {
        if let Some(set) = reverse.get_mut(&user) {
            set.shift_remove(&author);
        }
    }
    for &user in added {
        reverse.entry(user).or_default().insert(author);
    }
}

fn build_reverse_index(
    forward: &IndexMap<u32, IndexSet<u32>>,
    reverse: &mut IndexMap<u32, IndexSet<u32>>,
) {
    for (&owner, targets) in forward {
        for &target in targets {
            reverse.entry(target).or_default().insert(owner);
        }
    }
}

fn is_valid_pubkey(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_canonical_uuid(value: &str) -> Option<Uuid> {
    let uuid = Uuid::parse_str(value).ok()?;
    (uuid.to_string() == value).then_some(uuid)
}

fn write_binary_node_id(out: &mut Vec<u8>, value: &str) {
    if is_valid_pubkey(value) {
        write_varint(out, BINARY_NODE_ID_PUBKEY);
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(value, &mut bytes).expect("validated pubkey hex should decode");
        out.extend_from_slice(&bytes);
        return;
    }

    if let Some(uuid) = is_canonical_uuid(value) {
        write_varint(out, BINARY_NODE_ID_UUID);
        out.extend_from_slice(uuid.as_bytes());
        return;
    }

    write_varint(out, BINARY_NODE_ID_STRING);
    write_varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

fn read_typed_node_id(data: &[u8], offset: &mut usize) -> Result<String> {
    match read_varint(data, offset)? {
        BINARY_NODE_ID_PUBKEY => Ok(hex::encode(read_bytes(data, offset, 32)?)),
        BINARY_NODE_ID_UUID => Ok(Uuid::from_slice(read_bytes(data, offset, 16)?)
            .map_err(|_| SocialGraphError::InvalidBinaryNodeIdUuid)?
            .to_string()),
        BINARY_NODE_ID_STRING => {
            let len = read_varint(data, offset)? as usize;
            let raw = str::from_utf8(read_bytes(data, offset, len)?)
                .map_err(|_| SocialGraphError::InvalidBinaryNodeIdUtf8)?;
            if raw.trim().is_empty() {
                return Err(SocialGraphError::EmptyString);
            }
            Ok(aliases::canonical_node_id(raw).into_owned())
        }
        other => Err(SocialGraphError::InvalidBinaryNodeIdType(other)),
    }
}

fn read_binary_node_id(version: u64, data: &[u8], offset: &mut usize) -> Result<(String, u32)> {
    if version == BINARY_FORMAT_VERSION_V2 {
        let hex_bytes = read_bytes(data, offset, 32)?;
        let id = read_varint(data, offset)? as u32;
        return Ok((hex::encode(hex_bytes), id));
    }

    let id = read_varint(data, offset)? as u32;
    Ok((read_typed_node_id(data, offset)?, id))
}

fn read_list_revision(
    data: &[u8],
    offset: &mut usize,
    created_at: u64,
) -> Result<Option<ListRevision>> {
    let flag = read_bytes(data, offset, 1)?[0];
    if flag == 0 {
        return Ok(None);
    }
    if flag > 2 {
        return Err(SocialGraphError::InvalidBinaryNodeIdType(flag as u64));
    }
    Ok(Some(ListRevision {
        id: hex::encode(read_bytes(data, offset, 32)?),
        created_at,
        complete: flag == 2,
    }))
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
        assert_eq!(ids.str(adam).unwrap(), ADAM);
        assert_eq!(ids.str(fiatjaf).unwrap(), FIATJAF);
    }

    #[test]
    fn unique_ids_reject_empty_strings_and_invalid_ids() {
        let mut ids = UniqueIds::default();

        assert_eq!(
            ids.id("   ").unwrap_err().to_string(),
            "cannot store empty or whitespace-only strings"
        );
        assert_eq!(ids.str(99).unwrap_err().to_string(), "invalid id 99");
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
    }
}
