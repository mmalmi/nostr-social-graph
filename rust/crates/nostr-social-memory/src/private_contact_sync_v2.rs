//! Transport-neutral private registers. Caller-authenticated encrypted sibling channels own delivery.
use anyhow::{anyhow, ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{cmp::Ordering, collections::BTreeMap};

pub const PRIVATE_CONTACT_CONTROL_KIND: u16 = 10452;
pub const PRIVATE_CONTACT_SYNC_VERSION: u8 = 2;
pub const PRIVATE_CONTACT_V2_MAX_NOTE_BYTES: usize = 16_384;
pub const PRIVATE_CONTACT_V2_MAX_NICKNAME_BYTES: usize = 320;
pub const PRIVATE_CONTACT_V2_MAX_DOCUMENT_BYTES: usize = 24_576;
const MAX_COUNTER: u64 = 9_007_199_254_740_991;
const FIELDS: [&str; 4] = ["favorite", "muted", "nickname", "note"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactRegisterV2 {
    pub counter: u64,
    pub writer: String,
    pub value: Value,
}
pub type PrivateContactFieldsV2 = BTreeMap<String, PrivateContactRegisterV2>;
pub type PrivateContactPatchV2 = BTreeMap<String, Value>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactDocumentV2 {
    pub version: u8,
    pub owner: String,
    pub contact: String,
    pub fields: PrivateContactFieldsV2,
}
/// Persist the whole returned state before projection or a durable sibling-outbox handoff.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactSyncStateV2 {
    pub version: u8,
    pub owner: String,
    pub writer: String,
    pub clock: u64,
    pub contacts: BTreeMap<String, PrivateContactFieldsV2>,
    pub pending: BTreeMap<String, PrivateContactDocumentV2>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateContactValuesV2 {
    pub favorite: bool,
    pub muted: bool,
    pub nickname: Option<String>,
    pub note: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum PrivateContactControlV2 {
    #[serde(rename = "private-contact-sync")]
    Sync {
        v: u8,
        document: PrivateContactDocumentV2,
    },
    #[serde(rename = "private-contact-sync-request")]
    Request { v: u8, owner: String },
}
fn require_hex(value: &str, length: usize) -> Result<()> {
    ensure!(
        value.len() == length
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid private contact key or writer"
    );
    Ok(())
}
fn require_counter(value: u64) -> Result<()> {
    ensure!(value <= MAX_COUNTER, "invalid private contact counter");
    Ok(())
}
fn validate_value(field: &str, value: &Value) -> Result<()> {
    ensure!(FIELDS.contains(&field), "invalid private contact field");
    if field == "favorite" || field == "muted" {
        ensure!(value.is_boolean(), "private contact flag must be boolean");
    } else if !value.is_null() {
        let text = value
            .as_str()
            .ok_or_else(|| anyhow!("private contact text must be string or null"))?;
        let limit = if field == "note" {
            PRIVATE_CONTACT_V2_MAX_NOTE_BYTES
        } else {
            PRIVATE_CONTACT_V2_MAX_NICKNAME_BYTES
        };
        ensure!(text.len() <= limit, "private contact text is too large");
    }
    Ok(())
}
fn validate_fields(fields: &PrivateContactFieldsV2, legacy: bool) -> Result<()> {
    for (field, register) in fields {
        ensure!(
            !legacy || field != "muted",
            "invalid legacy private contact field"
        );
        validate_value(field, &register.value)?;
        require_counter(register.counter)?;
        require_hex(&register.writer, 32)?;
    }
    Ok(())
}
pub fn validate_private_contact_document_v2(
    document: &PrivateContactDocumentV2,
    owner: &str,
) -> Result<()> {
    require_hex(owner, 64)?;
    ensure!(
        document.version == 2 && document.owner == owner,
        "private contact account or version mismatch"
    );
    require_hex(&document.contact, 64)?;
    validate_fields(&document.fields, false)?;
    ensure!(
        !document.fields.is_empty()
            && serde_json::to_vec(document)?.len() <= PRIVATE_CONTACT_V2_MAX_DOCUMENT_BYTES,
        "invalid private contact document size"
    );
    Ok(())
}
fn compare(left: &PrivateContactRegisterV2, right: &PrivateContactRegisterV2) -> Ordering {
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
pub fn merge_private_contact_fields_v2(
    left: &PrivateContactFieldsV2,
    right: &PrivateContactFieldsV2,
) -> Result<PrivateContactFieldsV2> {
    validate_fields(left, false)?;
    validate_fields(right, false)?;
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
pub fn create_private_contact_sync_v2(
    owner: &str,
    writer: &str,
) -> Result<PrivateContactSyncStateV2> {
    require_hex(owner, 64)?;
    require_hex(writer, 32)?;
    Ok(PrivateContactSyncStateV2 {
        version: 2,
        owner: owner.into(),
        writer: writer.into(),
        clock: 0,
        contacts: BTreeMap::new(),
        pending: BTreeMap::new(),
    })
}
pub fn restore_private_contact_sync_v2(
    value: &Value,
    owner: &str,
) -> Result<PrivateContactSyncStateV2> {
    let state: PrivateContactSyncStateV2 = serde_json::from_value(value.clone())?;
    ensure!(
        state.version == 2 && state.owner == owner,
        "invalid saved private contact account or version"
    );
    require_hex(owner, 64)?;
    require_hex(&state.writer, 32)?;
    require_counter(state.clock)?;
    let mut restored = state.clone();
    for (contact, fields) in &state.contacts {
        require_hex(contact, 64)?;
        validate_fields(fields, false)?;
    }
    for (contact, document) in &state.pending {
        ensure!(
            &document.contact == contact,
            "pending private contact mismatch"
        );
        restored = merge_private_contact_document_v2(&restored, document)?;
    }
    ensure!(
        restored.contacts.values().all(|fields| fields
            .values()
            .all(|register| register.counter <= state.clock)),
        "saved private contact clock is behind its data"
    );
    Ok(restored)
}
/// Import local V1 fields/records once; old ciphertext, relay ACKs and read history are retired.
pub fn migrate_private_contact_sync_v2(
    value: &Value,
    owner: &str,
) -> Result<PrivateContactSyncStateV2> {
    if value.get("version").and_then(Value::as_u64) == Some(2) {
        return restore_private_contact_sync_v2(value, owner);
    }
    ensure!(
        value.get("version").and_then(Value::as_u64) == Some(1)
            && value.get("owner").and_then(Value::as_str) == Some(owner),
        "invalid legacy private contact account or version"
    );
    let writer = value
        .get("writer")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing legacy contact writer"))?;
    let clock = value
        .get("clock")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing legacy contact clock"))?;
    require_counter(clock)?;
    let mut state = create_private_contact_sync_v2(owner, writer)?;
    state.clock = clock;
    let contacts = value
        .get("contacts")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("missing legacy contact fields"))?;
    for (contact, fields) in contacts {
        require_hex(contact, 64)?;
        let fields: PrivateContactFieldsV2 = serde_json::from_value(fields.clone())?;
        validate_fields(&fields, true)?;
        state.contacts.insert(contact.clone(), fields);
    }
    let records = value
        .get("records")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("missing legacy contact records"))?;
    for (contact, record) in records {
        let document = record
            .get("document")
            .ok_or_else(|| anyhow!("missing legacy contact document"))?;
        ensure!(
            document.get("version").and_then(Value::as_u64) == Some(1)
                && document.get("owner").and_then(Value::as_str) == Some(owner)
                && document.get("contact").and_then(Value::as_str) == Some(contact.as_str())
                && document.get("writer").and_then(Value::as_str) == Some(writer),
            "invalid legacy contact record"
        );
        require_hex(
            document
                .get("record_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("missing legacy record id"))?,
            32,
        )?;
        let fields: PrivateContactFieldsV2 = serde_json::from_value(
            document
                .get("fields")
                .ok_or_else(|| anyhow!("missing legacy document fields"))?
                .clone(),
        )?;
        validate_fields(&fields, true)?;
        state = merge_private_contact_document_v2(
            &state,
            &PrivateContactDocumentV2 {
                version: 2,
                owner: owner.into(),
                contact: contact.clone(),
                fields,
            },
        )?;
    }
    ensure!(
        state
            .contacts
            .values()
            .all(|fields| fields.values().all(|register| register.counter <= clock)),
        "legacy contact clock is behind its data"
    );
    Ok(queue_private_contact_snapshot_v2(&state))
}
pub fn private_contact_values_v2(
    state: &PrivateContactSyncStateV2,
    contact: &str,
) -> PrivateContactValuesV2 {
    let value = |field: &str| {
        state
            .contacts
            .get(contact)
            .and_then(|fields| fields.get(field))
            .map(|register| &register.value)
    };
    PrivateContactValuesV2 {
        favorite: value("favorite").and_then(Value::as_bool).unwrap_or(false),
        muted: value("muted").and_then(Value::as_bool).unwrap_or(false),
        nickname: value("nickname").and_then(Value::as_str).map(str::to_owned),
        note: value("note").and_then(Value::as_str).map(str::to_owned),
    }
}
pub fn private_contact_documents_v2(
    state: &PrivateContactSyncStateV2,
) -> Vec<PrivateContactDocumentV2> {
    state
        .contacts
        .iter()
        .filter(|(_, fields)| !fields.is_empty())
        .map(|(contact, fields)| PrivateContactDocumentV2 {
            version: 2,
            owner: state.owner.clone(),
            contact: contact.clone(),
            fields: fields.clone(),
        })
        .collect()
}
/// Caller must authenticate both owner and active sibling; this validator does not authorize a channel.
pub fn merge_private_contact_document_v2(
    state: &PrivateContactSyncStateV2,
    document: &PrivateContactDocumentV2,
) -> Result<PrivateContactSyncStateV2> {
    validate_private_contact_document_v2(document, &state.owner)?;
    let mut next = state.clone();
    let fields = merge_private_contact_fields_v2(
        state
            .contacts
            .get(&document.contact)
            .unwrap_or(&BTreeMap::new()),
        &document.fields,
    )?;
    next.clock = next.clock.max(
        document
            .fields
            .values()
            .map(|register| register.counter)
            .max()
            .unwrap_or(0),
    );
    next.contacts.insert(document.contact.clone(), fields);
    Ok(next)
}
fn apply_patch(
    state: &PrivateContactSyncStateV2,
    contact: &str,
    patch: &PrivateContactPatchV2,
    seed: bool,
) -> Result<PrivateContactSyncStateV2> {
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
                .ok_or_else(|| anyhow!("private contact counter overflow"))?
        };
        require_counter(counter)?;
        fields.insert(
            field.clone(),
            PrivateContactRegisterV2 {
                counter,
                writer: state.writer.clone(),
                value: value.clone(),
            },
        );
    }
    if fields.is_empty() {
        return Ok(state.clone());
    }
    let document = PrivateContactDocumentV2 {
        version: 2,
        owner: state.owner.clone(),
        contact: contact.into(),
        fields: merge_private_contact_fields_v2(
            state.contacts.get(contact).unwrap_or(&BTreeMap::new()),
            &fields,
        )?,
    };
    let mut next = merge_private_contact_document_v2(state, &document)?;
    next.pending.insert(contact.into(), document);
    Ok(next)
}
pub fn edit_private_contact_v2(
    state: &PrivateContactSyncStateV2,
    contact: &str,
    patch: &PrivateContactPatchV2,
) -> Result<PrivateContactSyncStateV2> {
    apply_patch(state, contact, patch, false)
}
pub fn seed_private_contact_v2(
    state: &PrivateContactSyncStateV2,
    contact: &str,
    patch: &PrivateContactPatchV2,
) -> Result<PrivateContactSyncStateV2> {
    apply_patch(state, contact, patch, true)
}
pub fn queue_private_contact_snapshot_v2(
    state: &PrivateContactSyncStateV2,
) -> PrivateContactSyncStateV2 {
    let mut next = state.clone();
    for document in private_contact_documents_v2(state) {
        next.pending.insert(document.contact.clone(), document);
    }
    next
}
pub fn pending_private_contacts_v2(
    state: &PrivateContactSyncStateV2,
) -> Vec<PrivateContactDocumentV2> {
    state.pending.values().cloned().collect()
}
/// Acknowledge only durable sibling-outbox handoff of the exact still-current document.
pub fn acknowledge_private_contact_document_v2(
    state: &PrivateContactSyncStateV2,
    document: &PrivateContactDocumentV2,
) -> Result<PrivateContactSyncStateV2> {
    validate_private_contact_document_v2(document, &state.owner)?;
    let mut next = state.clone();
    if next.pending.get(&document.contact) == Some(document) {
        next.pending.remove(&document.contact);
    }
    Ok(next)
}
pub fn build_private_contact_control_v2(
    document: &PrivateContactDocumentV2,
) -> Result<PrivateContactControlV2> {
    validate_private_contact_document_v2(document, &document.owner)?;
    Ok(PrivateContactControlV2::Sync {
        v: 2,
        document: document.clone(),
    })
}
pub fn build_private_contact_request_v2(owner: &str) -> Result<PrivateContactControlV2> {
    require_hex(owner, 64)?;
    Ok(PrivateContactControlV2::Request {
        v: 2,
        owner: owner.into(),
    })
}
/// Parse decrypted data only after caller authentication. Does not verify signatures or authorize senders.
pub fn parse_private_contact_control_v2(
    value: &Value,
    owner: &str,
) -> Result<PrivateContactControlV2> {
    require_hex(owner, 64)?;
    let fields = value
        .as_object()
        .ok_or_else(|| anyhow!("invalid private contact control"))?;
    let allowed = if fields.get("type").and_then(Value::as_str) == Some("private-contact-sync") {
        ["type", "v", "document"]
    } else {
        ["type", "v", "owner"]
    };
    ensure!(
        fields.len() == 3 && fields.keys().all(|field| allowed.contains(&field.as_str())),
        "ambiguous private contact control"
    );
    let control: PrivateContactControlV2 = serde_json::from_value(value.clone())?;
    match &control {
        PrivateContactControlV2::Sync { v, document } => {
            ensure!(*v == 2, "unsupported private contact control");
            validate_private_contact_document_v2(document, owner)?;
        }
        PrivateContactControlV2::Request { v, owner: claimed } => ensure!(
            *v == 2 && claimed == owner,
            "private contact request account or version mismatch"
        ),
    }
    Ok(control)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("../../../../fixtures/private-contact-sync-v2.json");
    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("fixture")
    }
    fn text<'a>(value: &'a Value, field: &str) -> &'a str {
        value
            .get(field)
            .and_then(Value::as_str)
            .expect("fixture string")
    }
    #[test]
    fn interop_converges_and_retains_clears() -> Result<()> {
        let data = fixture();
        let owner = text(&data, "owner");
        let contact = text(&data, "contact");
        let docs: Vec<PrivateContactDocumentV2> =
            serde_json::from_value(data.get("documents").cloned().expect("docs"))?;
        for documents in [docs.clone(), docs.into_iter().rev().collect()] {
            let mut state = create_private_contact_sync_v2(owner, &"1".repeat(32))?;
            for document in documents {
                state = merge_private_contact_document_v2(&state, &document)?;
            }
            assert_eq!(
                serde_json::to_value(private_contact_values_v2(&state, contact))?,
                data.get("expected").cloned().expect("expected")
            );
            assert!(state.pending.is_empty());
            let queued = queue_private_contact_snapshot_v2(&state);
            assert_eq!(
                restore_private_contact_sync_v2(&serde_json::to_value(&queued)?, owner)?,
                queued
            );
        }
        Ok(())
    }
    #[test]
    fn migration_retires_relay_material_and_is_idempotent() -> Result<()> {
        let data = fixture();
        let owner = text(&data, "owner");
        let legacy = data.get("legacy_state").expect("legacy");
        let state = migrate_private_contact_sync_v2(legacy, owner)?;
        assert_eq!(
            serde_json::to_value(&state.contacts)?,
            legacy.get("contacts").cloned().expect("contacts")
        );
        assert_eq!(state.pending.len(), 1);
        let encoded = serde_json::to_value(&state)?;
        assert!(!encoded.to_string().contains("30078"));
        assert_eq!(migrate_private_contact_sync_v2(&encoded, owner)?, state);
        Ok(())
    }
    #[test]
    fn old_handoff_cannot_ack_new_edit_and_seed_cannot_resurrect_clear() -> Result<()> {
        let data = fixture();
        let owner = text(&data, "owner");
        let contact = text(&data, "contact");
        let patch = |value| BTreeMap::from([("muted".into(), Value::Bool(value))]);
        let old = seed_private_contact_v2(
            &create_private_contact_sync_v2(owner, &"1".repeat(32))?,
            contact,
            &patch(true),
        )?;
        let doc = pending_private_contacts_v2(&old)
            .into_iter()
            .next()
            .expect("pending");
        let cleared = edit_private_contact_v2(&old, contact, &patch(false))?;
        assert_eq!(
            acknowledge_private_contact_document_v2(&cleared, &doc)?,
            cleared
        );
        let merged = merge_private_contact_document_v2(&cleared, &doc)?;
        assert!(
            !private_contact_values_v2(
                &seed_private_contact_v2(&merged, contact, &patch(true))?,
                contact
            )
            .muted
        );
        Ok(())
    }
    #[test]
    fn control_account_and_version_boundary() -> Result<()> {
        let owner = "a".repeat(64);
        let request = serde_json::to_value(build_private_contact_request_v2(&owner)?)?;
        assert!(parse_private_contact_control_v2(&request, &owner).is_ok());
        assert!(parse_private_contact_control_v2(&request, &"b".repeat(64)).is_err());
        let wrong = serde_json::json!({"type":"private-contact-sync-request","v":1,"owner":owner});
        assert!(parse_private_contact_control_v2(&wrong, &owner).is_err());
        Ok(())
    }
}
