use std::collections::VecDeque;
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::{IndexMap, IndexSet};
use uuid::Uuid;

mod rating;
pub use rating::*;

const BINARY_FORMAT_VERSION: u64 = 3;
const BINARY_FORMAT_VERSION_V2: u64 = 2;
const BINARY_CHUNK_SIZE: usize = 16 * 1024;
const MAX_FUTURE_EVENT_SECONDS: u64 = 10 * 60;
const UNKNOWN_FOLLOW_DISTANCE: u32 = 1000;
const BINARY_NODE_ID_PUBKEY: u64 = 0;
const BINARY_NODE_ID_UUID: u64 = 1;
const BINARY_NODE_ID_STRING: u64 = 2;

#[derive(Debug, thiserror::Error)]
pub enum SocialGraphError {
    #[error("cannot store empty or whitespace-only strings")]
    EmptyString,
    #[error("invalid id {0}")]
    InvalidId(u32),
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
    fn id(&mut self, value: &str) -> Result<u32> {
        if value.trim().is_empty() {
            return Err(SocialGraphError::EmptyString);
        }
        if let Some(id) = self.str_to_unique_id.get(value) {
            return Ok(*id);
        }
        let id = self.current_unique_id;
        self.current_unique_id = self.current_unique_id.saturating_add(1);
        self.str_to_unique_id.insert(value.to_owned(), id);
        self.unique_id_to_str.insert(id, value.to_owned());
        Ok(id)
    }

    fn existing_id(&self, value: &str) -> Option<u32> {
        self.str_to_unique_id.get(value).copied()
    }

    fn str(&self, id: u32) -> Result<&str> {
        self.unique_id_to_str
            .get(&id)
            .map(String::as_str)
            .ok_or(SocialGraphError::InvalidId(id))
    }

    fn clear(&mut self) {
        self.str_to_unique_id.clear();
        self.unique_id_to_str.clear();
        self.current_unique_id = 0;
    }

    fn insert_with_id(&mut self, value: String, id: u32) {
        self.current_unique_id = self.current_unique_id.max(id.saturating_add(1));
        self.str_to_unique_id.insert(value.clone(), id);
        self.unique_id_to_str.insert(id, value);
    }

    fn remove(&mut self, id: u32) {
        if let Some(value) = self.unique_id_to_str.shift_remove(&id) {
            self.str_to_unique_id.shift_remove(&value);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SocialGraph {
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

#[derive(Debug, Clone, Copy)]
struct PotentialEdge {
    owner: u32,
    target: u32,
    is_follow: bool,
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

        if self.is_overmuted(&event.pubkey, overmute_threshold) {
            return;
        }

        match event.kind {
            3 => self.handle_follow_list(author, event.created_at, &event.tags),
            10_000 => self.handle_mute_list(author, event.created_at, &event.tags),
            _ => {}
        }
    }

    pub fn add_positive_relation(
        &mut self,
        rater: &str,
        target: &str,
        created_at: u64,
    ) -> Result<bool> {
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
            let Some(followed) = self.followed_by_user.get(&user).cloned() else {
                continue;
            };
            let next_distance = distance.saturating_add(1);
            for target in followed {
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
            self.ids.remove(user);
        }

        for followed_users in self.followed_by_user.values_mut() {
            for user in &users_to_remove {
                followed_users.shift_remove(user);
            }
        }
        for followers in self.followers_by_user.values_mut() {
            for user in &users_to_remove {
                followers.shift_remove(user);
            }
        }
        for muted_users in self.muted_by_user.values_mut() {
            for user in &users_to_remove {
                muted_users.shift_remove(user);
            }
        }
        for muters in self.user_muted_by.values_mut() {
            for user in &users_to_remove {
                muters.shift_remove(user);
            }
        }

        users_to_remove.len()
    }

    pub fn export_state(&self) -> SocialGraphState {
        SocialGraphState {
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
        let mut graph = Self::new(&state.root);
        graph.ids.clear();
        graph.follow_distance_by_user.clear();
        graph.users_by_follow_distance.clear();
        graph.followed_by_user.clear();
        graph.followers_by_user.clear();
        graph.follow_list_created_at.clear();
        graph.muted_by_user.clear();
        graph.user_muted_by.clear();
        graph.mute_list_created_at.clear();

        for (value, id) in state.unique_ids {
            graph.ids.insert_with_id(value, id);
        }

        graph.root = graph.ids.id(&state.root)?;

        for (user, distance) in state.follow_distance_by_user {
            graph.follow_distance_by_user.insert(user, distance);
        }
        for (distance, users) in state.users_by_follow_distance {
            graph
                .users_by_follow_distance
                .insert(distance, users.into_iter().collect());
        }
        for (user, followed) in state.followed_by_user {
            graph
                .followed_by_user
                .insert(user, followed.into_iter().collect());
        }
        for (user, followers) in state.followers_by_user {
            graph
                .followers_by_user
                .insert(user, followers.into_iter().collect());
        }
        for (user, created_at) in state.follow_list_created_at {
            graph.follow_list_created_at.insert(user, created_at);
        }
        for (user, muted) in state.muted_by_user {
            graph
                .muted_by_user
                .insert(user, muted.into_iter().collect());
        }
        for (user, muters) in state.user_muted_by {
            graph
                .user_muted_by
                .insert(user, muters.into_iter().collect());
        }
        for (user, created_at) in state.mute_list_created_at {
            graph.mute_list_created_at.insert(user, created_at);
        }

        Ok(graph)
    }

    pub fn to_binary(&self) -> Result<Vec<u8>> {
        self.to_binary_with_budget(BinaryBudget::default())
    }

    pub fn to_binary_with_budget(&self, budget: BinaryBudget) -> Result<Vec<u8>> {
        let plan = self.plan_binary(budget)?;
        self.serialize_binary(&plan)
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

    fn serialize_binary(&self, plan: &BinaryPlan) -> Result<Vec<u8>> {
        let BinaryPlan {
            used_ids,
            follow_edge_count,
            mute_edge_count,
            follow_owners,
            mute_owners,
        } = plan;

        let mut out = Vec::new();
        write_varint(&mut out, BINARY_FORMAT_VERSION);
        write_varint(&mut out, used_ids.len() as u64);
        for id in used_ids.iter().copied() {
            write_varint(&mut out, id as u64);
            write_binary_node_id(&mut out, self.ids.str(id)?);
        }

        write_varint(&mut out, follow_owners.len() as u64);
        for owner in follow_owners.iter().copied() {
            let limit = follow_edge_count
                .get(&owner)
                .copied()
                .expect("follow owner must have edge count");
            write_varint(&mut out, owner as u64);
            write_varint(
                &mut out,
                self.follow_list_created_at
                    .get(&owner)
                    .copied()
                    .unwrap_or(0),
            );
            write_varint(&mut out, limit as u64);
            if let Some(targets) = self.followed_by_user.get(&owner) {
                for target in targets.iter().take(limit) {
                    write_varint(&mut out, *target as u64);
                }
            }
        }

        write_varint(&mut out, mute_owners.len() as u64);
        for owner in mute_owners.iter().copied() {
            let limit = mute_edge_count
                .get(&owner)
                .copied()
                .expect("mute owner must have edge count");
            write_varint(&mut out, owner as u64);
            write_varint(
                &mut out,
                self.mute_list_created_at.get(&owner).copied().unwrap_or(0),
            );
            write_varint(&mut out, limit as u64);
            if let Some(targets) = self.muted_by_user.get(&owner) {
                for target in targets.iter().take(limit) {
                    write_varint(&mut out, *target as u64);
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

        let mut potential_edges = Vec::new();
        for distance in distances {
            if budget
                .max_distance
                .is_some_and(|max_distance| distance > max_distance)
            {
                continue;
            }
            let Some(users) = self.users_by_follow_distance.get(&distance) else {
                continue;
            };

            for owner in users.iter().copied() {
                let mut owner_edge_count = 0usize;

                if let Some(followed_users) = self.followed_by_user.get(&owner) {
                    for target in followed_users.iter().copied() {
                        if max_edges_per_node.is_none_or(|limit| owner_edge_count < limit) {
                            potential_edges.push(PotentialEdge {
                                owner,
                                target,
                                is_follow: true,
                            });
                            owner_edge_count += 1;
                        }
                    }
                }

                if let Some(muted_users) = self.muted_by_user.get(&owner) {
                    for target in muted_users.iter().copied() {
                        if max_edges_per_node.is_none_or(|limit| owner_edge_count < limit) {
                            potential_edges.push(PotentialEdge {
                                owner,
                                target,
                                is_follow: false,
                            });
                            owner_edge_count += 1;
                        }
                    }
                }
            }
        }

        let mut used_ids = IndexSet::new();
        let mut follow_edge_count = IndexMap::new();
        let mut mute_edge_count = IndexMap::new();
        let mut edge_count = 0usize;

        for edge in potential_edges {
            if max_edges.is_some_and(|limit| edge_count >= limit) {
                break;
            }

            if self.ids.str(edge.owner).is_err() || self.ids.str(edge.target).is_err() {
                continue;
            }

            if let Some(limit) = max_nodes {
                let new_nodes_count = usize::from(!used_ids.contains(&edge.owner))
                    + usize::from(!used_ids.contains(&edge.target));
                if used_ids.len() + new_nodes_count > limit {
                    break;
                }
            }

            used_ids.insert(edge.owner);
            used_ids.insert(edge.target);
            edge_count += 1;

            let edge_counts = if edge.is_follow {
                &mut follow_edge_count
            } else {
                &mut mute_edge_count
            };
            *edge_counts.entry(edge.owner).or_insert(0) += 1;
        }

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
        let mut used_ids = IndexSet::new();
        let mut follow_edge_count = IndexMap::new();
        let mut mute_edge_count = IndexMap::new();

        for (user, followed_users) in &self.followed_by_user {
            used_ids.insert(*user);
            follow_edge_count.insert(*user, followed_users.len());
            for followed in followed_users {
                used_ids.insert(*followed);
            }
        }

        for (user, muted_users) in &self.muted_by_user {
            used_ids.insert(*user);
            mute_edge_count.insert(*user, muted_users.len());
            for muted in muted_users {
                used_ids.insert(*muted);
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
        if !matches!(version, BINARY_FORMAT_VERSION | BINARY_FORMAT_VERSION_V2) {
            return Err(SocialGraphError::InvalidVersion(version));
        }

        let ids_count = read_varint(data, &mut offset)? as usize;
        let mut unique_ids = Vec::with_capacity(ids_count);
        for _ in 0..ids_count {
            let (value, id) = read_binary_node_id(version, data, &mut offset)?;
            if value.trim().is_empty() {
                return Err(SocialGraphError::EmptyString);
            }
            unique_ids.push((value, id));
        }

        let follow_lists_count = read_varint(data, &mut offset)? as usize;
        let mut follow_lists = Vec::with_capacity(follow_lists_count);
        for _ in 0..follow_lists_count {
            let user = read_varint(data, &mut offset)? as u32;
            let timestamp = read_varint(data, &mut offset)?;
            let followed_count = read_varint(data, &mut offset)? as usize;
            let mut followed = Vec::with_capacity(followed_count);
            for _ in 0..followed_count {
                followed.push(read_varint(data, &mut offset)? as u32);
            }
            follow_lists.push((user, followed, timestamp));
        }

        let mute_lists_count = read_varint(data, &mut offset)? as usize;
        let mut mute_lists = Vec::with_capacity(mute_lists_count);
        for _ in 0..mute_lists_count {
            let user = read_varint(data, &mut offset)? as u32;
            let timestamp = read_varint(data, &mut offset)?;
            let muted_count = read_varint(data, &mut offset)? as usize;
            let mut muted = Vec::with_capacity(muted_count);
            for _ in 0..muted_count {
                muted.push(read_varint(data, &mut offset)? as u32);
            }
            mute_lists.push((user, muted, timestamp));
        }

        let mut graph = Self::new(root);
        graph.ids.clear();
        graph.follow_distance_by_user.clear();
        graph.users_by_follow_distance.clear();
        graph.followed_by_user.clear();
        graph.followers_by_user.clear();
        graph.follow_list_created_at.clear();
        graph.muted_by_user.clear();
        graph.user_muted_by.clear();
        graph.mute_list_created_at.clear();

        for (value, id) in unique_ids {
            graph.ids.insert_with_id(value, id);
        }

        graph.root = graph.ids.id(root)?;

        for (follower, followed_users, created_at) in follow_lists {
            graph
                .followed_by_user
                .insert(follower, followed_users.iter().copied().collect());
            for followed_user in followed_users {
                graph
                    .followers_by_user
                    .entry(followed_user)
                    .or_default()
                    .insert(follower);
            }
            graph.follow_list_created_at.insert(follower, created_at);
        }

        for (muter, muted_users, created_at) in mute_lists {
            let entry = graph.muted_by_user.entry(muter).or_default();
            for muted_user in muted_users {
                entry.insert(muted_user);
                graph
                    .user_muted_by
                    .entry(muted_user)
                    .or_default()
                    .insert(muter);
            }
            graph.mute_list_created_at.insert(muter, created_at);
        }

        graph.recalculate_follow_distances();

        Ok(graph)
    }

    pub fn is_overmuted(&self, user: &str, threshold: f64) -> bool {
        if user == self.get_root() {
            return false;
        }

        let Some(user_id) = self.ids.existing_id(user) else {
            return false;
        };
        let Some(muters) = self.user_muted_by.get(&user_id) else {
            return false;
        };
        if muters.is_empty() {
            return false;
        }
        if muters.contains(&self.root) {
            return true;
        }

        let mut stats = IndexMap::<u32, (u32, u32)>::new();
        if let Some(followers) = self.followers_by_user.get(&user_id) {
            for follower in followers {
                if let Some(distance) = self.follow_distance_by_user.get(follower).copied() {
                    let entry = stats.entry(distance).or_insert((0, 0));
                    entry.0 += 1;
                }
            }
        }
        for muter in muters {
            if let Some(distance) = self.follow_distance_by_user.get(muter).copied() {
                let entry = stats.entry(distance).or_insert((0, 0));
                entry.1 += 1;
            }
        }

        let mut distances: Vec<u32> = stats.keys().copied().collect();
        distances.sort_unstable();
        for distance in distances {
            let (followers, muters) = stats[&distance];
            if followers + muters > 0 {
                return (muters as f64) * threshold > followers as f64;
            }
        }
        false
    }

    fn handle_follow_list(&mut self, author: u32, created_at: u64, tags: &[Vec<String>]) {
        if self
            .follow_list_created_at
            .get(&author)
            .is_some_and(|existing| created_at <= *existing)
        {
            return;
        }
        self.follow_list_created_at.insert(author, created_at);

        let mut followed_in_event = IndexSet::new();
        for tag in tags {
            if tag.first().is_some_and(|value| value == "p")
                && tag.get(1).is_some_and(|pk| is_valid_pubkey(pk))
            {
                let Ok(followed_user) = self.ids.id(&tag[1]) else {
                    continue;
                };
                if followed_user != author {
                    followed_in_event.insert(followed_user);
                }
            }
        }

        let currently_followed = self
            .followed_by_user
            .get(&author)
            .cloned()
            .unwrap_or_default();
        for user in currently_followed {
            if !followed_in_event.contains(&user) {
                self.private_remove_follower(user, author);
            }
        }
        for user in followed_in_event {
            self.private_add_follower(user, author);
        }
    }

    fn handle_mute_list(&mut self, author: u32, created_at: u64, tags: &[Vec<String>]) {
        if self
            .mute_list_created_at
            .get(&author)
            .is_some_and(|existing| created_at <= *existing)
        {
            return;
        }
        self.mute_list_created_at.insert(author, created_at);

        let mut muted_in_event = IndexSet::new();
        for tag in tags {
            if tag.first().is_some_and(|value| value == "p")
                && tag.get(1).is_some_and(|pk| is_valid_pubkey(pk))
            {
                let Ok(muted_user) = self.ids.id(&tag[1]) else {
                    continue;
                };
                if muted_user != author {
                    muted_in_event.insert(muted_user);
                }
            }
        }

        let currently_muted = self.muted_by_user.get(&author).cloned().unwrap_or_default();
        for user in currently_muted {
            if !muted_in_event.contains(&user) {
                if let Some(set) = self.muted_by_user.get_mut(&author) {
                    set.shift_remove(&user);
                }
                if let Some(set) = self.user_muted_by.get_mut(&user) {
                    set.shift_remove(&author);
                }
            }
        }

        for user in muted_in_event {
            self.muted_by_user.entry(author).or_default().insert(user);
            self.user_muted_by.entry(user).or_default().insert(author);
        }
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

        if followed_user == self.root {
            return;
        }

        if follower == self.root {
            self.follow_distance_by_user.insert(followed_user, 1);
            self.add_user_by_follow_distance(1, followed_user);
            return;
        }

        let existing = self.follow_distance_by_user.get(&followed_user).copied();
        let follower_distance = self.follow_distance_by_user.get(&follower).copied();
        let new_distance = follower_distance.map(|distance| distance.saturating_add(1));
        if let Some(distance) = new_distance
            .filter(|distance| existing.is_none() || *distance < existing.unwrap_or(u32::MAX))
        {
            self.follow_distance_by_user.insert(followed_user, distance);
            self.add_user_by_follow_distance(distance, followed_user);
        }
    }

    fn private_remove_follower(&mut self, unfollowed_user: u32, follower: u32) {
        if let Some(set) = self.followed_by_user.get_mut(&follower) {
            set.shift_remove(&unfollowed_user);
        }
        if let Some(set) = self.followers_by_user.get_mut(&unfollowed_user) {
            set.shift_remove(&follower);
        }

        if unfollowed_user == self.root {
            return;
        }

        let mut smallest = None;
        if let Some(followers) = self.followers_by_user.get(&unfollowed_user) {
            for follower in followers {
                if let Some(distance) = self.follow_distance_by_user.get(follower).copied() {
                    let candidate = distance.saturating_add(1);
                    smallest =
                        Some(smallest.map_or(candidate, |current: u32| current.min(candidate)));
                }
            }
        }

        match smallest {
            Some(distance) => {
                self.follow_distance_by_user
                    .insert(unfollowed_user, distance);
                self.add_user_by_follow_distance(distance, unfollowed_user);
            }
            None => {
                self.follow_distance_by_user.shift_remove(&unfollowed_user);
                for bucket in self.users_by_follow_distance.values_mut() {
                    bucket.shift_remove(&unfollowed_user);
                }
            }
        }
    }

    fn add_user_by_follow_distance(&mut self, distance: u32, user: u32) {
        self.users_by_follow_distance
            .entry(distance)
            .or_default()
            .insert(user);
        let keys: Vec<u32> = self.users_by_follow_distance.keys().copied().collect();
        for key in keys.into_iter().filter(|key| *key > distance) {
            if let Some(bucket) = self.users_by_follow_distance.get_mut(&key) {
                bucket.shift_remove(&user);
            }
        }
    }
}

impl SocialGraphBackend for SocialGraph {
    type Error = SocialGraphError;

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
        let bytes = hex::decode(value).expect("validated pubkey hex should decode");
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

fn read_binary_node_id(version: u64, data: &[u8], offset: &mut usize) -> Result<(String, u32)> {
    if version == BINARY_FORMAT_VERSION_V2 {
        let hex_bytes = read_bytes(data, offset, 32)?;
        let id = read_varint(data, offset)? as u32;
        return Ok((hex::encode(hex_bytes), id));
    }

    let id = read_varint(data, offset)? as u32;
    let node_id_type = read_varint(data, offset)?;
    let value = match node_id_type {
        BINARY_NODE_ID_PUBKEY => {
            let bytes = read_bytes(data, offset, 32)?;
            hex::encode(bytes)
        }
        BINARY_NODE_ID_UUID => {
            let bytes = read_bytes(data, offset, 16)?;
            Uuid::from_slice(bytes)
                .map_err(|_| SocialGraphError::InvalidBinaryNodeIdUuid)?
                .to_string()
        }
        BINARY_NODE_ID_STRING => {
            let len = read_varint(data, offset)? as usize;
            let bytes = read_bytes(data, offset, len)?;
            str::from_utf8(bytes)
                .map_err(|_| SocialGraphError::InvalidBinaryNodeIdUtf8)?
                .to_owned()
        }
        other => return Err(SocialGraphError::InvalidBinaryNodeIdType(other)),
    };
    Ok((value, id))
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(((value as u8) & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push((value & 0x7f) as u8);
}

fn read_varint(data: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*offset).ok_or(SocialGraphError::UnexpectedEof)?;
        *offset += 1;
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
