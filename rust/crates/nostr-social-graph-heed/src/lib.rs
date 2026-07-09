use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};

use heed::byteorder::BigEndian;
use heed::types::{Bytes, SerdeBincode, Str, U32, U64};
use heed::{Database, Env, EnvFlags, EnvOpenOptions, RoTxn};
use nostr_social_graph::{
    GraphStats, NostrEvent, SocialGraph, SocialGraphBackend, SocialGraphError, SocialGraphState,
};

#[cfg(target_pointer_width = "64")]
const DEFAULT_MAP_SIZE: usize = 4 * 1024 * 1024 * 1024;
#[cfg(target_pointer_width = "32")]
const DEFAULT_MAP_SIZE: usize = 1024 * 1024 * 1024;
const MAX_DBS: u32 = 16;
const MAX_FUTURE_EVENT_SECONDS: u64 = 10 * 60;
const UNKNOWN_FOLLOW_DISTANCE: u32 = 1000;

const METADATA_DB: &str = "metadata";
const ROOT_KEY: &str = "root";
const NEXT_UNIQUE_ID_KEY: &str = "next_unique_id";
const SNAPSHOT_KEY: &str = "snapshot";
const STR_TO_UNIQUE_ID_DB: &str = "str_to_unique_id";
const UNIQUE_ID_TO_STR_DB: &str = "unique_id_to_str";
const FOLLOW_DISTANCE_BY_USER_DB: &str = "follow_distance_by_user";
const FOLLOWED_BY_USER_DB: &str = "followed_by_user";
const FOLLOWERS_BY_USER_DB: &str = "followers_by_user";
const FOLLOW_LIST_CREATED_AT_DB: &str = "follow_list_created_at";
const MUTED_BY_USER_DB: &str = "muted_by_user";
const USER_MUTED_BY_DB: &str = "user_muted_by";
const MUTE_LIST_CREATED_AT_DB: &str = "mute_list_created_at";
const USERS_BY_FOLLOW_DISTANCE_DB: &str = "users_by_follow_distance";

#[derive(Debug, thiserror::Error)]
pub enum HeedSocialGraphError {
    #[error(transparent)]
    Graph(#[from] SocialGraphError),
    #[error(transparent)]
    Heed(#[from] heed::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("stored root is not valid utf-8")]
    InvalidStoredRoot(#[from] std::str::Utf8Error),
    #[error("stored graph root is missing")]
    MissingRoot,
    #[error("required database {0} is missing")]
    MissingDatabase(&'static str),
    #[error("stored next unique id is invalid")]
    InvalidStoredNextUniqueId,
}

pub type Result<T> = std::result::Result<T, HeedSocialGraphError>;

pub struct HeedSocialGraph {
    env: Env,
    metadata: Database<Str, Bytes>,
    str_to_unique_id: Database<Str, U32<BigEndian>>,
    unique_id_to_str: Database<U32<BigEndian>, Str>,
    follow_distance_by_user: Database<U32<BigEndian>, U32<BigEndian>>,
    followed_by_user: Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    followers_by_user: Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    follow_list_created_at: Database<U32<BigEndian>, U64<BigEndian>>,
    muted_by_user: Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user_muted_by: Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    mute_list_created_at: Database<U32<BigEndian>, U64<BigEndian>>,
    users_by_follow_distance: Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
}

impl HeedSocialGraph {
    pub fn open<P: AsRef<Path>>(path: P, default_root: &str) -> Result<Self> {
        unsafe { Self::open_with_env_flags(path, default_root, EnvFlags::empty()) }
    }

    /// Open a graph store with explicit LMDB environment flags.
    ///
    /// # Safety
    ///
    /// The caller must uphold LMDB's safety requirements for any unsafe flags.
    /// In particular, `EnvFlags::NO_LOCK` requires external synchronization so
    /// no other process concurrently opens the same environment for writing.
    pub unsafe fn open_with_env_flags<P: AsRef<Path>>(
        path: P,
        default_root: &str,
        flags: EnvFlags,
    ) -> Result<Self> {
        unsafe {
            Self::open_with_env_flags_and_map_size(path, default_root, flags, DEFAULT_MAP_SIZE)
        }
    }

    /// Open a graph store with explicit LMDB environment flags and map size.
    ///
    /// # Safety
    ///
    /// The caller must uphold LMDB's safety requirements for any unsafe flags.
    /// In particular, `EnvFlags::NO_LOCK` requires external synchronization so
    /// no other process concurrently opens the same environment for writing.
    pub unsafe fn open_with_env_flags_and_map_size<P: AsRef<Path>>(
        path: P,
        default_root: &str,
        flags: EnvFlags,
        map_size: usize,
    ) -> Result<Self> {
        fs::create_dir_all(path.as_ref())?;
        let mut options = EnvOpenOptions::new();
        options.map_size(map_size).max_dbs(MAX_DBS);
        unsafe {
            options.flags(flags);
        }
        let env = unsafe { options.open(path.as_ref())? };

        Self::open_env(env, default_root)
    }

    fn open_env(env: Env, default_root: &str) -> Result<Self> {
        let mut wtxn = env.write_txn()?;
        let metadata = env.create_database(&mut wtxn, Some(METADATA_DB))?;
        let str_to_unique_id = env.create_database(&mut wtxn, Some(STR_TO_UNIQUE_ID_DB))?;
        let unique_id_to_str = env.create_database(&mut wtxn, Some(UNIQUE_ID_TO_STR_DB))?;
        let follow_distance_by_user =
            env.create_database(&mut wtxn, Some(FOLLOW_DISTANCE_BY_USER_DB))?;
        let followed_by_user = env.create_database(&mut wtxn, Some(FOLLOWED_BY_USER_DB))?;
        let followers_by_user = env.create_database(&mut wtxn, Some(FOLLOWERS_BY_USER_DB))?;
        let follow_list_created_at =
            env.create_database(&mut wtxn, Some(FOLLOW_LIST_CREATED_AT_DB))?;
        let muted_by_user = env.create_database(&mut wtxn, Some(MUTED_BY_USER_DB))?;
        let user_muted_by = env.create_database(&mut wtxn, Some(USER_MUTED_BY_DB))?;
        let mute_list_created_at = env.create_database(&mut wtxn, Some(MUTE_LIST_CREATED_AT_DB))?;
        let users_by_follow_distance =
            env.create_database(&mut wtxn, Some(USERS_BY_FOLLOW_DISTANCE_DB))?;

        let snapshot = metadata.get(&wtxn, SNAPSHOT_KEY)?.map(ToOwned::to_owned);
        if let Some(snapshot) = snapshot {
            let root = read_root_from_wtxn(&metadata, &wtxn)?.to_string();
            let graph = SocialGraph::from_binary(&root, &snapshot)?;
            persist_state(
                &mut wtxn,
                &metadata,
                &str_to_unique_id,
                &unique_id_to_str,
                &follow_distance_by_user,
                &followed_by_user,
                &followers_by_user,
                &follow_list_created_at,
                &muted_by_user,
                &user_muted_by,
                &mute_list_created_at,
                &users_by_follow_distance,
                &graph.export_state(),
            )?;
            metadata.delete(&mut wtxn, SNAPSHOT_KEY)?;
        }

        ensure_next_unique_id(&mut wtxn, &metadata, &unique_id_to_str)?;

        if metadata.get(&wtxn, ROOT_KEY)?.is_none() {
            initialize_root(
                &mut wtxn,
                &metadata,
                &str_to_unique_id,
                &unique_id_to_str,
                &follow_distance_by_user,
                &users_by_follow_distance,
                default_root,
            )?;
        }

        wtxn.commit()?;

        Ok(Self {
            env,
            metadata,
            str_to_unique_id,
            unique_id_to_str,
            follow_distance_by_user,
            followed_by_user,
            followers_by_user,
            follow_list_created_at,
            muted_by_user,
            user_muted_by,
            mute_list_created_at,
            users_by_follow_distance,
        })
    }

    pub fn get_root(&self) -> Result<String> {
        let rtxn = self.env.read_txn()?;
        Ok(read_root_from_rtxn(&self.metadata, &rtxn)?.to_string())
    }

    pub fn handle_event(
        &mut self,
        event: &NostrEvent,
        allow_unknown_authors: bool,
        overmute_threshold: f64,
    ) -> Result<()> {
        if !matches!(event.kind, 3 | 10_000) {
            return Ok(());
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if event.created_at > now.saturating_add(MAX_FUTURE_EVENT_SECONDS) {
            return Ok(());
        }

        let mut wtxn = self.env.write_txn()?;
        let author = match self.str_to_unique_id.get(&wtxn, &event.pubkey)? {
            Some(id) => id,
            None if allow_unknown_authors => get_or_create_id(
                &mut wtxn,
                &self.metadata,
                &self.str_to_unique_id,
                &self.unique_id_to_str,
                &event.pubkey,
            )?,
            None => return Ok(()),
        };

        if !allow_unknown_authors && self.follow_distance_by_user.get(&wtxn, &author)?.is_none() {
            return Ok(());
        }

        if is_overmuted_in_wtxn(
            &wtxn,
            &self.metadata,
            &self.str_to_unique_id,
            &self.follow_distance_by_user,
            &self.followers_by_user,
            &self.user_muted_by,
            &event.pubkey,
            overmute_threshold,
        )? {
            return Ok(());
        }

        let recalc = match event.kind {
            3 => handle_follow_list(
                &mut wtxn,
                &self.metadata,
                &self.str_to_unique_id,
                &self.unique_id_to_str,
                &self.followed_by_user,
                &self.followers_by_user,
                &self.follow_list_created_at,
                author,
                event.created_at,
                &event.tags,
            )?,
            10_000 => {
                handle_mute_list(
                    &mut wtxn,
                    &self.metadata,
                    &self.str_to_unique_id,
                    &self.unique_id_to_str,
                    &self.muted_by_user,
                    &self.user_muted_by,
                    &self.mute_list_created_at,
                    author,
                    event.created_at,
                    &event.tags,
                )?;
                false
            }
            _ => false,
        };

        wtxn.commit()?;

        if recalc {
            self.recalculate_follow_distances()?;
        }

        Ok(())
    }

    pub fn set_root(&mut self, root: &str) -> Result<()> {
        let mut wtxn = self.env.write_txn()?;
        let current_root = self.metadata.get(&wtxn, ROOT_KEY)?.map(ToOwned::to_owned);
        if current_root.as_deref() == Some(root.as_bytes()) {
            return Ok(());
        }
        let _ = get_or_create_id(
            &mut wtxn,
            &self.metadata,
            &self.str_to_unique_id,
            &self.unique_id_to_str,
            root,
        )?;
        self.metadata.put(&mut wtxn, ROOT_KEY, root.as_bytes())?;
        wtxn.commit()?;
        self.recalculate_follow_distances()
    }

    pub fn get_follow_distance(&self, user: &str) -> Result<u32> {
        let rtxn = self.env.read_txn()?;
        let Some(user_id) = self.str_to_unique_id.get(&rtxn, user)? else {
            return Ok(UNKNOWN_FOLLOW_DISTANCE);
        };
        Ok(self
            .follow_distance_by_user
            .get(&rtxn, &user_id)?
            .unwrap_or(UNKNOWN_FOLLOW_DISTANCE))
    }

    pub fn is_following(&self, follower: &str, followed_user: &str) -> Result<bool> {
        let rtxn = self.env.read_txn()?;
        let Some(follower_id) = self.str_to_unique_id.get(&rtxn, follower)? else {
            return Ok(false);
        };
        let Some(followed_id) = self.str_to_unique_id.get(&rtxn, followed_user)? else {
            return Ok(false);
        };
        Ok(self
            .followed_by_user
            .get(&rtxn, &follower_id)?
            .is_some_and(|followed| followed.contains(&followed_id)))
    }

    pub fn get_followed_by_user(&self, user: &str) -> Result<Vec<String>> {
        let rtxn = self.env.read_txn()?;
        let Some(user_id) = self.str_to_unique_id.get(&rtxn, user)? else {
            return Ok(Vec::new());
        };
        ids_to_strings(
            &rtxn,
            &self.unique_id_to_str,
            self.followed_by_user
                .get(&rtxn, &user_id)?
                .unwrap_or_default(),
        )
    }

    pub fn get_followers_by_user(&self, user: &str) -> Result<Vec<String>> {
        let rtxn = self.env.read_txn()?;
        let Some(user_id) = self.str_to_unique_id.get(&rtxn, user)? else {
            return Ok(Vec::new());
        };
        ids_to_strings(
            &rtxn,
            &self.unique_id_to_str,
            self.followers_by_user
                .get(&rtxn, &user_id)?
                .unwrap_or_default(),
        )
    }

    pub fn get_muted_by_user(&self, user: &str) -> Result<Vec<String>> {
        let rtxn = self.env.read_txn()?;
        let Some(user_id) = self.str_to_unique_id.get(&rtxn, user)? else {
            return Ok(Vec::new());
        };
        ids_to_strings(
            &rtxn,
            &self.unique_id_to_str,
            self.muted_by_user.get(&rtxn, &user_id)?.unwrap_or_default(),
        )
    }

    pub fn get_user_muted_by(&self, user: &str) -> Result<Vec<String>> {
        let rtxn = self.env.read_txn()?;
        let Some(user_id) = self.str_to_unique_id.get(&rtxn, user)? else {
            return Ok(Vec::new());
        };
        ids_to_strings(
            &rtxn,
            &self.unique_id_to_str,
            self.user_muted_by.get(&rtxn, &user_id)?.unwrap_or_default(),
        )
    }

    pub fn get_follow_list_created_at(&self, user: &str) -> Result<Option<u64>> {
        let rtxn = self.env.read_txn()?;
        let Some(user_id) = self.str_to_unique_id.get(&rtxn, user)? else {
            return Ok(None);
        };
        self.follow_list_created_at
            .get(&rtxn, &user_id)
            .map_err(Into::into)
    }

    pub fn get_mute_list_created_at(&self, user: &str) -> Result<Option<u64>> {
        let rtxn = self.env.read_txn()?;
        let Some(user_id) = self.str_to_unique_id.get(&rtxn, user)? else {
            return Ok(None);
        };
        self.mute_list_created_at
            .get(&rtxn, &user_id)
            .map_err(Into::into)
    }

    pub fn is_overmuted(&self, user: &str, threshold: f64) -> Result<bool> {
        let rtxn = self.env.read_txn()?;
        is_overmuted_in_rotxn(
            &rtxn,
            &self.metadata,
            &self.str_to_unique_id,
            &self.follow_distance_by_user,
            &self.followers_by_user,
            &self.user_muted_by,
            user,
            threshold,
        )
    }

    pub fn flush(&mut self) -> Result<()> {
        self.env.force_sync()?;
        Ok(())
    }

    pub fn has_unflushed_changes(&self) -> bool {
        false
    }

    pub fn export_state(&self) -> Result<SocialGraphState> {
        let rtxn = self.env.read_txn()?;
        export_state_from_databases(
            &rtxn,
            &self.metadata,
            &self.unique_id_to_str,
            &self.follow_distance_by_user,
            &self.users_by_follow_distance,
            &self.followed_by_user,
            &self.followers_by_user,
            &self.follow_list_created_at,
            &self.muted_by_user,
            &self.user_muted_by,
            &self.mute_list_created_at,
        )
    }

    pub fn export_state_from_path<P: AsRef<Path>>(path: P) -> Result<SocialGraphState> {
        let path = path.as_ref();
        fs::create_dir_all(path)?;
        let mut options = EnvOpenOptions::new();
        options.map_size(DEFAULT_MAP_SIZE).max_dbs(MAX_DBS);
        // The relay mirror only needs a consistent snapshot of the shared graph DB.
        // Open the LMDB env in read-only mode so it never contends for the writer lock.
        unsafe {
            options.flags(EnvFlags::READ_ONLY);
        }
        let env = unsafe { options.open(path)? };
        let rtxn = env.read_txn()?;
        let metadata = open_existing_database::<Str, Bytes>(&env, &rtxn, METADATA_DB)?;
        let unique_id_to_str =
            open_existing_database::<U32<BigEndian>, Str>(&env, &rtxn, UNIQUE_ID_TO_STR_DB)?;
        let follow_distance_by_user = open_existing_database::<U32<BigEndian>, U32<BigEndian>>(
            &env,
            &rtxn,
            FOLLOW_DISTANCE_BY_USER_DB,
        )?;
        let users_by_follow_distance = open_existing_database::<
            U32<BigEndian>,
            SerdeBincode<Vec<u32>>,
        >(&env, &rtxn, USERS_BY_FOLLOW_DISTANCE_DB)?;
        let followed_by_user = open_existing_database::<U32<BigEndian>, SerdeBincode<Vec<u32>>>(
            &env,
            &rtxn,
            FOLLOWED_BY_USER_DB,
        )?;
        let followers_by_user = open_existing_database::<U32<BigEndian>, SerdeBincode<Vec<u32>>>(
            &env,
            &rtxn,
            FOLLOWERS_BY_USER_DB,
        )?;
        let follow_list_created_at = open_existing_database::<U32<BigEndian>, U64<BigEndian>>(
            &env,
            &rtxn,
            FOLLOW_LIST_CREATED_AT_DB,
        )?;
        let muted_by_user = open_existing_database::<U32<BigEndian>, SerdeBincode<Vec<u32>>>(
            &env,
            &rtxn,
            MUTED_BY_USER_DB,
        )?;
        let user_muted_by = open_existing_database::<U32<BigEndian>, SerdeBincode<Vec<u32>>>(
            &env,
            &rtxn,
            USER_MUTED_BY_DB,
        )?;
        let mute_list_created_at = open_existing_database::<U32<BigEndian>, U64<BigEndian>>(
            &env,
            &rtxn,
            MUTE_LIST_CREATED_AT_DB,
        )?;

        export_state_from_databases(
            &rtxn,
            &metadata,
            &unique_id_to_str,
            &follow_distance_by_user,
            &users_by_follow_distance,
            &followed_by_user,
            &followers_by_user,
            &follow_list_created_at,
            &muted_by_user,
            &user_muted_by,
            &mute_list_created_at,
        )
    }

    pub fn replace_state(&mut self, state: &SocialGraphState) -> Result<()> {
        let mut wtxn = self.env.write_txn()?;
        persist_state(
            &mut wtxn,
            &self.metadata,
            &self.str_to_unique_id,
            &self.unique_id_to_str,
            &self.follow_distance_by_user,
            &self.followed_by_user,
            &self.followers_by_user,
            &self.follow_list_created_at,
            &self.muted_by_user,
            &self.user_muted_by,
            &self.mute_list_created_at,
            &self.users_by_follow_distance,
            state,
        )?;
        wtxn.commit()?;
        Ok(())
    }

    pub fn size(&self) -> Result<GraphStats> {
        let graph = SocialGraph::from_state(self.export_state()?)?;
        Ok(graph.size())
    }

    fn recalculate_follow_distances(&self) -> Result<()> {
        let rtxn = self.env.read_txn()?;
        let root = read_root_from_rtxn(&self.metadata, &rtxn)?.to_string();
        let Some(root_id) = self.str_to_unique_id.get(&rtxn, &root)? else {
            return Err(HeedSocialGraphError::MissingRoot);
        };

        let mut distances = HashMap::<u32, u32>::new();
        let mut users_by_distance = BTreeMap::<u32, Vec<u32>>::new();
        let mut queue = VecDeque::from([root_id]);
        distances.insert(root_id, 0);
        users_by_distance.insert(0, vec![root_id]);

        while let Some(user) = queue.pop_front() {
            let distance = distances[&user];
            if let Some(followed) = self.followed_by_user.get(&rtxn, &user)? {
                for target in followed {
                    if distances.contains_key(&target) {
                        continue;
                    }
                    let next_distance = distance.saturating_add(1);
                    distances.insert(target, next_distance);
                    users_by_distance
                        .entry(next_distance)
                        .or_default()
                        .push(target);
                    queue.push_back(target);
                }
            }
        }
        drop(rtxn);

        let mut wtxn = self.env.write_txn()?;
        self.follow_distance_by_user.clear(&mut wtxn)?;
        self.users_by_follow_distance.clear(&mut wtxn)?;

        for (user, distance) in distances {
            self.follow_distance_by_user
                .put(&mut wtxn, &user, &distance)?;
        }
        for (distance, users) in users_by_distance {
            self.users_by_follow_distance
                .put(&mut wtxn, &distance, &users)?;
        }

        wtxn.commit()?;
        Ok(())
    }
}

impl SocialGraphBackend for HeedSocialGraph {
    type Error = HeedSocialGraphError;

    fn get_root(&self) -> std::result::Result<String, Self::Error> {
        HeedSocialGraph::get_root(self)
    }

    fn set_root(&mut self, root: &str) -> std::result::Result<(), Self::Error> {
        HeedSocialGraph::set_root(self, root)
    }

    fn handle_event(
        &mut self,
        event: &NostrEvent,
        allow_unknown_authors: bool,
        overmute_threshold: f64,
    ) -> std::result::Result<(), Self::Error> {
        HeedSocialGraph::handle_event(self, event, allow_unknown_authors, overmute_threshold)
    }

    fn get_follow_distance(&self, user: &str) -> std::result::Result<u32, Self::Error> {
        HeedSocialGraph::get_follow_distance(self, user)
    }

    fn is_following(
        &self,
        follower: &str,
        followed_user: &str,
    ) -> std::result::Result<bool, Self::Error> {
        HeedSocialGraph::is_following(self, follower, followed_user)
    }

    fn get_followed_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        HeedSocialGraph::get_followed_by_user(self, user)
    }

    fn get_followers_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        HeedSocialGraph::get_followers_by_user(self, user)
    }

    fn get_muted_by_user(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        HeedSocialGraph::get_muted_by_user(self, user)
    }

    fn get_user_muted_by(&self, user: &str) -> std::result::Result<Vec<String>, Self::Error> {
        HeedSocialGraph::get_user_muted_by(self, user)
    }

    fn get_follow_list_created_at(
        &self,
        user: &str,
    ) -> std::result::Result<Option<u64>, Self::Error> {
        HeedSocialGraph::get_follow_list_created_at(self, user)
    }

    fn get_mute_list_created_at(
        &self,
        user: &str,
    ) -> std::result::Result<Option<u64>, Self::Error> {
        HeedSocialGraph::get_mute_list_created_at(self, user)
    }

    fn is_overmuted(&self, user: &str, threshold: f64) -> std::result::Result<bool, Self::Error> {
        HeedSocialGraph::is_overmuted(self, user, threshold)
    }

    fn flush(&mut self) -> std::result::Result<(), Self::Error> {
        HeedSocialGraph::flush(self)
    }

    fn has_unflushed_changes(&self) -> bool {
        HeedSocialGraph::has_unflushed_changes(self)
    }
}

fn read_root_from_rtxn<'a>(
    metadata: &'a Database<Str, Bytes>,
    rtxn: &'a heed::RoTxn<'_>,
) -> Result<&'a str> {
    let root = metadata
        .get(rtxn, ROOT_KEY)?
        .ok_or(HeedSocialGraphError::MissingRoot)?;
    Ok(str::from_utf8(root)?)
}

fn read_root_from_wtxn<'a>(
    metadata: &'a Database<Str, Bytes>,
    wtxn: &'a heed::RwTxn<'_>,
) -> Result<&'a str> {
    let root = metadata
        .get(wtxn, ROOT_KEY)?
        .ok_or(HeedSocialGraphError::MissingRoot)?;
    Ok(str::from_utf8(root)?)
}

#[allow(clippy::too_many_arguments)]
fn export_state_from_databases(
    rtxn: &RoTxn,
    metadata: &Database<Str, Bytes>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    follow_distance_by_user: &Database<U32<BigEndian>, U32<BigEndian>>,
    users_by_follow_distance: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    followed_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    followers_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    follow_list_created_at: &Database<U32<BigEndian>, U64<BigEndian>>,
    muted_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user_muted_by: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    mute_list_created_at: &Database<U32<BigEndian>, U64<BigEndian>>,
) -> Result<SocialGraphState> {
    let root = read_root_from_rtxn(metadata, rtxn)?.to_string();

    let unique_ids = unique_id_to_str
        .iter(rtxn)?
        .map(|entry| {
            let (id, value) = entry?;
            Ok((value.to_string(), id))
        })
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;

    let follow_distance_by_user = follow_distance_by_user
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;
    let users_by_follow_distance = users_by_follow_distance
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;
    let followed_by_user = followed_by_user
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;
    let followers_by_user = followers_by_user
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;
    let follow_list_created_at = follow_list_created_at
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;
    let muted_by_user = muted_by_user
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;
    let user_muted_by = user_muted_by
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;
    let mute_list_created_at = mute_list_created_at
        .iter(rtxn)?
        .collect::<std::result::Result<Vec<_>, heed::Error>>()?;

    Ok(SocialGraphState {
        root,
        unique_ids,
        follow_distance_by_user,
        users_by_follow_distance,
        followed_by_user,
        followers_by_user,
        follow_list_created_at,
        muted_by_user,
        user_muted_by,
        mute_list_created_at,
    })
}

fn open_existing_database<KC, DC>(
    env: &Env,
    rtxn: &RoTxn,
    name: &'static str,
) -> Result<Database<KC, DC>>
where
    KC: 'static,
    DC: 'static,
{
    env.open_database(rtxn, Some(name))?
        .ok_or(HeedSocialGraphError::MissingDatabase(name))
}

fn ids_to_strings(
    rtxn: &heed::RoTxn<'_>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    ids: Vec<u32>,
) -> Result<Vec<String>> {
    let mut values = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(value) = unique_id_to_str.get(rtxn, &id)? {
            values.push(value.to_string());
        }
    }
    Ok(values)
}

fn is_valid_pubkey(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn contains_id(ids: &[u32], target: u32) -> bool {
    ids.contains(&target)
}

fn collect_tag_ids(
    wtxn: &mut heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    author: u32,
    tags: &[Vec<String>],
) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    for tag in tags {
        if tag.first().is_none_or(|value| value != "p") {
            continue;
        }
        let Some(pubkey) = tag.get(1) else {
            continue;
        };
        if !is_valid_pubkey(pubkey) {
            continue;
        }
        let id = get_or_create_id(wtxn, metadata, str_to_unique_id, unique_id_to_str, pubkey)?;
        if id != author && !contains_id(&ids, id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

#[allow(clippy::too_many_arguments)]
fn handle_follow_list(
    wtxn: &mut heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    followed_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    followers_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    follow_list_created_at: &Database<U32<BigEndian>, U64<BigEndian>>,
    author: u32,
    created_at: u64,
    tags: &[Vec<String>],
) -> Result<bool> {
    if follow_list_created_at
        .get(wtxn, &author)?
        .is_some_and(|existing| created_at <= existing)
    {
        return Ok(false);
    }
    follow_list_created_at.put(wtxn, &author, &created_at)?;

    let followed_in_event = collect_tag_ids(
        wtxn,
        metadata,
        str_to_unique_id,
        unique_id_to_str,
        author,
        tags,
    )?;
    let current = followed_by_user.get(wtxn, &author)?.unwrap_or_default();
    if current == followed_in_event {
        return Ok(false);
    }

    for unfollowed in current
        .iter()
        .copied()
        .filter(|id| !contains_id(&followed_in_event, *id))
    {
        let mut followers = followers_by_user
            .get(wtxn, &unfollowed)?
            .unwrap_or_default();
        followers.retain(|id| *id != author);
        if followers.is_empty() {
            followers_by_user.delete(wtxn, &unfollowed)?;
        } else {
            followers_by_user.put(wtxn, &unfollowed, &followers)?;
        }
    }

    for followed in followed_in_event
        .iter()
        .copied()
        .filter(|id| !contains_id(&current, *id))
    {
        let mut followers = followers_by_user.get(wtxn, &followed)?.unwrap_or_default();
        if !contains_id(&followers, author) {
            followers.push(author);
        }
        followers_by_user.put(wtxn, &followed, &followers)?;
    }

    if followed_in_event.is_empty() {
        followed_by_user.delete(wtxn, &author)?;
    } else {
        followed_by_user.put(wtxn, &author, &followed_in_event)?;
    }

    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn handle_mute_list(
    wtxn: &mut heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    muted_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user_muted_by: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    mute_list_created_at: &Database<U32<BigEndian>, U64<BigEndian>>,
    author: u32,
    created_at: u64,
    tags: &[Vec<String>],
) -> Result<()> {
    if mute_list_created_at
        .get(wtxn, &author)?
        .is_some_and(|existing| created_at <= existing)
    {
        return Ok(());
    }
    mute_list_created_at.put(wtxn, &author, &created_at)?;

    let muted_in_event = collect_tag_ids(
        wtxn,
        metadata,
        str_to_unique_id,
        unique_id_to_str,
        author,
        tags,
    )?;
    let current = muted_by_user.get(wtxn, &author)?.unwrap_or_default();

    for unmuted in current
        .iter()
        .copied()
        .filter(|id| !contains_id(&muted_in_event, *id))
    {
        let mut muters = user_muted_by.get(wtxn, &unmuted)?.unwrap_or_default();
        muters.retain(|id| *id != author);
        if muters.is_empty() {
            user_muted_by.delete(wtxn, &unmuted)?;
        } else {
            user_muted_by.put(wtxn, &unmuted, &muters)?;
        }
    }

    for muted in muted_in_event
        .iter()
        .copied()
        .filter(|id| !contains_id(&current, *id))
    {
        let mut muters = user_muted_by.get(wtxn, &muted)?.unwrap_or_default();
        if !contains_id(&muters, author) {
            muters.push(author);
        }
        user_muted_by.put(wtxn, &muted, &muters)?;
    }

    if muted_in_event.is_empty() {
        muted_by_user.delete(wtxn, &author)?;
    } else {
        muted_by_user.put(wtxn, &author, &muted_in_event)?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn is_overmuted_in_rotxn(
    txn: &heed::RoTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    follow_distance_by_user: &Database<U32<BigEndian>, U32<BigEndian>>,
    followers_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user_muted_by: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user: &str,
    threshold: f64,
) -> Result<bool> {
    let root = match metadata.get(txn, ROOT_KEY)? {
        Some(root) => str::from_utf8(root)?,
        None => return Err(HeedSocialGraphError::MissingRoot),
    };
    if user == root {
        return Ok(false);
    }

    let Some(user_id) = str_to_unique_id.get(txn, user)? else {
        return Ok(false);
    };
    let Some(muters) = user_muted_by.get(txn, &user_id)? else {
        return Ok(false);
    };
    if muters.is_empty() {
        return Ok(false);
    }

    let root_id = str_to_unique_id
        .get(txn, root)?
        .ok_or(HeedSocialGraphError::MissingRoot)?;
    if contains_id(&muters, root_id) {
        return Ok(true);
    }

    let mut stats = BTreeMap::<u32, (u32, u32)>::new();
    if let Some(followers) = followers_by_user.get(txn, &user_id)? {
        for follower in followers {
            if let Some(distance) = follow_distance_by_user.get(txn, &follower)? {
                let entry = stats.entry(distance).or_insert((0, 0));
                entry.0 += 1;
            }
        }
    }
    for muter in muters {
        if let Some(distance) = follow_distance_by_user.get(txn, &muter)? {
            let entry = stats.entry(distance).or_insert((0, 0));
            entry.1 += 1;
        }
    }

    for (_distance, (followers, muters)) in stats {
        if followers + muters > 0 {
            return Ok((muters as f64) * threshold > followers as f64);
        }
    }

    Ok(false)
}

#[allow(clippy::too_many_arguments)]
fn is_overmuted_in_wtxn(
    txn: &heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    follow_distance_by_user: &Database<U32<BigEndian>, U32<BigEndian>>,
    followers_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user_muted_by: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user: &str,
    threshold: f64,
) -> Result<bool> {
    let root = match metadata.get(txn, ROOT_KEY)? {
        Some(root) => str::from_utf8(root)?,
        None => return Err(HeedSocialGraphError::MissingRoot),
    };
    if user == root {
        return Ok(false);
    }

    let Some(user_id) = str_to_unique_id.get(txn, user)? else {
        return Ok(false);
    };
    let Some(muters) = user_muted_by.get(txn, &user_id)? else {
        return Ok(false);
    };
    if muters.is_empty() {
        return Ok(false);
    }

    let root_id = str_to_unique_id
        .get(txn, root)?
        .ok_or(HeedSocialGraphError::MissingRoot)?;
    if contains_id(&muters, root_id) {
        return Ok(true);
    }

    let mut stats = BTreeMap::<u32, (u32, u32)>::new();
    if let Some(followers) = followers_by_user.get(txn, &user_id)? {
        for follower in followers {
            if let Some(distance) = follow_distance_by_user.get(txn, &follower)? {
                let entry = stats.entry(distance).or_insert((0, 0));
                entry.0 += 1;
            }
        }
    }
    for muter in muters {
        if let Some(distance) = follow_distance_by_user.get(txn, &muter)? {
            let entry = stats.entry(distance).or_insert((0, 0));
            entry.1 += 1;
        }
    }

    for (_distance, (followers, muters)) in stats {
        if followers + muters > 0 {
            return Ok((muters as f64) * threshold > followers as f64);
        }
    }

    Ok(false)
}

fn ensure_next_unique_id(
    wtxn: &mut heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
) -> Result<u32> {
    if let Some(bytes) = metadata.get(wtxn, NEXT_UNIQUE_ID_KEY)? {
        return decode_u32(bytes);
    }

    let mut next = 0u32;
    {
        let iter = unique_id_to_str.iter(wtxn)?;
        for entry in iter {
            let (id, _value) = entry?;
            next = next.max(id.saturating_add(1));
        }
    }
    metadata.put(wtxn, NEXT_UNIQUE_ID_KEY, &next.to_be_bytes())?;
    Ok(next)
}

fn decode_u32(bytes: &[u8]) -> Result<u32> {
    let [a, b, c, d]: [u8; 4] = bytes
        .try_into()
        .map_err(|_| HeedSocialGraphError::InvalidStoredNextUniqueId)?;
    Ok(u32::from_be_bytes([a, b, c, d]))
}

fn get_or_create_id(
    wtxn: &mut heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    value: &str,
) -> Result<u32> {
    if value.trim().is_empty() {
        return Err(SocialGraphError::EmptyString.into());
    }

    if let Some(id) = str_to_unique_id.get(wtxn, value)? {
        return Ok(id);
    }

    let next = ensure_next_unique_id(wtxn, metadata, unique_id_to_str)?;
    str_to_unique_id.put(wtxn, value, &next)?;
    unique_id_to_str.put(wtxn, &next, value)?;
    metadata.put(
        wtxn,
        NEXT_UNIQUE_ID_KEY,
        &next.saturating_add(1).to_be_bytes(),
    )?;
    Ok(next)
}

#[allow(clippy::too_many_arguments)]
fn initialize_root(
    wtxn: &mut heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    follow_distance_by_user: &Database<U32<BigEndian>, U32<BigEndian>>,
    users_by_follow_distance: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    default_root: &str,
) -> Result<()> {
    let root_id = get_or_create_id(
        wtxn,
        metadata,
        str_to_unique_id,
        unique_id_to_str,
        default_root,
    )?;
    metadata.put(wtxn, ROOT_KEY, default_root.as_bytes())?;
    follow_distance_by_user.clear(wtxn)?;
    users_by_follow_distance.clear(wtxn)?;
    follow_distance_by_user.put(wtxn, &root_id, &0)?;
    users_by_follow_distance.put(wtxn, &0, &vec![root_id])?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn persist_state(
    wtxn: &mut heed::RwTxn<'_>,
    metadata: &Database<Str, Bytes>,
    str_to_unique_id: &Database<Str, U32<BigEndian>>,
    unique_id_to_str: &Database<U32<BigEndian>, Str>,
    follow_distance_by_user: &Database<U32<BigEndian>, U32<BigEndian>>,
    followed_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    followers_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    follow_list_created_at: &Database<U32<BigEndian>, U64<BigEndian>>,
    muted_by_user: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    user_muted_by: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    mute_list_created_at: &Database<U32<BigEndian>, U64<BigEndian>>,
    users_by_follow_distance: &Database<U32<BigEndian>, SerdeBincode<Vec<u32>>>,
    state: &nostr_social_graph::SocialGraphState,
) -> Result<()> {
    metadata.clear(wtxn)?;
    str_to_unique_id.clear(wtxn)?;
    unique_id_to_str.clear(wtxn)?;
    follow_distance_by_user.clear(wtxn)?;
    followed_by_user.clear(wtxn)?;
    followers_by_user.clear(wtxn)?;
    follow_list_created_at.clear(wtxn)?;
    muted_by_user.clear(wtxn)?;
    user_muted_by.clear(wtxn)?;
    mute_list_created_at.clear(wtxn)?;
    users_by_follow_distance.clear(wtxn)?;

    metadata.put(wtxn, ROOT_KEY, state.root.as_bytes())?;
    let next = state
        .unique_ids
        .iter()
        .map(|(_value, id)| id.saturating_add(1))
        .max()
        .unwrap_or(0);
    metadata.put(wtxn, NEXT_UNIQUE_ID_KEY, &next.to_be_bytes())?;

    for (value, id) in &state.unique_ids {
        str_to_unique_id.put(wtxn, value, id)?;
        unique_id_to_str.put(wtxn, id, value)?;
    }
    for (user, distance) in &state.follow_distance_by_user {
        follow_distance_by_user.put(wtxn, user, distance)?;
    }
    for (distance, users) in &state.users_by_follow_distance {
        users_by_follow_distance.put(wtxn, distance, users)?;
    }
    for (user, followed) in &state.followed_by_user {
        followed_by_user.put(wtxn, user, followed)?;
    }
    for (user, followers) in &state.followers_by_user {
        followers_by_user.put(wtxn, user, followers)?;
    }
    for (user, created_at) in &state.follow_list_created_at {
        follow_list_created_at.put(wtxn, user, created_at)?;
    }
    for (user, muted) in &state.muted_by_user {
        muted_by_user.put(wtxn, user, muted)?;
    }
    for (user, muters) in &state.user_muted_by {
        user_muted_by.put(wtxn, user, muters)?;
    }
    for (user, created_at) in &state.mute_list_created_at {
        mute_list_created_at.put(wtxn, user, created_at)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn open_with_env_flags_keeps_requested_lmdb_flags() {
        let tempdir = tempfile::tempdir().unwrap();
        let store = unsafe {
            HeedSocialGraph::open_with_env_flags(tempdir.path(), ROOT, EnvFlags::NO_LOCK).unwrap()
        };

        let flags = store.env.flags().unwrap().unwrap_or(EnvFlags::empty());
        assert!(flags.contains(EnvFlags::NO_LOCK));
    }
}
