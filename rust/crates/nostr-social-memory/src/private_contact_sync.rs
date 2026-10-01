//! Encrypted private preferences. Storage, authenticated sibling channels and relay delivery are caller-owned.
use anyhow::{Result, bail, ensure};
use nostr_sdk::{Event, EventBuilder, Keys, Kind, Tag, Timestamp, nips::nip44};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{cmp::Ordering, collections::BTreeMap};

pub const PRIVATE_CONTACT_SYNC_KIND: u16 = 30078;
pub const PRIVATE_CONTACT_SYNC_NAMESPACE: &str = "nostr-social-memory/v1";
pub const PRIVATE_CONTACT_MAX_NOTE_BYTES: usize = 16384;
pub const PRIVATE_CONTACT_MAX_NICKNAME_BYTES: usize = 320;
const MAX_PLAINTEXT_BYTES: usize = 24576;
const MAX_CIPHERTEXT_BYTES: usize = 40000;
const MAX_COUNTER: u64 = 9_007_199_254_740_991;
const FIELDS: [&str; 3] = ["favorite", "nickname", "note"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactRegister {
    pub counter: u64,
    pub writer: String,
    pub value: Value,
}
pub type PrivateContactFields = BTreeMap<String, PrivateContactRegister>;
pub type PrivateContactPatch = BTreeMap<String, Value>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactDocument {
    pub version: u8,
    pub owner: String,
    pub contact: String,
    pub writer: String,
    pub record_id: String,
    pub fields: PrivateContactFields,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactLocalRecord {
    pub document: PrivateContactDocument,
    pub pending: bool,
    pub event: Option<Event>,
    pub last_created_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactSyncState {
    pub version: u8,
    pub owner: String,
    pub writer: String,
    pub clock: u64,
    pub contacts: BTreeMap<String, PrivateContactFields>,
    pub records: BTreeMap<String, PrivateContactLocalRecord>,
    #[serde(default)]
    pub received_event_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactValues {
    pub favorite: bool,
    pub nickname: Option<String>,
    pub note: Option<String>,
}
#[derive(Clone, Debug)]
pub struct PreparedPrivateContactEvent {
    pub state: PrivateContactSyncState,
    pub event: Option<Event>,
    pub retry_at: Option<u64>,
}
fn require_hex(value: &str, len: usize) -> Result<()> {
    ensure!(
        value.len() == len
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid private contact key or ID"
    );
    Ok(())
}
fn require_counter(value: u64) -> Result<()> {
    ensure!(value <= MAX_COUNTER, "invalid private contact counter");
    Ok(())
}
fn validate_value(field: &str, value: &Value) -> Result<()> {
    ensure!(FIELDS.contains(&field), "invalid private contact field");
    if field == "favorite" {
        ensure!(value.is_boolean(), "favorite must be a boolean");
    } else if !value.is_null() {
        let text = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("private contact text must be string or null"))?;
        let limit = if field == "note" {
            PRIVATE_CONTACT_MAX_NOTE_BYTES
        } else {
            PRIVATE_CONTACT_MAX_NICKNAME_BYTES
        };
        ensure!(text.len() <= limit, "private contact text is too large");
    }
    Ok(())
}
fn validate_fields(fields: &PrivateContactFields) -> Result<()> {
    for (field, register) in fields {
        validate_value(field, &register.value)?;
        require_counter(register.counter)?;
        require_hex(&register.writer, 32)?;
    }
    Ok(())
}
pub fn validate_private_contact_document(
    document: &PrivateContactDocument,
    owner: &str,
) -> Result<()> {
    require_hex(owner, 64)?;
    ensure!(
        document.version == 1 && document.owner == owner,
        "private contact account or version mismatch"
    );
    require_hex(&document.contact, 64)?;
    require_hex(&document.writer, 32)?;
    require_hex(&document.record_id, 32)?;
    validate_fields(&document.fields)?;
    ensure!(
        !document.fields.is_empty() && serde_json::to_vec(document)?.len() <= MAX_PLAINTEXT_BYTES,
        "invalid private contact document size"
    );
    Ok(())
}
fn compare(left: &PrivateContactRegister, right: &PrivateContactRegister) -> Ordering {
    left.counter
        .cmp(&right.counter)
        .then_with(|| left.writer.cmp(&right.writer))
        .then_with(|| {
            left.value
                .to_string()
                .as_bytes()
                .cmp(right.value.to_string().as_bytes())
        })
}
pub fn merge_private_contact_fields(
    left: &PrivateContactFields,
    right: &PrivateContactFields,
) -> Result<PrivateContactFields> {
    validate_fields(left)?;
    validate_fields(right)?;
    let mut merged = left.clone();
    for (field, incoming) in right {
        if merged
            .get(field)
            .is_none_or(|known| compare(incoming, known).is_gt())
        {
            merged.insert(field.clone(), incoming.clone());
        }
    }
    Ok(merged)
}
pub fn create_private_contact_sync(owner: &str, writer: &str) -> Result<PrivateContactSyncState> {
    require_hex(owner, 64)?;
    require_hex(writer, 32)?;
    Ok(PrivateContactSyncState {
        version: 1,
        owner: owner.into(),
        writer: writer.into(),
        clock: 0,
        contacts: BTreeMap::new(),
        records: BTreeMap::new(),
        received_event_ids: vec![],
    })
}
pub fn remember_private_contact_event(
    state: &PrivateContactSyncState,
    event_id: &str,
) -> Result<PrivateContactSyncState> {
    require_hex(event_id, 64)?;
    let mut next = state.clone();
    if !next.received_event_ids.iter().any(|id| id == event_id) {
        next.received_event_ids.push(event_id.into());
        if next.received_event_ids.len() > 512 {
            next.received_event_ids.remove(0);
        }
    }
    Ok(next)
}
pub fn private_contact_values(
    state: &PrivateContactSyncState,
    contact: &str,
) -> PrivateContactValues {
    let value = |field: &str| {
        state
            .contacts
            .get(contact)
            .and_then(|fields| fields.get(field))
            .map(|register| &register.value)
    };
    PrivateContactValues {
        favorite: value("favorite").and_then(Value::as_bool).unwrap_or(false),
        nickname: value("nickname").and_then(Value::as_str).map(str::to_owned),
        note: value("note").and_then(Value::as_str).map(str::to_owned),
    }
}
/// Authenticated sibling snapshot only: stage at the receiver's random publication address.
pub fn private_contact_documents(state: &PrivateContactSyncState) -> Vec<PrivateContactDocument> {
    state
        .contacts
        .iter()
        .map(|(contact, fields)| PrivateContactDocument {
            version: 1,
            owner: state.owner.clone(),
            contact: contact.clone(),
            writer: state.writer.clone(),
            record_id: state
                .records
                .get(contact)
                .map_or_else(|| state.writer.clone(), |r| r.document.record_id.clone()),
            fields: fields.clone(),
        })
        .collect()
}
/// The caller must authenticate sibling documents; relay input must use open_private_contact_event first.
pub fn merge_private_contact_document(
    state: &PrivateContactSyncState,
    document: &PrivateContactDocument,
) -> Result<PrivateContactSyncState> {
    validate_private_contact_document(document, &state.owner)?;
    let mut result = state.clone();
    let fields = merge_private_contact_fields(
        state
            .contacts
            .get(&document.contact)
            .unwrap_or(&BTreeMap::new()),
        &document.fields,
    )?;
    result.clock = result.clock.max(
        document
            .fields
            .values()
            .map(|r| r.counter)
            .max()
            .unwrap_or(0),
    );
    result.contacts.insert(document.contact.clone(), fields);
    Ok(result)
}
fn save_own_fields(
    state: &PrivateContactSyncState,
    contact: &str,
    fields: &PrivateContactFields,
    record_id: Option<&str>,
) -> Result<PrivateContactSyncState> {
    let old = state.records.get(contact);
    let record_id = old
        .map(|record| record.document.record_id.as_str())
        .or(record_id)
        .ok_or_else(|| anyhow::anyhow!("new contact requires random record ID"))?;
    require_hex(record_id, 32)?;
    let document = PrivateContactDocument {
        version: 1,
        owner: state.owner.clone(),
        contact: contact.into(),
        writer: state.writer.clone(),
        record_id: record_id.into(),
        fields: merge_private_contact_fields(
            old.map(|record| &record.document.fields)
                .unwrap_or(&BTreeMap::new()),
            fields,
        )?,
    };
    let mut result = merge_private_contact_document(state, &document)?;
    result.records.insert(
        contact.into(),
        PrivateContactLocalRecord {
            document,
            pending: true,
            event: None,
            last_created_at: old.map_or(0, |r| r.last_created_at),
        },
    );
    Ok(result)
}
fn apply_patch(
    state: &PrivateContactSyncState,
    contact: &str,
    patch: &PrivateContactPatch,
    record_id: Option<&str>,
    seed: bool,
) -> Result<PrivateContactSyncState> {
    require_hex(contact, 64)?;
    let mut fields = BTreeMap::new();
    for (field, value) in patch {
        validate_value(field, value)?;
        let known = state
            .contacts
            .get(contact)
            .and_then(|fields| fields.get(field));
        if seed && (known.is_some() || value == false || value.is_null() || value == "") {
            continue;
        }
        if !seed && known.is_some_and(|known| known.value == *value) {
            continue;
        }
        let counter = if seed {
            0
        } else {
            state
                .clock
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("contact counter overflow"))?
        };
        require_counter(counter)?;
        fields.insert(
            field.clone(),
            PrivateContactRegister {
                counter,
                writer: state.writer.clone(),
                value: value.clone(),
            },
        );
    }
    if fields.is_empty() {
        Ok(state.clone())
    } else {
        save_own_fields(state, contact, &fields, record_id)
    }
}
pub fn edit_private_contact(
    state: &PrivateContactSyncState,
    contact: &str,
    patch: &PrivateContactPatch,
    record_id: Option<&str>,
) -> Result<PrivateContactSyncState> {
    apply_patch(state, contact, patch, record_id, false)
}
pub fn seed_private_contact(
    state: &PrivateContactSyncState,
    contact: &str,
    patch: &PrivateContactPatch,
    record_id: Option<&str>,
) -> Result<PrivateContactSyncState> {
    apply_patch(state, contact, patch, record_id, true)
}
/// Forward authenticated sibling registers without changing their causal stamps.
pub fn stage_private_contact_document(
    state: &PrivateContactSyncState,
    document: &PrivateContactDocument,
    record_id: Option<&str>,
) -> Result<PrivateContactSyncState> {
    let merged = merge_private_contact_document(state, document)?;
    let fields = merged
        .contacts
        .get(&document.contact)
        .ok_or_else(|| anyhow::anyhow!("missing merged private contact"))?;
    if state
        .records
        .get(&document.contact)
        .is_some_and(|r| r.document.fields == *fields)
    {
        return Ok(merged);
    }
    save_own_fields(&merged, &document.contact, fields, record_id)
}
pub fn pending_private_contacts(
    state: &PrivateContactSyncState,
) -> Vec<&PrivateContactLocalRecord> {
    state.records.values().filter(|r| r.pending).collect()
}
pub fn private_contact_sync_filter(owner: &str) -> Result<Value> {
    require_hex(owner, 64)?;
    Ok(
        serde_json::json!({ "kinds": [PRIVATE_CONTACT_SYNC_KIND], "authors": [owner], "#t": [PRIVATE_CONTACT_SYNC_NAMESPACE] }),
    )
}
fn document_tags(document: &PrivateContactDocument) -> Result<Vec<Tag>> {
    Ok(vec![
        Tag::parse([
            "d",
            &format!(
                "{}:{}:{}",
                PRIVATE_CONTACT_SYNC_NAMESPACE, document.writer, document.record_id
            ),
        ])?,
        Tag::parse(["t", PRIVATE_CONTACT_SYNC_NAMESPACE])?,
    ])
}
pub fn prepare_private_contact_event(
    state: &PrivateContactSyncState,
    contact: &str,
    keys: &Keys,
    now_secs: u64,
) -> Result<PreparedPrivateContactEvent> {
    require_counter(now_secs)?;
    let mut prepared = PreparedPrivateContactEvent {
        state: state.clone(),
        event: None,
        retry_at: None,
    };
    let Some(record) = state.records.get(contact).filter(|r| r.pending) else {
        return Ok(prepared);
    };
    ensure!(
        keys.public_key().to_hex() == state.owner,
        "private contact signer account mismatch"
    );
    if let Some(event) = &record.event {
        prepared.event = Some(event.clone());
        return Ok(prepared);
    }
    if now_secs <= record.last_created_at {
        prepared.retry_at = Some(record.last_created_at + 1);
        return Ok(prepared);
    }
    validate_private_contact_document(&record.document, &state.owner)?;
    let content = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        serde_json::to_string(&record.document)?,
        nip44::Version::V2,
    )?;
    ensure!(
        content.len() <= MAX_CIPHERTEXT_BYTES,
        "encrypted private contact is too large"
    );
    let event = EventBuilder::new(Kind::from(PRIVATE_CONTACT_SYNC_KIND), content)
        .tags(document_tags(&record.document)?)
        .custom_created_at(Timestamp::from(now_secs))
        .sign_with_keys(keys)?;
    let mut next = record.clone();
    next.event = Some(event.clone());
    next.last_created_at = now_secs;
    prepared.state.records.insert(contact.into(), next);
    prepared.event = Some(event);
    Ok(prepared)
}
/// Call only after explicit remote acceptance of this exact signed event.
pub fn acknowledge_private_contact_event(
    state: &PrivateContactSyncState,
    contact: &str,
    event_id: &str,
) -> PrivateContactSyncState {
    let mut result = state.clone();
    if let Some(record) = result.records.get_mut(contact)
        && record.pending
        && record
            .event
            .as_ref()
            .is_some_and(|event| event.id.to_hex() == event_id)
    {
        record.pending = false;
    }
    result
}
pub fn open_private_contact_event(
    event: &Event,
    owner: &str,
    keys: &Keys,
) -> Result<PrivateContactDocument> {
    require_hex(owner, 64)?;
    ensure!(
        event.pubkey.to_hex() == owner
            && event.kind == Kind::from(PRIVATE_CONTACT_SYNC_KIND)
            && event.content.len() <= MAX_CIPHERTEXT_BYTES
            && event.tags.len() == 2,
        "invalid private contact event"
    );
    event.verify()?;
    let d = event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().is_some_and(|v| v == "d"))
        .ok_or_else(|| anyhow::anyhow!("missing private contact address"))?;
    let t = event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().is_some_and(|v| v == "t"))
        .ok_or_else(|| anyhow::anyhow!("missing private contact namespace"))?;
    ensure!(
        d.as_slice().len() == 2 && t.as_slice() == ["t", PRIVATE_CONTACT_SYNC_NAMESPACE],
        "invalid private contact tags"
    );
    let address = d
        .as_slice()
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("missing private contact address"))?;
    let suffix = address
        .strip_prefix(&format!("{PRIVATE_CONTACT_SYNC_NAMESPACE}:"))
        .ok_or_else(|| anyhow::anyhow!("invalid contact namespace"))?;
    let (writer, record_id) = suffix
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid contact address"))?;
    require_hex(writer, 32)?;
    require_hex(record_id, 32)?;
    ensure!(
        keys.public_key().to_hex() == owner,
        "private contact decrypt account mismatch"
    );
    let plaintext = nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content)?;
    ensure!(
        plaintext.len() <= MAX_PLAINTEXT_BYTES,
        "private contact document is too large"
    );
    let document: PrivateContactDocument = serde_json::from_str(&plaintext)?;
    validate_private_contact_document(&document, owner)?;
    ensure!(
        document_tags(&document)?.first() == Some(d),
        "private contact address mismatch"
    );
    Ok(document)
}
pub fn restore_private_contact_sync(json: &str, owner: &str) -> Result<PrivateContactSyncState> {
    let mut state: PrivateContactSyncState = serde_json::from_str(json)?;
    require_hex(owner, 64)?;
    require_hex(&state.writer, 32)?;
    require_counter(state.clock)?;
    ensure!(
        state.version == 1 && state.owner == owner,
        "saved private contact account mismatch"
    );
    ensure!(
        state.received_event_ids.len() <= 512,
        "too many saved contact event IDs"
    );
    for id in &state.received_event_ids {
        require_hex(id, 64)?;
    }
    for (contact, fields) in &state.contacts {
        require_hex(contact, 64)?;
        validate_fields(fields)?;
    }
    for (contact, record) in &state.records {
        require_hex(contact, 64)?;
        validate_private_contact_document(&record.document, owner)?;
        require_counter(record.last_created_at)?;
        ensure!(
            &record.document.contact == contact && record.document.writer == state.writer,
            "saved private contact writer mismatch"
        );
        if let Some(event) = &record.event {
            ensure!(
                event.pubkey.to_hex() == owner
                    && event.kind == Kind::from(PRIVATE_CONTACT_SYNC_KIND)
                    && event.created_at.as_secs() == record.last_created_at
                    && event.tags.clone().to_vec() == document_tags(&record.document)?
                    && event.content.len() <= MAX_CIPHERTEXT_BYTES,
                "invalid saved private contact event"
            );
            event.verify()?;
        }
        state.contacts.insert(
            contact.clone(),
            merge_private_contact_fields(
                state.contacts.get(contact).unwrap_or(&BTreeMap::new()),
                &record.document.fields,
            )?,
        );
    }
    if state
        .contacts
        .values()
        .flat_map(|f| f.values())
        .any(|r| r.counter > state.clock)
    {
        bail!("saved private contact clock is behind data");
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../../fixtures/private-contact-sync.json"
        ))
        .unwrap()
    }
    fn patch(value: Value) -> PrivateContactPatch {
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn typescript_fixture_merges_and_decrypts_with_identical_wire_state() {
        let f = fixture();
        let owner = f["owner"].as_str().unwrap();
        let contact = f["contact"].as_str().unwrap();
        let keys = Keys::parse(f["test_secret_key"].as_str().unwrap()).unwrap();
        let mut state = create_private_contact_sync(owner, &"3".repeat(32)).unwrap();
        let documents: Vec<PrivateContactDocument> =
            serde_json::from_value(f["documents"].clone()).unwrap();
        for doc in &documents {
            state = merge_private_contact_document(&state, doc).unwrap();
        }
        assert_eq!(
            serde_json::to_value(private_contact_values(&state, contact)).unwrap(),
            f["expected"]
        );
        for doc in documents.iter().rev() {
            state = merge_private_contact_document(&state, doc).unwrap();
        }
        assert_eq!(
            serde_json::to_value(private_contact_values(&state, contact)).unwrap(),
            f["expected"]
        );
        let event: Event = serde_json::from_value(f["event"].clone()).unwrap();
        assert_eq!(
            open_private_contact_event(&event, owner, &keys).unwrap(),
            documents[2]
        );
        let restored = restore_private_contact_sync(&f["state"].to_string(), owner).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), f["state"]);
    }
    #[test]
    fn offline_edits_tombstones_and_legacy_seed_converge() {
        let f = fixture();
        let owner = f["owner"].as_str().unwrap();
        let contact = f["contact"].as_str().unwrap();
        let mut a = seed_private_contact(
            &create_private_contact_sync(owner, &"1".repeat(32)).unwrap(),
            contact,
            &patch(serde_json::json!({"favorite":true,"nickname":"Old"})),
            Some(&"a".repeat(32)),
        )
        .unwrap();
        let b = merge_private_contact_document(
            &create_private_contact_sync(owner, &"2".repeat(32)).unwrap(),
            &a.records[contact].document,
        )
        .unwrap();
        let b = edit_private_contact(
            &b,
            contact,
            &patch(serde_json::json!({"favorite":false,"nickname":null,"note":"New"})),
            Some(&"b".repeat(32)),
        )
        .unwrap();
        a = merge_private_contact_document(&a, &b.records[contact].document).unwrap();
        a = seed_private_contact(
            &a,
            contact,
            &patch(serde_json::json!({"favorite":true,"nickname":"Old"})),
            None,
        )
        .unwrap();
        assert_eq!(
            private_contact_values(&a, contact),
            PrivateContactValues {
                favorite: false,
                nickname: None,
                note: Some("New".into())
            }
        );
        assert_eq!(
            private_contact_values(
                &merge_private_contact_document(&b, &a.records[contact].document).unwrap(),
                contact
            ),
            private_contact_values(&a, contact)
        );
        let staged =
            stage_private_contact_document(&a, &b.records[contact].document, None).unwrap();
        assert_eq!(staged.clock, b.clock);
        assert_eq!(
            staged.records[contact].document.fields["note"].writer,
            b.writer
        );
    }
    #[test]
    fn durable_retry_same_second_wait_and_stale_ack_are_safe() {
        let f = fixture();
        let owner = f["owner"].as_str().unwrap();
        let contact = f["contact"].as_str().unwrap();
        let keys = Keys::parse(f["test_secret_key"].as_str().unwrap()).unwrap();
        let state = edit_private_contact(
            &create_private_contact_sync(owner, &"1".repeat(32)).unwrap(),
            contact,
            &patch(serde_json::json!({"favorite":true})),
            Some(&"a".repeat(32)),
        )
        .unwrap();
        let first = prepare_private_contact_event(&state, contact, &keys, 100).unwrap();
        assert_eq!(
            first.event,
            prepare_private_contact_event(&first.state, contact, &keys, 100)
                .unwrap()
                .event
        );
        let edited = edit_private_contact(
            &first.state,
            contact,
            &patch(serde_json::json!({"note":"Saved offline"})),
            None,
        )
        .unwrap();
        let old_ack =
            acknowledge_private_contact_event(&edited, contact, &first.event.unwrap().id.to_hex());
        assert!(old_ack.records[contact].pending);
        assert_eq!(
            prepare_private_contact_event(&old_ack, contact, &keys, 100)
                .unwrap()
                .retry_at,
            Some(101)
        );
        let next = prepare_private_contact_event(&old_ack, contact, &keys, 101).unwrap();
        assert_eq!(
            open_private_contact_event(next.event.as_ref().unwrap(), owner, &keys).unwrap(),
            next.state.records[contact].document
        );
        assert!(
            pending_private_contacts(&acknowledge_private_contact_event(
                &next.state,
                contact,
                &next.event.unwrap().id.to_hex()
            ))
            .is_empty()
        );
    }
    #[test]
    fn foreign_accounts_invalid_fields_and_bad_sizes_fail_closed() {
        let f = fixture();
        let owner = f["owner"].as_str().unwrap();
        let contact = f["contact"].as_str().unwrap();
        let keys = Keys::parse(f["test_secret_key"].as_str().unwrap()).unwrap();
        let event: Event = serde_json::from_value(f["event"].clone()).unwrap();
        assert!(open_private_contact_event(&event, contact, &keys).is_err());
        assert!(restore_private_contact_sync(&f["state"].to_string(), contact).is_err());
        let state = create_private_contact_sync(owner, &"1".repeat(32)).unwrap();
        assert!(
            edit_private_contact(
                &state,
                contact,
                &patch(serde_json::json!({"accepted_name":"Bypass"})),
                Some(&"a".repeat(32))
            )
            .is_err()
        );
        assert!(
            edit_private_contact(
                &state,
                contact,
                &patch(serde_json::json!({"note":"x".repeat(16385)})),
                Some(&"a".repeat(32))
            )
            .is_err()
        );
    }
}
