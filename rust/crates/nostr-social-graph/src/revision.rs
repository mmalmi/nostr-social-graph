/// Revision metadata is not a signature proof: callers verify events before ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRevision {
    pub id: String,
    pub created_at: u64,
    /// False for projected or locally edited lists. Do not include these in reconciliation.
    pub complete: bool,
}

pub fn valid_event_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Accept the NIP-01 winner or a complete replacement for an incomplete copy.
pub fn accepts_list_revision(
    previous_at: Option<u64>,
    previous: Option<&ListRevision>,
    at: u64,
    id: &str,
) -> bool {
    let Some(previous_at) = previous_at else {
        return true;
    };
    if at != previous_at {
        return at > previous_at;
    }
    valid_event_id(id) && previous.is_none_or(|p| id < p.id.as_str() || (id == p.id && !p.complete))
}
