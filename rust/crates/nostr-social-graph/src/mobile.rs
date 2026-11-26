//! Bounded v4/v5 decoding for public mobile discovery. Personal and legacy
//! imports continue to use `SocialGraph::from_binary` with their existing policy.
use super::{
    BINARY_FORMAT_VERSION, BINARY_FORMAT_VERSION_V5, DecodedLists, DistanceMap, EdgeSet,
    SocialGraph, SocialGraphError, UniqueIds, build_reverse_index, is_valid_pubkey, read_count,
    read_list_revision, read_typed_node_key, read_v5_identities, read_v5_lists, read_varint,
    storage::CompactRevision,
};
use indexmap::IndexMap;
use std::collections::HashSet;

pub const MOBILE_MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MOBILE_MIN_USERS: usize = 10_000;
const MAX_NODES: usize = 30_000;
const MAX_EDGES: usize = 180_000;

#[derive(Debug, thiserror::Error)]
pub enum MobileGraphError {
    #[error("invalid public mobile graph: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Decode(#[from] SocialGraphError),
}

impl SocialGraph {
    /// Read and validate a public discovery seed in a single pass. The coverage
    /// threshold counts distinct reachable users from the published root.
    /// Applications choose their own local cache policy before calling this helper.
    pub fn from_mobile_binary(
        root: &str,
        bytes: &[u8],
        min_users: usize,
    ) -> Result<Self, MobileGraphError> {
        use MobileGraphError::Invalid;
        if bytes.len() > MOBILE_MAX_BYTES || !is_valid_pubkey(root) || min_users > MAX_NODES {
            return Err(Invalid("root, size or threshold"));
        }
        let mut pos = 0;
        let version = read_varint(bytes, &mut pos)?;
        if !matches!(version, BINARY_FORMAT_VERSION | BINARY_FORMAT_VERSION_V5) {
            return Err(Invalid("expected v4 or v5"));
        }

        let mut list_revisions = IndexMap::<(u32, u32), CompactRevision>::new();
        let (
            ids,
            followed_by_user,
            followers_by_user,
            follow_list_created_at,
            muted_by_user,
            user_muted_by,
            mute_list_created_at,
        ) = if version == BINARY_FORMAT_VERSION_V5 {
            let ids = read_v5_identities(bytes, &mut pos, Some(MAX_NODES), Some(min_users))
                .map_err(map_v5_mobile_err)?;
            let mut remaining_edges = MAX_EDGES;
            let (followed, followers, follow_at) = read_v5_lists(
                bytes,
                &mut pos,
                3,
                &ids,
                &mut list_revisions,
                Some(&mut remaining_edges),
            )
            .map_err(map_v5_mobile_err)?;
            let (muted, muters, mute_at) = read_v5_lists(
                bytes,
                &mut pos,
                10_000,
                &ids,
                &mut list_revisions,
                Some(&mut remaining_edges),
            )
            .map_err(map_v5_mobile_err)?;
            (ids, followed, followers, follow_at, muted, muters, mute_at)
        } else {
            let count = read_count(bytes, &mut pos)?;
            if count > MAX_NODES || count < min_users {
                return Err(Invalid("node count"));
            }

            let mut ids = UniqueIds::with_capacity(count);
            for _ in 0..count {
                let id = node_id(bytes, &mut pos)?;
                let key = read_typed_node_key(bytes, &mut pos).map_err(|err| match err {
                    SocialGraphError::EmptyString => Invalid("duplicate or empty identity"),
                    other => MobileGraphError::Decode(other),
                })?;
                if !ids.try_insert_unique_key(key, id) {
                    return Err(Invalid("duplicate or empty identity"));
                }
            }

            let mut remaining_edges = MAX_EDGES;
            let mut seen_targets = HashSet::new();
            let mut decode_lists = |kind: u32| -> Result<DecodedLists, MobileGraphError> {
                let owners = read_count(bytes, &mut pos)?;
                if owners > count {
                    return Err(Invalid("too many list owners"));
                }
                let mut forward = IndexMap::with_capacity(owners);
                let mut timestamps = IndexMap::with_capacity(owners);
                for _ in 0..owners {
                    let owner = node_id(bytes, &mut pos)?;
                    if !ids.contains_id(owner) || forward.contains_key(&owner) {
                        return Err(Invalid("unknown or repeated list owner"));
                    }
                    let timestamp = read_varint(bytes, &mut pos)?;
                    let revision = read_list_revision(bytes, &mut pos, timestamp).map_err(
                        |err| match err {
                            SocialGraphError::InvalidBinaryNodeIdType(_) => {
                                Invalid("revision flag")
                            }
                            other => MobileGraphError::Decode(other),
                        },
                    )?;
                    if let Some(revision) = revision {
                        list_revisions.insert((owner, kind), revision);
                    }
                    let edges = read_count(bytes, &mut pos)?;
                    if edges > count || edges > remaining_edges {
                        return Err(Invalid("edge count"));
                    }
                    remaining_edges -= edges;
                    let mut targets = Vec::with_capacity(edges);
                    if edges > 32 {
                        seen_targets.clear();
                    }
                    for _ in 0..edges {
                        let target = node_id(bytes, &mut pos)?;
                        let dup = if edges <= 32 {
                            targets.contains(&target)
                        } else {
                            !seen_targets.insert(target)
                        };
                        if !ids.contains_id(target) || dup {
                            return Err(Invalid("unknown or repeated edge"));
                        }
                        targets.push(target);
                    }
                    forward.insert(owner, EdgeSet::from_vec_exact(targets));
                    timestamps.insert(owner, timestamp);
                }
                let mut reverse = IndexMap::new();
                build_reverse_index(&forward, &ids, &mut reverse);
                Ok((forward, reverse, timestamps))
            };

            let (followed, followers, follow_at) = decode_lists(3)?;
            let (muted, muters, mute_at) = decode_lists(10_000)?;
            (ids, followed, followers, follow_at, muted, muters, mute_at)
        };

        let Some(root_id) = ids.existing_id(root) else {
            return Err(Invalid("missing discovery root"));
        };
        if pos != bytes.len() {
            return Err(Invalid("trailing data"));
        }

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

        if graph.follow_distance_by_user.len() < min_users
            || (!graph.list_revisions.contains_key(&(root_id, 3))
                && graph
                    .followed_by_user
                    .get(&root_id)
                    .is_none_or(EdgeSet::is_empty))
        {
            return Err(Invalid("insufficient reachable graph"));
        }
        Ok(graph)
    }
}

fn map_v5_mobile_err(err: SocialGraphError) -> MobileGraphError {
    use MobileGraphError::Invalid;
    match err {
        SocialGraphError::EmptyString => Invalid("duplicate or empty identity"),
        SocialGraphError::InvalidId(_) => Invalid("unknown or repeated edge"),
        SocialGraphError::InvalidBinaryNodeIdType(_) => Invalid("revision flag"),
        other => MobileGraphError::Decode(other),
    }
}

fn node_id(bytes: &[u8], pos: &mut usize) -> Result<u32, MobileGraphError> {
    u32::try_from(read_varint(bytes, pos)?)
        .map_err(|_| MobileGraphError::Invalid("node ID exceeds u32"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::write_varint;

    pub(crate) fn fixture(nodes: u64, reached: u64) -> (String, Vec<u8>) {
        let root = "0".repeat(64);
        let mut bytes = vec![4];
        write_varint(&mut bytes, nodes);
        for i in 0..nodes {
            write_varint(&mut bytes, i);
            bytes.push(0);
            bytes.extend([0; 24]);
            bytes.extend(i.to_be_bytes());
        }
        // One known complete root follow list and one partial root mute list.
        bytes.extend([1, 0, 11, 2]);
        bytes.extend([0x11; 32]);
        write_varint(&mut bytes, reached - 1);
        for i in 1..reached {
            write_varint(&mut bytes, i);
        }
        bytes.extend([1, 0, 12, 1]);
        bytes.extend([0x22; 32]);
        bytes.push(0);
        (root, bytes)
    }

    #[test]
    fn coverage_counts_reachable_people_and_preserves_list_revisions() {
        let (root, valid) = fixture(10_000, 10_000);
        let graph = SocialGraph::from_mobile_binary(&root, &valid, MOBILE_MIN_USERS).unwrap();
        assert!(graph.get_list_revision(&root, 3).unwrap().complete);
        assert!(!graph.get_list_revision(&root, 10_000).unwrap().complete);
        // Round-trip via v5 mobile encoding as well.
        let v5 = graph
            .to_binary_with_budget_version(crate::BinaryBudget::default(), 5)
            .unwrap();
        assert_eq!(v5[0], 5);
        assert!(v5.len() < valid.len());
        let restored_v5 = SocialGraph::from_mobile_binary(&root, &v5, MOBILE_MIN_USERS).unwrap();
        assert_eq!(restored_v5.size().users, 10_000);
        assert!(restored_v5.get_list_revision(&root, 3).unwrap().complete);
        assert!(
            !restored_v5
                .get_list_revision(&root, 10_000)
                .unwrap()
                .complete
        );

        assert!(
            SocialGraph::from_mobile_binary(&root, &fixture(10_000, 1_000).1, MOBILE_MIN_USERS)
                .is_err()
        );
        assert!(
            SocialGraph::from_mobile_binary(&root, &fixture(1_000, 1_000).1, MOBILE_MIN_USERS)
                .is_err()
        );
    }

    #[test]
    fn public_wire_validation_rejects_invalid_rows_before_graph_construction() {
        let (root, valid) = fixture(3, 3);
        let mut duplicate = valid.clone();
        duplicate[2 + 34] = 0;
        let mut alias = valid.clone();
        alias[2 + 34 + 33] = 0;
        let mut unknown = valid.clone();
        unknown[2 + 3 * 34 + 37] = 12;
        let mut repeated_target = valid.clone();
        repeated_target[2 + 3 * 34 + 38] = 1;
        let mut extra = valid.clone();
        extra.push(0);
        let mut oversized_id = valid.clone();
        oversized_id.splice(2..3, [0x80, 0x80, 0x80, 0x80, 0x10]);
        for bytes in [
            duplicate,
            alias,
            unknown,
            repeated_target,
            extra,
            oversized_id,
            valid[..valid.len() - 1].to_vec(),
        ] {
            assert!(SocialGraph::from_mobile_binary(&root, &bytes, 1).is_err());
        }
        assert!(SocialGraph::from_mobile_binary(&"f".repeat(64), &valid, 1).is_err());
    }
}
