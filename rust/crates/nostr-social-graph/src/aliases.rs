use super::*;
use std::borrow::Cow;

// Binary pubkey encoding is case-insensitive. Apply the same identity rule in
// memory so differently cased tags cannot allocate different graph nodes.
pub(crate) fn canonical_node_id(value: &str) -> Cow<'_, str> {
    if is_valid_pubkey(value) && value.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(value.to_ascii_lowercase())
    } else {
        Cow::Borrowed(value)
    }
}

pub(crate) fn canonicalize(mut state: SocialGraphState) -> Result<(SocialGraphState, bool)> {
    let mut canonical = IndexMap::<&str, u32>::new();
    let mut by_id = IndexMap::<u32, &str>::new();
    for (value, _) in &mut state.unique_ids {
        if let Cow::Owned(normalized) = canonical_node_id(value) {
            *value = normalized;
        }
    }
    for (value, id) in &state.unique_ids {
        if value.trim().is_empty() {
            return Err(SocialGraphError::EmptyString);
        }
        if let Some(previous) = by_id.insert(*id, value.as_str()) {
            if previous != value {
                return Err(SocialGraphError::InvalidId(*id));
            }
        }
        canonical
            .entry(value.as_str())
            .and_modify(|v| *v = (*v).min(*id))
            .or_insert(*id);
    }
    state.root = canonical_node_id(&state.root).into_owned();
    let aliases: IndexMap<u32, u32> = state
        .unique_ids
        .iter()
        .filter_map(|(value, id)| {
            (canonical[value.as_str()] != *id).then_some((*id, canonical[value.as_str()]))
        })
        .collect();
    if aliases.is_empty() {
        return Ok((state, false));
    }
    state.unique_ids = canonical
        .into_iter()
        .map(|(value, id)| (value.to_owned(), id))
        .collect();
    let map = |id: u32| aliases.get(&id).copied().unwrap_or(id);
    let revisions: IndexMap<(u32, u32), ListRevision> = std::mem::take(&mut state.list_revisions)
        .into_iter()
        .map(|(owner, kind, revision)| ((owner, kind), revision))
        .collect();
    for (kind, lists, timestamps) in [
        (
            3,
            &mut state.followed_by_user,
            &mut state.follow_list_created_at,
        ),
        (
            10_000,
            &mut state.muted_by_user,
            &mut state.mute_list_created_at,
        ),
    ] {
        let times: IndexMap<u32, u64> = std::mem::take(timestamps).into_iter().collect();
        let mut owners: IndexMap<u32, Vec<u32>> = std::mem::take(lists).into_iter().collect();
        for owner in times.keys() {
            owners.entry(*owner).or_default();
        }
        // Replaceable lists are whole revisions: never union an obsolete list
        // into a newer one. Unresolved equal-time follows cannot confer trust;
        // unresolved mute ties retain every protection without claiming an ID.
        let mut chosen = IndexMap::<u32, (u32, Vec<u32>, u64, Option<ListRevision>, bool)>::new();
        for (owner, targets) in owners {
            let at = times.get(&owner).copied().unwrap_or(0);
            let revision = revisions
                .get(&(owner, kind))
                .filter(|r| r.created_at == at && valid_event_id(&r.id))
                .cloned();
            let mut entry = (owner, targets, at, revision, false);
            let key = map(owner);
            if let Some(old) = chosen.get_mut(&key) {
                if at == old.2 {
                    let resolved = matches!((&entry.3, &old.3), (Some(a), Some(b))
                        if a.id != b.id || a.complete != b.complete);
                    if !resolved {
                        let a: IndexSet<_> = entry.1.iter().copied().map(map).collect();
                        let b: IndexSet<_> = old.1.iter().copied().map(map).collect();
                        if a != b {
                            if kind == 3 {
                                old.4 = true;
                            } else {
                                old.1 = b
                                    .into_iter()
                                    .chain(a)
                                    .collect::<IndexSet<_>>()
                                    .into_iter()
                                    .collect();
                                old.3 = None;
                                continue;
                            }
                        }
                    }
                    entry.4 = old.4;
                }
            }
            let replace = chosen.get(&key).is_none_or(|old| {
                if at != old.2 {
                    return at > old.2;
                }
                match (&entry.3, &old.3) {
                    (Some(a), Some(b)) if a.id != b.id => a.id < b.id,
                    (Some(a), Some(b)) if a.complete != b.complete => a.complete,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    _ => owner < old.0,
                }
            });
            if replace {
                chosen.insert(key, entry);
            }
        }
        for (owner, (_, targets, at, revision, conflict)) in chosen {
            if conflict {
                return Err(SocialGraphError::ConflictingFollowLists(owner));
            }
            lists.push((
                owner,
                targets
                    .into_iter()
                    .map(map)
                    .collect::<IndexSet<_>>()
                    .into_iter()
                    .collect(),
            ));
            timestamps.push((owner, at));
            if let Some(revision) = revision {
                state.list_revisions.push((owner, kind, revision));
            }
        }
    }
    state.followers_by_user = reverse(&state.followed_by_user);
    state.user_muted_by = reverse(&state.muted_by_user);
    state.follow_distance_by_user.clear();
    state.users_by_follow_distance.clear();
    Ok((state, true))
}

fn reverse(lists: &[(u32, Vec<u32>)]) -> Vec<(u32, Vec<u32>)> {
    let mut reverse = IndexMap::<u32, Vec<u32>>::new();
    for (owner, targets) in lists {
        for target in targets {
            reverse.entry(*target).or_default().push(*owner);
        }
    }
    reverse.into_iter().collect()
}
