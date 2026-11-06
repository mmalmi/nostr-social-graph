use super::*;
use std::collections::BTreeSet;

pub(super) const IDENTITY_SCHEMA_KEY: &str = "identity_schema";

impl HeedSocialGraph {
    // All reads and writes share the opening transaction. An error, including
    // MapFull or an ambiguous identity/list, leaves the old generation intact.
    pub(super) fn migrate_identities(&self, tx: &mut heed::RwTxn<'_>) -> Result<()> {
        match self.metadata.get(tx, IDENTITY_SCHEMA_KEY)? {
            Some([1]) => return Ok(()),
            Some(_) => return Err(HeedSocialGraphError::UnsupportedIdentitySchema),
            None => {}
        }
        let mut state = export_state_from_databases(
            tx,
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
        )?;

        // The reverse index alone loses evidence when a previous import gave
        // distinct identities the same numeric ID. Recover the union of both
        // indexes, rejecting ambiguous ownership before touching either one.
        let mut by_id = BTreeMap::<u32, String>::new();
        let mut insert = |value: &str, id| -> Result<()> {
            let value = canonical_node_id(value).into_owned();
            if let Some(previous) = by_id.insert(id, value.clone()) {
                if previous != value {
                    return Err(SocialGraphError::InvalidId(id).into());
                }
            }
            Ok(())
        };
        for (value, id) in &state.unique_ids {
            insert(value, *id)?;
        }
        for row in self.str_to_unique_id.iter(tx)? {
            let (value, id) = row?;
            insert(value, id)?;
        }
        state.unique_ids = by_id.into_iter().map(|(id, value)| (value, id)).collect();
        let state = normalize_state(state)?;
        persist_state(
            tx,
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
            &state,
        )
    }
}

pub(super) fn normalize_state(mut state: SocialGraphState) -> Result<SocialGraphState> {
    if state.root.trim().is_empty() {
        return Err(SocialGraphError::EmptyString.into());
    }
    let ids: BTreeSet<_> = state.unique_ids.iter().map(|(_, id)| *id).collect();
    // A missing index row is recoverable only if the other index supplied its
    // identity. Never silently discard an authoritative edge or revision.
    for id in state
        .followed_by_user
        .iter()
        .chain(&state.muted_by_user)
        .flat_map(|(owner, targets)| std::iter::once(owner).chain(targets))
        .chain(
            state
                .follow_list_created_at
                .iter()
                .chain(&state.mute_list_created_at)
                .map(|(id, _)| id),
        )
        .chain(state.list_revisions.iter().map(|(id, _, _)| id))
    {
        if !ids.contains(id) {
            return Err(SocialGraphError::InvalidId(*id).into());
        }
    }
    if ids.contains(&u32::MAX)
        && !state
            .unique_ids
            .iter()
            .any(|(value, _)| canonical_node_id(value) == canonical_node_id(&state.root))
    {
        return Err(HeedSocialGraphError::IdentitySpaceExhausted);
    }
    // Heed stores an empty latest list as a timestamp without a list row.
    // Materialize it before core alias resolution so a newer empty revision
    // cannot accidentally resurrect an older alias's follows or mutes.
    for (lists, timestamps) in [
        (&mut state.followed_by_user, &state.follow_list_created_at),
        (&mut state.muted_by_user, &state.mute_list_created_at),
    ] {
        let present: BTreeSet<_> = lists.iter().map(|(owner, _)| *owner).collect();
        lists.extend(
            timestamps
                .iter()
                .filter(|(owner, _)| !present.contains(owner))
                .map(|(owner, _)| (*owner, Vec::new())),
        );
    }
    // These are derived indexes, not independent authority. Rebuild even when
    // normalization only changed spelling or recovered a missing index row.
    fn reverse(lists: &[(u32, Vec<u32>)]) -> Vec<(u32, Vec<u32>)> {
        let mut out = BTreeMap::<u32, BTreeSet<u32>>::new();
        for (owner, targets) in lists {
            for target in targets {
                out.entry(*target).or_default().insert(*owner);
            }
        }
        out.into_iter()
            .map(|(id, owners)| (id, owners.into_iter().collect()))
            .collect()
    }
    state.followers_by_user = reverse(&state.followed_by_user);
    state.user_muted_by = reverse(&state.muted_by_user);
    let mut graph = SocialGraph::from_state(state)?;
    graph.recalculate_follow_distances();
    Ok(graph.export_state())
}
