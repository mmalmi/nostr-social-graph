use anyhow::{Result, anyhow, bail};
use nostr_sdk::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Regular, append-only identity/entity fact op events.
pub const IDENTITY_OP_KIND: u16 = 7368;

/// Addressable latest-state snapshots for one UUID subject.
pub const IDENTITY_SNAPSHOT_KIND: u16 = 37_368;

const SUBJECT_MARKER: &str = "subject";
const PREV_MARKER: &str = "prev";
const REPLACE_MARKER: &str = "replace";
const DISPUTE_MARKER: &str = "dispute";
const HEAD_MARKER: &str = "head";

const RESERVED_TAGS: &[&str] = &["d", "e", "i", "p"];

/// A tag-native predicate assertion for one UUID subject.
///
/// The Nostr tag name is the predicate. Tag values are predicate-specific
/// object/argument strings. UUID and pubkey values are bare canonical strings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdentityFact {
    pub predicate: String,
    pub values: Vec<String>,
}

impl IdentityFact {
    pub fn new(predicate: impl Into<String>, values: impl IntoIterator<Item = String>) -> Self {
        Self {
            predicate: predicate.into(),
            values: values.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityOpLinks {
    pub prev: Vec<String>,
    pub replace: Vec<String>,
    pub dispute: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityOp {
    pub op_id: String,
    pub author_pubkey: String,
    pub subject: Uuid,
    pub facts: Vec<IdentityFact>,
    pub pubkeys: BTreeSet<String>,
    pub external_identifiers: BTreeSet<String>,
    pub mentioned_subjects: BTreeSet<Uuid>,
    pub prev: Vec<String>,
    pub replace: Vec<String>,
    pub dispute: Vec<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentitySnapshot {
    pub snapshot_id: String,
    pub author_pubkey: String,
    pub subject: Uuid,
    pub facts: Vec<IdentityFact>,
    pub pubkeys: BTreeSet<String>,
    pub external_identifiers: BTreeSet<String>,
    pub mentioned_subjects: BTreeSet<Uuid>,
    pub heads: Vec<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityProjection {
    pub subject: Uuid,
    pub facts: BTreeMap<String, BTreeSet<Vec<String>>>,
    pub pubkeys: BTreeSet<String>,
    pub external_identifiers: BTreeSet<String>,
    pub mentioned_subjects: BTreeSet<Uuid>,
    pub applied_op_ids: BTreeSet<String>,
    pub replaced_op_ids: BTreeSet<String>,
    pub disputed_op_ids: BTreeSet<String>,
    pub heads: BTreeSet<String>,
}

impl IdentityProjection {
    pub fn facts_for(&self, predicate: &str) -> Option<&BTreeSet<Vec<String>>> {
        self.facts.get(predicate)
    }
}

pub fn identity_fact(predicate: impl Into<String>, values: &[&str]) -> IdentityFact {
    IdentityFact::new(
        predicate,
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    )
}

pub fn build_identity_op_event(
    keys: &Keys,
    subject: Uuid,
    facts: impl IntoIterator<Item = IdentityFact>,
    prev: impl IntoIterator<Item = String>,
    created_at: u64,
) -> Result<Event> {
    build_identity_op_event_with_links(
        keys,
        subject,
        facts,
        IdentityOpLinks {
            prev: prev.into_iter().collect(),
            ..IdentityOpLinks::default()
        },
        created_at,
    )
}

pub fn build_identity_op_event_with_links(
    keys: &Keys,
    subject: Uuid,
    facts: impl IntoIterator<Item = IdentityFact>,
    links: IdentityOpLinks,
    created_at: u64,
) -> Result<Event> {
    build_identity_op_event_with_links_and_identifiers(keys, subject, facts, links, [], created_at)
}

pub fn build_identity_op_event_with_links_and_identifiers(
    keys: &Keys,
    subject: Uuid,
    facts: impl IntoIterator<Item = IdentityFact>,
    links: IdentityOpLinks,
    external_identifiers: impl IntoIterator<Item = String>,
    created_at: u64,
) -> Result<Event> {
    let facts = normalize_facts(facts)?;
    let links = normalize_links(links)?;
    let external_identifiers = normalize_external_identifiers(external_identifiers)?;
    let tags = identity_op_tags(subject, &facts, &links, &external_identifiers)?;
    EventBuilder::new(Kind::from(IDENTITY_OP_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .map_err(|error| anyhow!("failed to sign identity op event: {error}"))
}

pub fn build_identity_snapshot_event(
    keys: &Keys,
    subject: Uuid,
    facts: impl IntoIterator<Item = IdentityFact>,
    heads: impl IntoIterator<Item = String>,
    created_at: u64,
) -> Result<Event> {
    build_identity_snapshot_event_with_identifiers(keys, subject, facts, [], heads, created_at)
}

pub fn build_identity_snapshot_event_with_identifiers(
    keys: &Keys,
    subject: Uuid,
    facts: impl IntoIterator<Item = IdentityFact>,
    external_identifiers: impl IntoIterator<Item = String>,
    heads: impl IntoIterator<Item = String>,
    created_at: u64,
) -> Result<Event> {
    let facts = normalize_facts(facts)?;
    let external_identifiers = normalize_external_identifiers(external_identifiers)?;
    let heads = normalize_event_ids(heads, HEAD_MARKER)?;
    let tags = identity_snapshot_tags(subject, &facts, &external_identifiers, &heads)?;
    EventBuilder::new(Kind::from(IDENTITY_SNAPSHOT_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .map_err(|error| anyhow!("failed to sign identity snapshot event: {error}"))
}

pub fn parse_identity_op_event(event: &Event) -> Result<IdentityOp> {
    if event.kind != Kind::from(IDENTITY_OP_KIND) {
        bail!(
            "wrong identity op kind: expected {}, got {:?}",
            IDENTITY_OP_KIND,
            event.kind
        );
    }
    parse_common_event(event, false).map(|parsed| IdentityOp {
        op_id: event.id.to_hex(),
        author_pubkey: event.pubkey.to_hex(),
        subject: parsed.subject,
        facts: parsed.facts,
        pubkeys: parsed.pubkeys,
        external_identifiers: parsed.external_identifiers,
        mentioned_subjects: parsed.mentioned_subjects,
        prev: parsed.prev,
        replace: parsed.replace,
        dispute: parsed.dispute,
        created_at: event.created_at.as_secs(),
    })
}

pub fn parse_identity_snapshot_event(event: &Event) -> Result<IdentitySnapshot> {
    if event.kind != Kind::from(IDENTITY_SNAPSHOT_KIND) {
        bail!(
            "wrong identity snapshot kind: expected {}, got {:?}",
            IDENTITY_SNAPSHOT_KIND,
            event.kind
        );
    }
    let parsed = parse_common_event(event, true)?;
    let d_tag = event
        .tags
        .identifier()
        .ok_or_else(|| anyhow!("identity snapshot is missing d tag"))?;
    let d_subject = parse_uuid(d_tag)?;
    if d_subject != parsed.subject {
        bail!(
            "identity snapshot d tag {} does not match subject {}",
            d_subject,
            parsed.subject
        );
    }
    Ok(IdentitySnapshot {
        snapshot_id: event.id.to_hex(),
        author_pubkey: event.pubkey.to_hex(),
        subject: parsed.subject,
        facts: parsed.facts,
        pubkeys: parsed.pubkeys,
        external_identifiers: parsed.external_identifiers,
        mentioned_subjects: parsed.mentioned_subjects,
        heads: parsed.heads,
        created_at: event.created_at.as_secs(),
    })
}

pub fn project_identity_ops(
    subject: Uuid,
    ops: impl IntoIterator<Item = IdentityOp>,
) -> IdentityProjection {
    let mut ops: Vec<_> = ops.into_iter().filter(|op| op.subject == subject).collect();
    ops.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.op_id.cmp(&right.op_id))
    });

    let mut replaced_op_ids = BTreeSet::new();
    let mut disputed_op_ids = BTreeSet::new();
    for op in &ops {
        replaced_op_ids.extend(op.replace.iter().cloned());
        disputed_op_ids.extend(op.dispute.iter().cloned());
    }

    let mut facts: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
    let mut pubkeys = BTreeSet::new();
    let mut external_identifiers = BTreeSet::new();
    let mut mentioned_subjects = BTreeSet::new();
    let mut applied_op_ids = BTreeSet::new();
    let mut previous_ids = BTreeSet::new();

    for op in ops {
        if replaced_op_ids.contains(&op.op_id) {
            continue;
        }
        previous_ids.extend(op.prev.iter().cloned());
        pubkeys.extend(op.pubkeys.iter().cloned());
        external_identifiers.extend(op.external_identifiers.iter().cloned());
        mentioned_subjects.extend(op.mentioned_subjects.iter().copied());
        for fact in &op.facts {
            facts
                .entry(fact.predicate.clone())
                .or_default()
                .insert(fact.values.clone());
        }
        applied_op_ids.insert(op.op_id);
    }

    let heads = applied_op_ids
        .difference(&previous_ids)
        .cloned()
        .collect::<BTreeSet<_>>();

    IdentityProjection {
        subject,
        facts,
        pubkeys,
        external_identifiers,
        mentioned_subjects,
        applied_op_ids,
        replaced_op_ids,
        disputed_op_ids,
        heads,
    }
}

pub fn facts_from_projection(projection: &IdentityProjection) -> Vec<IdentityFact> {
    projection
        .facts
        .iter()
        .flat_map(|(predicate, values)| {
            values.iter().map(|value| IdentityFact {
                predicate: predicate.clone(),
                values: value.clone(),
            })
        })
        .collect()
}

fn identity_op_tags(
    subject: Uuid,
    facts: &[IdentityFact],
    links: &IdentityOpLinks,
    external_identifiers: &BTreeSet<String>,
) -> Result<Vec<Tag>> {
    let mut raw = Vec::new();
    raw.push(vec![
        "i".to_owned(),
        subject.to_string(),
        SUBJECT_MARKER.to_owned(),
    ]);
    for id in &links.prev {
        raw.push(vec![
            "e".to_owned(),
            id.clone(),
            String::new(),
            PREV_MARKER.to_owned(),
        ]);
    }
    for id in &links.replace {
        raw.push(vec![
            "e".to_owned(),
            id.clone(),
            String::new(),
            REPLACE_MARKER.to_owned(),
        ]);
    }
    for id in &links.dispute {
        raw.push(vec![
            "e".to_owned(),
            id.clone(),
            String::new(),
            DISPUTE_MARKER.to_owned(),
        ]);
    }
    raw.extend(index_tags(subject, facts, external_identifiers));
    raw.extend(facts.iter().map(fact_parts));
    raw_to_tags(raw)
}

fn identity_snapshot_tags(
    subject: Uuid,
    facts: &[IdentityFact],
    external_identifiers: &BTreeSet<String>,
    heads: &[String],
) -> Result<Vec<Tag>> {
    let mut raw = Vec::new();
    raw.push(vec!["d".to_owned(), subject.to_string()]);
    raw.push(vec![
        "i".to_owned(),
        subject.to_string(),
        SUBJECT_MARKER.to_owned(),
    ]);
    for id in heads {
        raw.push(vec![
            "e".to_owned(),
            id.clone(),
            String::new(),
            HEAD_MARKER.to_owned(),
        ]);
    }
    raw.extend(index_tags(subject, facts, external_identifiers));
    raw.extend(facts.iter().map(fact_parts));
    canonicalize_raw_tags(&mut raw);
    raw_to_tags(raw)
}

fn index_tags(
    subject: Uuid,
    facts: &[IdentityFact],
    external_identifiers: &BTreeSet<String>,
) -> Vec<Vec<String>> {
    let mut uuid_indexes = BTreeSet::new();
    let mut pubkey_indexes = BTreeSet::new();
    for fact in facts {
        for value in &fact.values {
            if let Ok(uuid) = parse_uuid(value) {
                if uuid != subject {
                    uuid_indexes.insert(uuid);
                }
            } else if let Some(pubkey) = normalize_pubkey(value) {
                pubkey_indexes.insert(pubkey);
            }
        }
    }
    for identifier in external_identifiers {
        if let Ok(uuid) = parse_uuid(identifier)
            && uuid != subject
        {
            uuid_indexes.insert(uuid);
        }
    }
    uuid_indexes
        .into_iter()
        .map(|uuid| vec!["i".to_owned(), uuid.to_string()])
        .chain(
            external_identifiers
                .iter()
                .filter(|identifier| parse_uuid(identifier).is_err())
                .map(|identifier| vec!["i".to_owned(), identifier.clone()]),
        )
        .chain(
            pubkey_indexes
                .into_iter()
                .map(|pubkey| vec!["p".to_owned(), pubkey]),
        )
        .collect()
}

fn fact_parts(fact: &IdentityFact) -> Vec<String> {
    let mut parts = Vec::with_capacity(fact.values.len() + 1);
    parts.push(fact.predicate.clone());
    parts.extend(fact.values.iter().cloned());
    parts
}

fn normalize_links(links: IdentityOpLinks) -> Result<IdentityOpLinks> {
    Ok(IdentityOpLinks {
        prev: normalize_event_ids(links.prev, PREV_MARKER)?,
        replace: normalize_event_ids(links.replace, REPLACE_MARKER)?,
        dispute: normalize_event_ids(links.dispute, DISPUTE_MARKER)?,
    })
}

fn normalize_external_identifiers(
    values: impl IntoIterator<Item = String>,
) -> Result<BTreeSet<String>> {
    let mut identifiers = BTreeSet::new();
    for value in values {
        identifiers.insert(normalize_external_identifier(&value)?);
    }
    Ok(identifiers)
}

fn normalize_external_identifier(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("external identity identifier cannot be empty");
    }
    if let Ok(uuid) = parse_uuid(trimmed) {
        return Ok(uuid.to_string());
    }
    if trimmed.chars().any(char::is_whitespace) {
        bail!("external identity identifier cannot contain whitespace: {trimmed}");
    }
    Ok(trimmed.to_lowercase())
}

fn normalize_facts(facts: impl IntoIterator<Item = IdentityFact>) -> Result<Vec<IdentityFact>> {
    let mut normalized = Vec::new();
    for fact in facts {
        let predicate = normalize_predicate(&fact.predicate)?;
        if RESERVED_TAGS.contains(&predicate.as_str()) {
            bail!("{predicate} is a reserved identity event tag");
        }
        let values = fact
            .values
            .into_iter()
            .map(normalize_value)
            .collect::<Vec<_>>();
        normalized.push(IdentityFact { predicate, values });
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_predicate(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("identity fact predicate cannot be empty");
    }
    if trimmed.chars().count() < 2 {
        bail!("identity fact predicate must be at least two characters: {trimmed}");
    }
    if trimmed.chars().any(char::is_whitespace) {
        bail!("identity fact predicate cannot contain whitespace: {trimmed}");
    }
    Ok(trimmed.to_owned())
}

fn normalize_value(value: String) -> String {
    let trimmed = value.trim();
    if let Ok(uuid) = parse_uuid(trimmed) {
        uuid.to_string()
    } else if let Some(pubkey) = normalize_pubkey(trimmed) {
        pubkey
    } else {
        trimmed.to_owned()
    }
}

fn normalize_event_ids(
    values: impl IntoIterator<Item = String>,
    role: &str,
) -> Result<Vec<String>> {
    let mut ids = BTreeSet::new();
    for value in values {
        let trimmed = value.trim().to_lowercase();
        if !is_lower_hex(&trimmed, 64) {
            bail!("invalid {role} event id: {value}");
        }
        ids.insert(trimmed);
    }
    Ok(ids.into_iter().collect())
}

fn raw_to_tags(raw: Vec<Vec<String>>) -> Result<Vec<Tag>> {
    raw.iter()
        .map(|parts| {
            Tag::parse(parts.iter().map(String::as_str))
                .map_err(|error| anyhow!("invalid identity event tag {:?}: {error}", parts))
        })
        .collect()
}

fn canonicalize_raw_tags(tags: &mut Vec<Vec<String>>) {
    tags.sort();
    tags.dedup();
}

fn parse_common_event(event: &Event, snapshot: bool) -> Result<ParsedIdentityEvent> {
    if !event.content.is_empty() {
        bail!("identity events must have empty content");
    }
    event
        .verify()
        .map_err(|error| anyhow!("identity event signature failed: {error}"))?;

    let mut subject = None;
    let mut facts = Vec::new();
    let mut pubkeys = BTreeSet::new();
    let mut external_identifiers = BTreeSet::new();
    let mut mentioned_subjects = BTreeSet::new();
    let mut prev = Vec::new();
    let mut replace = Vec::new();
    let mut dispute = Vec::new();
    let mut heads = Vec::new();

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(kind) = parts.first().map(String::as_str) else {
            continue;
        };
        match kind {
            "d" => {
                if !snapshot {
                    bail!("identity op event must not use d tag");
                }
            }
            "i" => {
                let Some(value) = parts.get(1) else {
                    bail!("identity i tag is missing value");
                };
                if parts.get(2).is_some_and(|marker| marker == SUBJECT_MARKER) {
                    let uuid = parse_uuid(value)?;
                    if subject.replace(uuid).is_some() {
                        bail!("identity event has multiple subject i tags");
                    }
                } else if let Ok(uuid) = parse_uuid(value) {
                    mentioned_subjects.insert(uuid);
                } else {
                    external_identifiers.insert(normalize_external_identifier(value)?);
                }
            }
            "p" => {
                let Some(value) = parts.get(1) else {
                    bail!("identity p tag is missing pubkey");
                };
                let pubkey = normalize_pubkey(value)
                    .ok_or_else(|| anyhow!("invalid identity p tag pubkey: {value}"))?;
                pubkeys.insert(pubkey);
            }
            "e" => {
                let Some(value) = parts.get(1) else {
                    bail!("identity e tag is missing event id");
                };
                let id = normalize_event_ids([value.to_owned()], "linked")?
                    .into_iter()
                    .next()
                    .expect("one id");
                match parts.get(3).map(String::as_str) {
                    Some(PREV_MARKER) => prev.push(id),
                    Some(REPLACE_MARKER) => replace.push(id),
                    Some(DISPUTE_MARKER) => dispute.push(id),
                    Some(HEAD_MARKER) if snapshot => heads.push(id),
                    Some(marker) => bail!("unsupported identity e tag marker: {marker}"),
                    None => bail!("identity e tag is missing marker"),
                }
            }
            _ => {
                if kind.chars().count() == 1 {
                    continue;
                }
                if snapshot && kind == "expiration" {
                    continue;
                }
                let predicate = normalize_predicate(kind)?;
                let values = parts
                    .iter()
                    .skip(1)
                    .map(|value| normalize_value(value.clone()))
                    .collect::<Vec<_>>();
                facts.push(IdentityFact { predicate, values });
            }
        }
    }

    let subject = subject.ok_or_else(|| anyhow!("identity event is missing subject i tag"))?;
    for fact in &facts {
        for value in &fact.values {
            if let Ok(uuid) = parse_uuid(value) {
                if uuid != subject {
                    mentioned_subjects.insert(uuid);
                }
            } else if let Some(pubkey) = normalize_pubkey(value) {
                pubkeys.insert(pubkey);
            }
        }
    }
    let facts = normalize_facts(facts)?;
    prev.sort();
    prev.dedup();
    replace.sort();
    replace.dedup();
    dispute.sort();
    dispute.dedup();
    heads.sort();
    heads.dedup();

    Ok(ParsedIdentityEvent {
        subject,
        facts,
        pubkeys,
        external_identifiers,
        mentioned_subjects,
        prev,
        replace,
        dispute,
        heads,
    })
}

fn parse_uuid(value: &str) -> Result<Uuid> {
    let trimmed = value.trim();
    let uuid =
        Uuid::parse_str(trimmed).map_err(|error| anyhow!("invalid UUID {value}: {error}"))?;
    if uuid.to_string() != trimmed.to_lowercase() {
        bail!("UUID must be canonical lowercase hyphenated text: {value}");
    }
    Ok(uuid)
}

fn normalize_pubkey(value: &str) -> Option<String> {
    let trimmed = value.trim().to_lowercase();
    is_lower_hex(&trimmed, 64).then_some(trimmed)
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

struct ParsedIdentityEvent {
    subject: Uuid,
    facts: Vec<IdentityFact>,
    pubkeys: BTreeSet<String>,
    external_identifiers: BTreeSet<String>,
    mentioned_subjects: BTreeSet<Uuid>,
    prev: Vec<String>,
    replace: Vec<String>,
    dispute: Vec<String>,
    heads: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBJECT: &str = "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a";
    const OTHER: &str = "9f3d3b6f-4f5e-4f8a-9d6e-7c2a7c6c9f11";
    const PUBKEY: &str = "4f355bdcb7c27f8f3e4f6c4ec8f996d45b33f2d9c93a9d5c7aa9a6e1a4e7f8aa";

    fn subject() -> Uuid {
        Uuid::parse_str(SUBJECT).unwrap()
    }

    fn fake_event_id(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[test]
    fn builds_and_parses_tag_only_identity_op() {
        let keys = Keys::generate();
        let event = build_identity_op_event_with_links_and_identifiers(
            &keys,
            subject(),
            [
                identity_fact("name", &["Alice"]),
                identity_fact("controls", &[PUBKEY]),
                identity_fact("same_as", &[OTHER]),
            ],
            IdentityOpLinks {
                prev: vec![fake_event_id(1)],
                ..IdentityOpLinks::default()
            },
            ["nip05:alice@example.com".to_owned()],
            123,
        )
        .unwrap();

        assert_eq!(event.kind, Kind::from(IDENTITY_OP_KIND));
        assert!(event.content.is_empty());

        let parsed = parse_identity_op_event(&event).unwrap();
        assert_eq!(parsed.subject, subject());
        assert_eq!(parsed.author_pubkey, keys.public_key().to_hex());
        assert_eq!(parsed.prev, vec![fake_event_id(1)]);
        assert!(parsed.pubkeys.contains(PUBKEY));
        assert!(
            parsed
                .external_identifiers
                .contains("nip05:alice@example.com")
        );
        assert!(
            parsed
                .mentioned_subjects
                .contains(&Uuid::parse_str(OTHER).unwrap())
        );
        assert!(parsed.facts.contains(&identity_fact("name", &["Alice"])));
        assert!(parsed.facts.contains(&identity_fact("controls", &[PUBKEY])));
    }

    #[test]
    fn snapshot_uses_bare_uuid_d_tag_and_canonical_tags() {
        let keys = Keys::generate();
        let head = fake_event_id(2);
        let event = build_identity_snapshot_event(
            &keys,
            subject(),
            [
                identity_fact("same_as", &[OTHER]),
                identity_fact("name", &["Alice"]),
                identity_fact("controls", &[PUBKEY]),
            ],
            [head.clone()],
            456,
        )
        .unwrap();

        assert_eq!(event.kind, Kind::from(IDENTITY_SNAPSHOT_KIND));
        assert!(event.content.is_empty());
        assert_eq!(event.tags.identifier(), Some(SUBJECT));

        let raw = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>();
        let mut sorted = raw.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(raw, sorted);

        let parsed = parse_identity_snapshot_event(&event).unwrap();
        assert_eq!(parsed.subject, subject());
        assert_eq!(parsed.heads, vec![head]);
        assert!(parsed.facts.contains(&identity_fact("same_as", &[OTHER])));
    }

    #[test]
    fn rejects_single_character_predicates() {
        let keys = Keys::generate();
        let error =
            build_identity_op_event(&keys, subject(), [identity_fact("x", &["y"])], [], 789)
                .unwrap_err()
                .to_string();
        assert!(error.contains("predicate must be at least two characters"));
    }

    #[test]
    fn projection_tracks_facts_and_author_heads() {
        let keys = Keys::generate();
        let first = parse_identity_op_event(
            &build_identity_op_event(&keys, subject(), [identity_fact("name", &["Alice"])], [], 1)
                .unwrap(),
        )
        .unwrap();
        let second = parse_identity_op_event(
            &build_identity_op_event(
                &keys,
                subject(),
                [identity_fact("picture", &["https://example.com/a.jpg"])],
                [first.op_id.clone()],
                2,
            )
            .unwrap(),
        )
        .unwrap();

        let projection = project_identity_ops(subject(), [first.clone(), second.clone()]);
        assert_eq!(projection.applied_op_ids.len(), 2);
        assert!(
            projection
                .facts_for("name")
                .unwrap()
                .contains(&vec!["Alice".to_owned()])
        );
        assert_eq!(
            projection.heads,
            [second.op_id].into_iter().collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn replaced_ops_do_not_contribute_facts() {
        let keys = Keys::generate();
        let old = parse_identity_op_event(
            &build_identity_op_event(
                &keys,
                subject(),
                [identity_fact("name", &["Alice Old"])],
                [],
                1,
            )
            .unwrap(),
        )
        .unwrap();
        let replacement = parse_identity_op_event(
            &build_identity_op_event_with_links(
                &keys,
                subject(),
                [identity_fact("name", &["Alice"])],
                IdentityOpLinks {
                    replace: vec![old.op_id.clone()],
                    ..IdentityOpLinks::default()
                },
                2,
            )
            .unwrap(),
        )
        .unwrap();

        let projection = project_identity_ops(subject(), [old, replacement.clone()]);
        assert!(projection.replaced_op_ids.contains(&replacement.replace[0]));
        assert!(
            !projection
                .facts_for("name")
                .unwrap()
                .contains(&vec!["Alice Old".to_owned()])
        );
        assert!(
            projection
                .facts_for("name")
                .unwrap()
                .contains(&vec!["Alice".to_owned()])
        );
    }
}
