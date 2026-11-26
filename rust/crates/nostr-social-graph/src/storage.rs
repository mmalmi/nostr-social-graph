use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::str;

use indexmap::{Equivalent, IndexMap, IndexSet};
use uuid::Uuid;

use crate::{
    BINARY_NODE_ID_PUBKEY, BINARY_NODE_ID_STRING, BINARY_NODE_ID_UUID, ListRevision, Result,
    SocialGraphError, aliases, is_canonical_uuid, valid_event_id, write_varint,
};

/// Sentinel for an absent follow distance in dense storage. Distinct from
/// `DETACHED_DISTANCE` (`u32::MAX`) and any reachable hop count.
const ABSENT_DISTANCE: u32 = u32::MAX - 1;
const MAX_DENSE_SLOTS: usize = 4_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeKey {
    Pubkey([u8; 32]),
    Uuid([u8; 16]),
    String(Box<str>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKeyRef<'a> {
    Pubkey([u8; 32]),
    Uuid([u8; 16]),
    String(&'a str),
}

impl Hash for NodeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Pubkey(bytes) => {
                0u8.hash(state);
                bytes.hash(state);
            }
            Self::Uuid(bytes) => {
                1u8.hash(state);
                bytes.hash(state);
            }
            Self::String(s) => {
                2u8.hash(state);
                s.hash(state);
            }
        }
    }
}

impl Hash for NodeKeyRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Pubkey(bytes) => {
                0u8.hash(state);
                bytes.hash(state);
            }
            Self::Uuid(bytes) => {
                1u8.hash(state);
                bytes.hash(state);
            }
            Self::String(s) => {
                2u8.hash(state);
                s.hash(state);
            }
        }
    }
}

impl Equivalent<NodeKey> for NodeKeyRef<'_> {
    fn equivalent(&self, key: &NodeKey) -> bool {
        match (self, key) {
            (Self::Pubkey(a), NodeKey::Pubkey(b)) => a == b,
            (Self::Uuid(a), NodeKey::Uuid(b)) => a == b,
            (Self::String(a), NodeKey::String(b)) => *a == b.as_ref(),
            _ => false,
        }
    }
}

impl<'a> NodeKeyRef<'a> {
    pub(crate) fn parse(value: &'a str) -> Self {
        if let Some(bytes) = decode_hex32_any_case(value) {
            return Self::Pubkey(bytes);
        }
        if value.len() == 36
            && let Some(uuid) = is_canonical_uuid(value)
        {
            return Self::Uuid(*uuid.as_bytes());
        }
        Self::String(value)
    }

    pub(crate) fn to_owned(self) -> NodeKey {
        match self {
            Self::Pubkey(bytes) => NodeKey::Pubkey(bytes),
            Self::Uuid(bytes) => NodeKey::Uuid(bytes),
            Self::String(s) => NodeKey::String(s.into()),
        }
    }
}

impl NodeKey {
    pub(crate) fn to_string_key(&self) -> String {
        match self {
            Self::Pubkey(bytes) => hex::encode(bytes),
            Self::Uuid(bytes) => Uuid::from_bytes(*bytes).to_string(),
            Self::String(s) => s.to_string(),
        }
    }

    pub(crate) fn write_typed(&self, out: &mut Vec<u8>) {
        match self {
            Self::Pubkey(bytes) => {
                write_varint(out, BINARY_NODE_ID_PUBKEY);
                out.extend_from_slice(bytes);
            }
            Self::Uuid(bytes) => {
                write_varint(out, BINARY_NODE_ID_UUID);
                out.extend_from_slice(bytes);
            }
            Self::String(s) => {
                write_varint(out, BINARY_NODE_ID_STRING);
                write_varint(out, s.len() as u64);
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
}

const HEX_LUT: [u8; 256] = {
    let mut table = [0xffu8; 256];
    let mut i = 0u8;
    while i < 10 {
        table[(b'0' + i) as usize] = i;
        i += 1;
    }
    let mut j = 0u8;
    while j < 6 {
        table[(b'a' + j) as usize] = 10 + j;
        table[(b'A' + j) as usize] = 10 + j;
        j += 1;
    }
    table
};

#[inline]
pub(crate) fn decode_hex32_any_case(value: &str) -> Option<[u8; 32]> {
    let raw = value.as_bytes();
    if raw.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    let mut invalid = 0u8;
    for (out, pair) in bytes.iter_mut().zip(raw.as_chunks::<2>().0) {
        let hi = HEX_LUT[pair[0] as usize];
        let lo = HEX_LUT[pair[1] as usize];
        invalid |= hi | lo;
        *out = (hi << 4) | lo;
    }
    (invalid < 16).then_some(bytes)
}

#[derive(Debug, Clone, Default)]
struct SparseIds {
    slot_to_id: Vec<u32>,
    id_to_slot: HashMap<u32, u32>,
}

/// Compact bidirectional identity table storing 32-byte pubkeys and 16-byte
/// UUIDs inline with a single copy per node. Stays fully dense (`id == slot`)
/// unless sparse IDs are imported or nodes are pruned.
#[derive(Debug, Clone, Default)]
pub(crate) struct UniqueIds {
    keys: IndexSet<NodeKey>,
    sparse: Option<SparseIds>,
    pub(crate) current_unique_id: u32,
}

impl UniqueIds {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            keys: IndexSet::with_capacity(capacity),
            sparse: None,
            current_unique_id: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    pub(crate) fn max_id(&self) -> Option<u32> {
        if self.keys.is_empty() {
            return None;
        }
        match &self.sparse {
            None => Some((self.keys.len() - 1) as u32),
            Some(sparse) => sparse.slot_to_id.iter().copied().max(),
        }
    }

    #[inline]
    fn slot_id(&self, slot: usize) -> u32 {
        match &self.sparse {
            None => slot as u32,
            Some(sparse) => sparse.slot_to_id[slot],
        }
    }

    fn ensure_sparse(&mut self) -> &mut SparseIds {
        self.sparse.get_or_insert_with(|| {
            let len = self.keys.len();
            let mut slot_to_id = Vec::with_capacity(len);
            let mut id_to_slot = HashMap::with_capacity(len);
            for slot in 0..len {
                let id = slot as u32;
                slot_to_id.push(id);
                id_to_slot.insert(id, id);
            }
            SparseIds {
                slot_to_id,
                id_to_slot,
            }
        })
    }

    pub(crate) fn id(&mut self, value: &str) -> Result<u32> {
        let key_ref = NodeKeyRef::parse(value);
        if let Some(slot) = self.keys.get_index_of(&key_ref) {
            return Ok(self.slot_id(slot));
        }
        if value.trim().is_empty() {
            return Err(SocialGraphError::EmptyString);
        }
        let id = self.current_unique_id;
        self.current_unique_id = self.current_unique_id.saturating_add(1);
        let next_slot = self.keys.len();
        if self.sparse.is_some() || id as usize != next_slot {
            self.ensure_sparse();
            self.keys.insert(key_ref.to_owned());
            let sparse = self.sparse.as_mut().expect("sparse initialized");
            sparse.slot_to_id.push(id);
            sparse.id_to_slot.insert(id, next_slot as u32);
        } else {
            self.keys.insert(key_ref.to_owned());
        }
        Ok(id)
    }

    pub(crate) fn existing_id(&self, value: &str) -> Option<u32> {
        let key_ref = NodeKeyRef::parse(value);
        self.keys
            .get_index_of(&key_ref)
            .map(|slot| self.slot_id(slot))
    }

    pub(crate) fn contains_id(&self, id: u32) -> bool {
        match &self.sparse {
            None => (id as usize) < self.keys.len(),
            Some(sparse) => sparse.id_to_slot.contains_key(&id),
        }
    }

    pub(crate) fn key(&self, id: u32) -> Result<&NodeKey> {
        let slot = match &self.sparse {
            None => id as usize,
            Some(sparse) => *sparse
                .id_to_slot
                .get(&id)
                .ok_or(SocialGraphError::InvalidId(id))? as usize,
        };
        self.keys
            .get_index(slot)
            .ok_or(SocialGraphError::InvalidId(id))
    }

    pub(crate) fn str_owned(&self, id: u32) -> Result<String> {
        self.key(id).map(NodeKey::to_string_key)
    }

    pub(crate) fn insert_with_id(&mut self, value: &str, id: u32) {
        let key = NodeKeyRef::parse(aliases::canonical_node_id(value).as_ref()).to_owned();
        if !self.try_insert_unique_key(key, id) {
            // Only reachable if a caller passed duplicate state outside canonicalization.
            self.current_unique_id = self.current_unique_id.max(id.saturating_add(1));
        }
    }

    pub(crate) fn push_dense_unique_key(&mut self, key: NodeKey) -> bool {
        debug_assert!(self.sparse.is_none());
        let id = self.keys.len() as u32;
        if !self.keys.insert(key) {
            return false;
        }
        self.current_unique_id = self.current_unique_id.max(id.saturating_add(1));
        true
    }

    pub(crate) fn try_insert_unique_key(&mut self, key: NodeKey, id: u32) -> bool {
        if self.contains_id(id) || self.keys.contains(&key) {
            return false;
        }
        let next_slot = self.keys.len();
        if self.sparse.is_none() && id as usize == next_slot {
            self.keys.insert(key);
        } else {
            self.ensure_sparse();
            self.keys.insert(key);
            let sparse = self.sparse.as_mut().expect("sparse initialized");
            sparse.slot_to_id.push(id);
            sparse.id_to_slot.insert(id, next_slot as u32);
        }
        self.current_unique_id = self.current_unique_id.max(id.saturating_add(1));
        true
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (u32, &NodeKey)> {
        self.keys
            .iter()
            .enumerate()
            .map(move |(slot, key)| (self.slot_id(slot), key))
    }

    pub(crate) fn retain_ids(&mut self, mut keep: impl FnMut(u32) -> bool) {
        let old_keys = std::mem::take(&mut self.keys);
        let old_sparse = self.sparse.take();
        let mut new_keys = IndexSet::with_capacity(old_keys.len());
        let mut slot_to_id = Vec::with_capacity(old_keys.len());
        let mut still_dense = true;
        for (slot, key) in old_keys.into_iter().enumerate() {
            let id = match &old_sparse {
                None => slot as u32,
                Some(sparse) => sparse.slot_to_id[slot],
            };
            if keep(id) {
                let new_slot = new_keys.len();
                if id as usize != new_slot {
                    still_dense = false;
                }
                new_keys.insert(key);
                slot_to_id.push(id);
            }
        }
        self.keys = new_keys;
        if !still_dense {
            let mut id_to_slot = HashMap::with_capacity(slot_to_id.len());
            for (slot, &id) in slot_to_id.iter().enumerate() {
                id_to_slot.insert(id, slot as u32);
            }
            self.sparse = Some(SparseIds {
                slot_to_id,
                id_to_slot,
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn remove_id(&mut self, target_id: u32) -> bool {
        if !self.contains_id(target_id) {
            return false;
        }
        self.retain_ids(|id| id != target_id);
        true
    }
}

/// Compact insertion-ordered edge set. Zero heap allocation for degree 0 or 1
/// (half of all reverse follower rows and nearly all mute lists), and an exact-
/// sized `Vec<u32>` for degree >= 2.
#[derive(Debug, Clone, Default)]
pub(crate) enum EdgeSet {
    #[default]
    Empty,
    One(u32),
    Many(Vec<u32>),
}

impl EdgeSet {
    pub(crate) fn with_exact_capacity(count: usize) -> Self {
        if count <= 1 {
            Self::Empty
        } else {
            Self::Many(Vec::with_capacity(count))
        }
    }

    pub(crate) fn from_vec_exact(mut values: Vec<u32>) -> Self {
        match values.len() {
            0 => Self::Empty,
            1 => Self::One(values[0]),
            _ => {
                values.shrink_to_fit();
                Self::Many(values)
            }
        }
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[u32] {
        match self {
            Self::Empty => &[],
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values.as_slice(),
        }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    #[inline]
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, u32> {
        self.as_slice().iter()
    }

    #[inline]
    pub(crate) fn contains(&self, target: u32) -> bool {
        self.as_slice().contains(&target)
    }

    pub(crate) fn insert(&mut self, value: u32) -> bool {
        match self {
            Self::Empty => {
                *self = Self::One(value);
                true
            }
            Self::One(existing) => {
                if *existing == value {
                    false
                } else {
                    *self = Self::Many(vec![*existing, value]);
                    true
                }
            }
            Self::Many(values) => {
                if values.contains(&value) {
                    false
                } else {
                    values.push(value);
                    true
                }
            }
        }
    }

    #[inline]
    pub(crate) fn push_unchecked(&mut self, value: u32) {
        match self {
            Self::Empty => *self = Self::One(value),
            Self::One(existing) => *self = Self::Many(vec![*existing, value]),
            Self::Many(values) => values.push(value),
        }
    }

    pub(crate) fn remove(&mut self, target: u32) -> bool {
        match self {
            Self::Empty => false,
            Self::One(existing) => {
                if *existing == target {
                    *self = Self::Empty;
                    true
                } else {
                    false
                }
            }
            Self::Many(values) => {
                let Some(pos) = values.iter().position(|&v| v == target) else {
                    return false;
                };
                values.remove(pos);
                if values.len() == 1 {
                    *self = Self::One(values[0]);
                } else if values.is_empty() {
                    *self = Self::Empty;
                }
                true
            }
        }
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&u32) -> bool) {
        match self {
            Self::Empty => {}
            Self::One(existing) => {
                if !keep(existing) {
                    *self = Self::Empty;
                }
            }
            Self::Many(values) => {
                values.retain(keep);
                if values.is_empty() {
                    *self = Self::Empty;
                } else if values.len() == 1 {
                    *self = Self::One(values[0]);
                }
            }
        }
    }

    pub(crate) fn from_iter_dedup(
        iter: impl IntoIterator<Item = u32>,
        cap_hint: usize,
        scratch: &mut HashSet<u32>,
    ) -> Self {
        let mut values = Vec::with_capacity(cap_hint);
        if cap_hint <= 32 {
            for target in iter {
                if !values.contains(&target) {
                    values.push(target);
                }
            }
        } else {
            scratch.clear();
            for target in iter {
                if scratch.insert(target) {
                    values.push(target);
                }
            }
        }
        Self::from_vec_exact(values)
    }
}

impl<'a> IntoIterator for &'a EdgeSet {
    type Item = &'a u32;
    type IntoIter = std::slice::Iter<'a, u32>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Dense `Vec<u32>` distance table for compact IDs, switching cleanly to
/// `IndexMap<u32, u32>` for sparse legacy IDs so no key can ever be shadowed.
#[derive(Debug, Clone)]
pub(crate) enum DistanceMap {
    Dense { slots: Vec<u32>, len: usize },
    Sparse(IndexMap<u32, u32>),
}

impl Default for DistanceMap {
    fn default() -> Self {
        Self::Sparse(IndexMap::new())
    }
}

impl DistanceMap {
    pub(crate) fn with_dense_hint(max_id_exclusive: usize) -> Self {
        if max_id_exclusive > 0 && max_id_exclusive <= MAX_DENSE_SLOTS {
            Self::Dense {
                slots: vec![ABSENT_DISTANCE; max_id_exclusive],
                len: 0,
            }
        } else {
            Self::Sparse(IndexMap::new())
        }
    }

    pub(crate) fn clear_with_dense_hint(&mut self, max_id_exclusive: usize) {
        if max_id_exclusive > 0 && max_id_exclusive <= MAX_DENSE_SLOTS {
            match self {
                Self::Dense { slots, len } => {
                    if slots.len() >= max_id_exclusive {
                        slots.truncate(max_id_exclusive);
                        slots.fill(ABSENT_DISTANCE);
                    } else {
                        slots.fill(ABSENT_DISTANCE);
                        slots.resize(max_id_exclusive, ABSENT_DISTANCE);
                    }
                    *len = 0;
                }
                Self::Sparse(_) => {
                    *self = Self::Dense {
                        slots: vec![ABSENT_DISTANCE; max_id_exclusive],
                        len: 0,
                    };
                }
            }
        } else {
            match self {
                Self::Sparse(map) => map.clear(),
                Self::Dense { .. } => *self = Self::Sparse(IndexMap::new()),
            }
        }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Dense { len, .. } => *len,
            Self::Sparse(map) => map.len(),
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub(crate) fn get(&self, id: u32) -> Option<u32> {
        match self {
            Self::Dense { slots, .. } => slots
                .get(id as usize)
                .copied()
                .filter(|&dist| dist != ABSENT_DISTANCE),
            Self::Sparse(map) => map.get(&id).copied(),
        }
    }

    #[inline]
    pub(crate) fn contains_key(&self, id: u32) -> bool {
        self.get(id).is_some()
    }

    fn switch_to_sparse(&mut self) -> &mut IndexMap<u32, u32> {
        if let Self::Dense { slots, len } = self {
            let mut map = IndexMap::with_capacity(*len);
            for (idx, &dist) in slots.iter().enumerate() {
                if dist != ABSENT_DISTANCE {
                    map.insert(idx as u32, dist);
                }
            }
            *self = Self::Sparse(map);
        }
        match self {
            Self::Sparse(map) => map,
            Self::Dense { .. } => unreachable!(),
        }
    }

    #[inline]
    pub(crate) fn insert(&mut self, id: u32, dist: u32) {
        debug_assert_ne!(dist, ABSENT_DISTANCE);
        match self {
            Self::Dense { slots, len } => {
                let idx = id as usize;
                if idx < slots.len() {
                    if slots[idx] == ABSENT_DISTANCE {
                        *len += 1;
                    }
                    slots[idx] = dist;
                    return;
                }
                if idx < MAX_DENSE_SLOTS && idx <= slots.len().saturating_add(64) {
                    slots.resize(idx + 1, ABSENT_DISTANCE);
                    slots[idx] = dist;
                    *len += 1;
                    return;
                }
                self.switch_to_sparse().insert(id, dist);
            }
            Self::Sparse(map) => {
                map.insert(id, dist);
            }
        }
    }

    pub(crate) fn remove(&mut self, id: u32) -> Option<u32> {
        match self {
            Self::Dense { slots, len } => {
                let slot = slots.get_mut(id as usize)?;
                if *slot == ABSENT_DISTANCE {
                    return None;
                }
                let prev = *slot;
                *slot = ABSENT_DISTANCE;
                *len -= 1;
                Some(prev)
            }
            Self::Sparse(map) => map.shift_remove(&id),
        }
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(u32, u32) -> bool) {
        match self {
            Self::Dense { slots, len } => {
                for (idx, slot) in slots.iter_mut().enumerate() {
                    if *slot != ABSENT_DISTANCE && !keep(idx as u32, *slot) {
                        *slot = ABSENT_DISTANCE;
                        *len -= 1;
                    }
                }
            }
            Self::Sparse(map) => map.retain(|&id, &mut dist| keep(id, dist)),
        }
    }

    pub(crate) fn iter(&self) -> Vec<(u32, u32)> {
        match self {
            Self::Dense { slots, len } => {
                let mut out = Vec::with_capacity(*len);
                for (idx, &dist) in slots.iter().enumerate() {
                    if dist != ABSENT_DISTANCE {
                        out.push((idx as u32, dist));
                    }
                }
                out
            }
            Self::Sparse(map) => map.iter().map(|(&id, &dist)| (id, dist)).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredRevisionId {
    Hex32([u8; 32]),
    Raw(Box<str>),
}

/// Inline 32-byte event ID revision metadata (zero heap allocation for valid
/// Nostr event IDs, lossless fallback for non-hex IDs passed via `from_state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactRevision {
    id: StoredRevisionId,
    pub(crate) created_at: u64,
    pub(crate) complete: bool,
}

impl CompactRevision {
    pub(crate) fn from_bytes(id: [u8; 32], created_at: u64, complete: bool) -> Self {
        Self {
            id: StoredRevisionId::Hex32(id),
            created_at,
            complete,
        }
    }

    pub(crate) fn from_valid_hex(id: &str, created_at: u64, complete: bool) -> Self {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(id, &mut bytes).expect("validated event ID");
        Self::from_bytes(bytes, created_at, complete)
    }

    pub(crate) fn from_public(revision: ListRevision) -> Self {
        if valid_event_id(&revision.id) {
            Self::from_valid_hex(&revision.id, revision.created_at, revision.complete)
        } else {
            Self {
                id: StoredRevisionId::Raw(revision.id.into_boxed_str()),
                created_at: revision.created_at,
                complete: revision.complete,
            }
        }
    }

    pub(crate) fn to_public(&self) -> ListRevision {
        ListRevision {
            id: match &self.id {
                StoredRevisionId::Hex32(bytes) => hex::encode(bytes),
                StoredRevisionId::Raw(raw) => raw.to_string(),
            },
            created_at: self.created_at,
            complete: self.complete,
        }
    }

    pub(crate) fn as_hex32(&self) -> Option<&[u8; 32]> {
        match &self.id {
            StoredRevisionId::Hex32(bytes) => Some(bytes),
            StoredRevisionId::Raw(_) => None,
        }
    }

    /// Compare incoming valid lowercase-hex event ID `incoming` against `self.id`.
    pub(crate) fn cmp_incoming_id(&self, incoming: &str) -> Ordering {
        match &self.id {
            StoredRevisionId::Hex32(stored) => {
                let mut incoming_bytes = [0u8; 32];
                hex::decode_to_slice(incoming, &mut incoming_bytes).expect("validated event ID");
                incoming_bytes.cmp(stored)
            }
            StoredRevisionId::Raw(stored) => incoming.cmp(stored.as_ref()),
        }
    }
}

pub(crate) fn accepts_compact_revision(
    previous_at: Option<u64>,
    previous: Option<&CompactRevision>,
    at: u64,
    id: &str,
) -> bool {
    let Some(previous_at) = previous_at else {
        return true;
    };
    if at != previous_at {
        return at > previous_at;
    }
    valid_event_id(id)
        && previous.is_none_or(|p| {
            matches!(p.cmp_incoming_id(id), Ordering::Less)
                || (matches!(p.cmp_incoming_id(id), Ordering::Equal) && !p.complete)
        })
}
