//! Bounded v4 decoding for public mobile discovery. Personal and legacy imports
//! continue to use `SocialGraph::from_binary` with their existing policy.
use super::{
    SocialGraph, SocialGraphError, canonical_node_id, is_valid_pubkey, read_binary_node_id,
    read_bytes, read_count, read_varint,
};
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
    /// Read a new public discovery seed. The coverage threshold counts distinct
    /// reachable users from the published root. Applications choose their own
    /// local cache policy before calling this helper.
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
        if version != 4 {
            return Err(Invalid("expected v4"));
        }
        let count = read_count(bytes, &mut pos)?;
        if count > MAX_NODES || count < min_users {
            return Err(Invalid("node count"));
        }
        let mut ids = HashSet::with_capacity(count);
        let mut keys = HashSet::with_capacity(count);
        for _ in 0..count {
            let mut id_pos = pos;
            let id = node_id(bytes, &mut id_pos)?;
            let (key, _) = read_binary_node_id(version, bytes, &mut pos)?;
            if key.trim().is_empty()
                || !ids.insert(id)
                || !keys.insert(canonical_node_id(&key).into_owned())
            {
                return Err(Invalid("duplicate or empty identity"));
            }
        }
        if !keys.contains(canonical_node_id(root).as_ref()) {
            return Err(Invalid("missing discovery root"));
        }
        drop(keys);
        let mut remaining_edges = MAX_EDGES;
        for _ in [3, 10_000] {
            let owners = read_count(bytes, &mut pos)?;
            if owners > count {
                return Err(Invalid("too many list owners"));
            }
            let mut rows = HashSet::with_capacity(owners);
            for _ in 0..owners {
                let owner = node_id(bytes, &mut pos)?;
                if !ids.contains(&owner) || !rows.insert(owner) {
                    return Err(Invalid("unknown or repeated list owner"));
                }
                read_varint(bytes, &mut pos)?;
                match read_bytes(bytes, &mut pos, 1)?[0] {
                    0 => {}
                    1 | 2 => {
                        read_bytes(bytes, &mut pos, 32)?;
                    }
                    _ => return Err(Invalid("revision flag")),
                }
                let edges = read_count(bytes, &mut pos)?;
                if edges > count || edges > remaining_edges {
                    return Err(Invalid("edge count"));
                }
                remaining_edges -= edges;
                let mut targets = HashSet::with_capacity(edges);
                for _ in 0..edges {
                    let target = node_id(bytes, &mut pos)?;
                    if !ids.contains(&target) || !targets.insert(target) {
                        return Err(Invalid("unknown or repeated edge"));
                    }
                }
            }
        }
        if pos != bytes.len() {
            return Err(Invalid("trailing data"));
        }
        drop(ids);
        let graph = Self::from_binary(root, bytes)?;
        if graph.size().users < min_users
            || (graph.get_list_revision(root, 3).is_none()
                && graph.get_followed_by_user(root).is_empty())
        {
            return Err(Invalid("insufficient reachable graph"));
        }
        Ok(graph)
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
