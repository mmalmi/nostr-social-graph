//! Unsigned account-private facts, authenticated only by the sibling transport.
//! Never convert these to signed public events or expose their tags in routing metadata.
use crate::{FactEventDraft, build_fact_op_draft, fact};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PRIVATE_CONTACT_FACT_CONTROL_KIND: u16 = 10456;
pub const PRIVATE_CONTACT_FACTS_VERSION: u8 = 2;
const FIELDS: [&str; 4] = ["favorite", "muted", "nickname", "note"];
const MAX_COUNTER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateContactFact {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u16,
    pub content: String,
    pub tags: Vec<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateFactRegister {
    pub counter: u64,
    pub writer: String,
    pub value: Value,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateContactFactValue {
    pub contact: String,
    pub field: String,
    pub register: PrivateFactRegister,
}
fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub fn build_private_contact_fact(
    owner: &str,
    contact: &str,
    field: &str,
    register: &PrivateFactRegister,
) -> Result<PrivateContactFact> {
    ensure!(
        valid_hex(owner, 64) && valid_hex(contact, 64) && valid_hex(&register.writer, 32),
        "Invalid private fact identity"
    );
    ensure!(
        register.counter <= MAX_COUNTER && FIELDS.contains(&field),
        "Invalid private fact stamp or field"
    );
    if field == "favorite" || field == "muted" {
        ensure!(register.value.is_boolean(), "Invalid private fact flag");
    } else if !register.value.is_null() {
        let text = register
            .value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid private fact text"))?;
        ensure!(
            text.len() <= if field == "note" { 16_384 } else { 320 },
            "Private fact too large"
        );
    }
    let draft = build_fact_op_draft(
        contact,
        [
            fact("type", &["private_contact"]),
            fact("schema", &["1"]),
            fact("counter", &[&register.counter.to_string()]),
            fact("writer", &[&register.writer]),
            fact(field, &[&register.value.to_string()]),
        ],
    )?;
    let canonical = serde_json::to_vec(&serde_json::json!([
        0,
        owner,
        0,
        draft.kind,
        draft.tags,
        draft.content
    ]))?;
    let id = format!("{:x}", Sha256::digest(canonical));
    Ok(PrivateContactFact {
        id,
        pubkey: owner.into(),
        created_at: 0,
        kind: draft.kind,
        tags: draft.tags,
        content: draft.content,
    })
}
pub fn parse_private_contact_fact(
    value: &PrivateContactFact,
    owner: &str,
) -> Result<PrivateContactFactValue> {
    ensure!(
        value.created_at == 0
            && value.pubkey == owner
            && value.kind == 7368
            && value.content.is_empty()
            && value.tags.len() == 6,
        "Invalid private fact envelope"
    );
    let tag = |name: &str| {
        value
            .tags
            .iter()
            .find(|tag| tag.first().is_some_and(|key| key == name))
            .and_then(|tag| tag.get(1))
    };
    let contact = tag("i").ok_or_else(|| anyhow::anyhow!("Missing private fact subject"))?;
    let field = FIELDS
        .iter()
        .find(|field| tag(field).is_some())
        .ok_or_else(|| anyhow::anyhow!("Missing private fact field"))?;
    let register = PrivateFactRegister {
        counter: tag("counter")
            .ok_or_else(|| anyhow::anyhow!("Missing counter"))?
            .parse()?,
        writer: tag("writer")
            .ok_or_else(|| anyhow::anyhow!("Missing writer"))?
            .clone(),
        value: serde_json::from_str(tag(field).ok_or_else(|| anyhow::anyhow!("Missing value"))?)?,
    };
    ensure!(
        *value == build_private_contact_fact(owner, contact, field, &register)?,
        "Noncanonical private fact"
    );
    Ok(PrivateContactFactValue {
        contact: contact.clone(),
        field: field.to_string(),
        register,
    })
}

/// Encode a validated V2 compatibility view as unsigned facts. The caller validates
/// the register clock and pending queue with its V2 engine before saving.
pub fn store_private_contact_facts(state: &Value) -> Result<Value> {
    let owner = state["owner"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing owner"))?;
    ensure!(state["version"] == 2, "Invalid compatibility state");
    let convert = |contacts: &Value, pending: bool| -> Result<Vec<PrivateContactFact>> {
        let mut facts = Vec::new();
        for (contact, fields) in contacts
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Missing contacts"))?
        {
            let fields = if pending { &fields["fields"] } else { fields };
            for (field, register) in fields
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("Missing fields"))?
            {
                facts.push(build_private_contact_fact(
                    owner,
                    contact,
                    field,
                    &serde_json::from_value(register.clone())?,
                )?);
            }
        }
        facts.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(facts)
    };
    Ok(
        serde_json::json!({"version":4,"owner":owner,"writer":state["writer"],"clock":state["clock"],
        "facts":convert(&state["contacts"],false)?,"pending":convert(&state["pending"],true)?}),
    )
}

/// Restore the compatibility view without minting new timestamps or field clocks.
/// Older state is passed through for the caller's V1/V2 migration and validation.
pub fn restore_private_contact_facts(state: Value, owner: &str) -> Result<Value> {
    if state["version"] != 3 && state["version"] != 4 {
        return Ok(state);
    }
    ensure!(
        state["owner"] == owner && state.as_object().is_some_and(|v| v.len() == 6),
        "Invalid private fact state"
    );
    let convert = |values: &Value, pending: bool| -> Result<Value> {
        let mut contacts = serde_json::Map::<String, Value>::new();
        for value in values
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing facts"))?
        {
            let fact: PrivateContactFact = if value.get("draft").is_some() {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct LegacyFact {
                    v: u8,
                    owner: String,
                    id: String,
                    draft: FactEventDraft,
                }
                let legacy: LegacyFact = serde_json::from_value(value.clone())?;
                ensure!(
                    legacy.v == 1 && legacy.owner == owner,
                    "Invalid legacy private fact"
                );
                let bytes = serde_json::to_vec(&serde_json::json!([
                    "iris-private-contact-fact",
                    1,
                    owner,
                    legacy.draft.kind,
                    legacy.draft.tags,
                    legacy.draft.content
                ]))?;
                ensure!(
                    legacy.id == format!("{:x}", Sha256::digest(bytes)),
                    "Invalid legacy private fact ID"
                );
                let bytes = serde_json::to_vec(&serde_json::json!([
                    0,
                    owner,
                    0,
                    legacy.draft.kind,
                    legacy.draft.tags,
                    legacy.draft.content
                ]))?;
                PrivateContactFact {
                    id: format!("{:x}", Sha256::digest(bytes)),
                    pubkey: owner.into(),
                    created_at: 0,
                    kind: legacy.draft.kind,
                    tags: legacy.draft.tags,
                    content: legacy.draft.content,
                }
            } else {
                serde_json::from_value(value.clone())?
            };
            let parsed = parse_private_contact_fact(&fact, owner)?;
            let fields = contacts
                .entry(parsed.contact)
                .or_insert_with(|| serde_json::json!({}));
            let fields = fields.as_object_mut().unwrap();
            ensure!(
                !fields.contains_key(&parsed.field),
                "Duplicate private fact field"
            );
            fields.insert(parsed.field, serde_json::to_value(parsed.register)?);
        }
        if pending {
            for (contact, fields) in &mut contacts {
                *fields = serde_json::json!({"version":2,"owner":owner,"contact":contact,"fields":fields});
            }
        }
        Ok(Value::Object(contacts))
    };
    Ok(
        serde_json::json!({"version":2,"owner":owner,"writer":state["writer"],"clock":state["clock"],
        "contacts":convert(&state["facts"],false)?,"pending":convert(&state["pending"],true)?}),
    )
}
