use crate::{Fact, FactOp, FactOpLinks, build_fact_op_event_with_links, fact, parse_fact_op_event};
use anyhow::{Result, anyhow, bail};
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Event, EventBuilder, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const NOSTR_IDENTITY_ROSTER_SCHEMA: u64 = 1;
pub const NOSTR_IDENTITY_KEY_ACCEPTANCE_SCHEMA: u64 = 1;
pub const NOSTR_IDENTITY_ROSTER_TYPE: &str = "nostr_identity_roster_op";
pub const NOSTR_IDENTITY_KEY_ACCEPTANCE_TYPE: &str = "nostr_identity_key_acceptance";
pub const NOSTR_IDENTITY_LINK_REQUEST_TYPE: &str = "nostr_identity_link_request";

pub const IDENTITY_CAPABILITY_ADMIN: &str = "admin";
pub const IDENTITY_CAPABILITY_WRITE: &str = "write";
pub const IDENTITY_CAPABILITY_RECOVER: &str = "recover";
pub const IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS: &str = "receive_secret_wraps";
pub const IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS: &str = "decrypt_secret_epochs";

pub const IDENTITY_PURPOSE_APP: &str = "app";
pub const IDENTITY_PURPOSE_RECOVERY: &str = "recovery";
pub const IDENTITY_PURPOSE_REMOTE_SIGNER: &str = "remote_signer";
pub const IDENTITY_PURPOSE_PROFILE: &str = "profile";

pub const IDENTITY_ADMIN_CAPABILITIES: &[&str] = &[
    IDENTITY_CAPABILITY_ADMIN,
    IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
    IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS,
];

pub const IDENTITY_APP_KEY_CAPABILITIES: &[&str] = &[
    IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
    IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS,
    IDENTITY_CAPABILITY_WRITE,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityKey {
    pub pubkey: String,
    pub subject: Option<Uuid>,
    pub purposes: Vec<String>,
    pub capabilities: Vec<String>,
    pub added_at: u64,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityRosterOp {
    AddKey {
        key: IdentityKey,
    },
    TombstoneKey {
        pubkey: String,
        reason: Option<String>,
    },
    SetKeyCapabilities {
        pubkey: String,
        capabilities: Vec<String>,
    },
    RotateSecretEpoch {
        epoch: u64,
        wrapped_secrets: BTreeMap<String, String>,
    },
    RepairSecretWraps {
        epoch: u64,
        wrapped_secrets: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRosterOpContent {
    pub schema: u64,
    pub identity: Uuid,
    pub actor_pubkey: String,
    pub actor_seq: Option<u64>,
    pub parents: Vec<String>,
    pub client_nonce: String,
    pub created_at: u64,
    pub op: IdentityRosterOp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIdentityRosterOp {
    pub op_id: String,
    pub signer_pubkey: String,
    pub content: IdentityRosterOpContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityKeyAcceptanceContent {
    pub schema: u64,
    pub identity: Uuid,
    pub key_pubkey: String,
    pub purposes: Vec<String>,
    pub roster_op_id: Option<String>,
    pub client_nonce: String,
    pub accepted_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityLinkRequestContent {
    pub identity: Uuid,
    pub admin_pubkey: String,
    pub invite_pubkey: String,
    pub joining_pubkey: String,
    pub client_nonce: String,
    pub requested_at: u64,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIdentityKeyAcceptance {
    pub acceptance_id: String,
    pub signer_pubkey: String,
    pub content: IdentityKeyAcceptanceContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIdentityLinkRequest {
    pub request_id: String,
    pub signer_pubkey: String,
    pub content: IdentityLinkRequestContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityLinkRequestPublicHeader {
    pub identity: Uuid,
    pub invite_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentitySecretEpoch {
    pub epoch: u64,
    pub created_at: u64,
    pub signed_by_pubkey: String,
    pub wrapped_secrets: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityKeyTombstone {
    pub pubkey: String,
    pub subject: Option<Uuid>,
    pub removed_by_pubkey: String,
    pub removed_at: u64,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRosterProjection {
    pub identity: Uuid,
    pub active_keys: BTreeMap<String, IdentityKey>,
    pub tombstones: BTreeMap<String, IdentityKeyTombstone>,
    pub secret_epochs: BTreeMap<u64, IdentitySecretEpoch>,
    pub accepted_op_ids: Vec<String>,
    pub rejected_op_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityKeyAcceptanceProjection {
    pub identity: Uuid,
    pub accepted_keys: BTreeMap<String, IdentityKeyAcceptanceContent>,
    pub accepted_acceptance_ids: Vec<String>,
    pub rejected_acceptance_ids: Vec<String>,
}

pub fn identity_key(
    pubkey: impl Into<String>,
    added_at: u64,
    purposes: impl IntoIterator<Item = String>,
    capabilities: impl IntoIterator<Item = String>,
    label: Option<String>,
) -> Result<IdentityKey> {
    normalize_identity_key(IdentityKey {
        pubkey: pubkey.into(),
        subject: None,
        purposes: purposes.into_iter().collect(),
        capabilities: capabilities.into_iter().collect(),
        added_at,
        label,
    })
}

pub fn build_identity_roster_op_event(
    keys: &Keys,
    identity: Uuid,
    op: IdentityRosterOp,
    parents: impl IntoIterator<Item = String>,
    actor_seq: Option<u64>,
    client_nonce: impl Into<String>,
    created_at: u64,
) -> Result<Event> {
    let actor_pubkey = keys.public_key().to_hex();
    let parents = normalize_event_ids(parents, "parent")?;
    let content = IdentityRosterOpContent {
        schema: NOSTR_IDENTITY_ROSTER_SCHEMA,
        identity,
        actor_pubkey,
        actor_seq,
        parents: parents.clone(),
        client_nonce: require_non_empty(client_nonce.into(), "client_nonce")?,
        created_at,
        op: normalize_identity_roster_op(op)?,
    };
    build_fact_op_event_with_links(
        keys,
        identity,
        roster_op_content_facts(&content),
        FactOpLinks {
            prev: parents,
            ..FactOpLinks::default()
        },
        created_at,
    )
}

pub fn build_identity_key_acceptance_event(
    keys: &Keys,
    identity: Uuid,
    purposes: impl IntoIterator<Item = String>,
    roster_op_id: Option<String>,
    client_nonce: impl Into<String>,
    accepted_at: u64,
) -> Result<Event> {
    let content = IdentityKeyAcceptanceContent {
        schema: NOSTR_IDENTITY_KEY_ACCEPTANCE_SCHEMA,
        identity,
        key_pubkey: keys.public_key().to_hex(),
        purposes: normalize_tokens(purposes, "purpose")?,
        roster_op_id: roster_op_id
            .map(|id| require_event_id(&id, "roster_op_id"))
            .transpose()?,
        client_nonce: require_non_empty(client_nonce.into(), "client_nonce")?,
        accepted_at,
    };
    if content.purposes.is_empty() {
        bail!("identity key acceptance purposes must not be empty");
    }
    build_fact_op_event_with_links(
        keys,
        identity,
        key_acceptance_content_facts(&content),
        FactOpLinks::default(),
        accepted_at,
    )
}

pub fn build_identity_link_request_event(
    keys: &Keys,
    identity: Uuid,
    admin_pubkey: impl Into<String>,
    invite_pubkey: impl Into<String>,
    client_nonce: impl Into<String>,
    label: Option<String>,
    requested_at: u64,
) -> Result<Event> {
    let invite_pubkey = require_pubkey(&invite_pubkey.into(), "identity link request invite")?;
    let content = IdentityLinkRequestContent {
        identity,
        admin_pubkey: require_pubkey(&admin_pubkey.into(), "identity link request admin")?,
        invite_pubkey: invite_pubkey.clone(),
        joining_pubkey: keys.public_key().to_hex(),
        client_nonce: require_non_empty(client_nonce.into(), "client_nonce")?,
        requested_at,
        label: label
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
    };
    let invite_pk = PublicKey::from_hex(&invite_pubkey)?;
    let content_json = serde_json::to_string(&IdentityLinkRequestWireContent::from(&content))?;
    let encrypted = nip44::encrypt(
        keys.secret_key(),
        &invite_pk,
        content_json,
        Nip44Version::V2,
    )?;
    let identity_tag = identity.to_string();
    let event = EventBuilder::new(Kind::from(crate::FACT_OP_KIND), encrypted)
        .tag(Tag::parse(["i", identity_tag.as_str(), "subject"])?)
        .tag(Tag::parse(["type", NOSTR_IDENTITY_LINK_REQUEST_TYPE])?)
        .tag(Tag::parse(["p", invite_pubkey.as_str()])?)
        .custom_created_at(nostr_sdk::Timestamp::from(requested_at))
        .sign_with_keys(keys)?;
    Ok(event)
}

pub fn parse_identity_link_request_event(
    event: &Event,
    invite_keys: &Keys,
) -> Result<SignedIdentityLinkRequest> {
    let header = parse_identity_link_request_public_header(event)?;
    let invite_pubkey = invite_keys.public_key().to_hex();
    let content = encrypted_link_request_content_from_event(event, invite_keys)?;
    if content.identity != header.identity {
        bail!("identity link request subject mismatch");
    }
    if content.joining_pubkey != event.pubkey.to_hex() {
        bail!("identity link request signer mismatch");
    }
    if content.invite_pubkey != invite_pubkey || !header.invite_pubkeys.contains(&invite_pubkey) {
        bail!("identity link request invite pubkey mismatch");
    }
    if content.requested_at != event.created_at.as_secs() {
        bail!("identity link request requested_at mismatch");
    }
    Ok(SignedIdentityLinkRequest {
        request_id: event.id.to_hex(),
        signer_pubkey: event.pubkey.to_hex(),
        content,
    })
}

pub fn parse_identity_link_request_event_for_invite_pubkey(
    event: &Event,
    invite_keys: &Keys,
    expected_invite_pubkey: impl Into<String>,
) -> Result<SignedIdentityLinkRequest> {
    let expected_invite_pubkey =
        require_pubkey(&expected_invite_pubkey.into(), "identity link request invite")?;
    if invite_keys.public_key().to_hex() != expected_invite_pubkey {
        bail!("identity link request invite key mismatch");
    }
    parse_identity_link_request_event(event, invite_keys)
}

fn encrypted_link_request_content_from_event(
    event: &Event,
    invite_keys: &Keys,
) -> Result<IdentityLinkRequestContent> {
    let plaintext = nip44::decrypt(invite_keys.secret_key(), &event.pubkey, &event.content)?;
    let wire: IdentityLinkRequestWireContent = serde_json::from_str(&plaintext)?;
    link_request_content_from_wire(wire)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IdentityLinkRequestWireContent {
    identity: String,
    admin_pubkey: String,
    invite_pubkey: String,
    joining_pubkey: String,
    client_nonce: String,
    requested_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

impl From<&IdentityLinkRequestContent> for IdentityLinkRequestWireContent {
    fn from(content: &IdentityLinkRequestContent) -> Self {
        Self {
            identity: content.identity.to_string(),
            admin_pubkey: content.admin_pubkey.clone(),
            invite_pubkey: content.invite_pubkey.clone(),
            joining_pubkey: content.joining_pubkey.clone(),
            client_nonce: content.client_nonce.clone(),
            requested_at: content.requested_at,
            label: content.label.clone(),
        }
    }
}

fn link_request_content_from_wire(
    wire: IdentityLinkRequestWireContent,
) -> Result<IdentityLinkRequestContent> {
    Ok(IdentityLinkRequestContent {
        identity: parse_identity_id(&wire.identity)?,
        admin_pubkey: require_pubkey(&wire.admin_pubkey, "identity link request admin")?,
        invite_pubkey: require_pubkey(&wire.invite_pubkey, "identity link request invite")?,
        joining_pubkey: require_pubkey(&wire.joining_pubkey, "identity link request signer")?,
        client_nonce: require_non_empty(wire.client_nonce, "client_nonce")?,
        requested_at: wire.requested_at,
        label: wire
            .label
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
    })
}

pub fn parse_identity_link_request_public_header(
    event: &Event,
) -> Result<IdentityLinkRequestPublicHeader> {
    if event.kind != Kind::from(crate::FACT_OP_KIND) {
        bail!(
            "wrong fact op kind: expected {}, got {:?}",
            crate::FACT_OP_KIND,
            event.kind
        );
    }
    event
        .verify()
        .map_err(|error| anyhow!("fact event signature failed: {error}"))?;

    let mut identity = None;
    let mut has_link_request_type = false;
    let mut invite_pubkeys = Vec::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(kind) = parts.first().map(String::as_str) else {
            continue;
        };
        match kind {
            "i" if parts.get(2).is_some_and(|marker| marker == "subject") => {
                let Some(value) = parts.get(1) else {
                    bail!("identity link request i tag is missing value");
                };
                let parsed = parse_identity_id(value)?;
                if identity.replace(parsed).is_some() {
                    bail!("identity link request has multiple subject i tags");
                }
            }
            "type" if parts
                .get(1)
                .is_some_and(|value| value == NOSTR_IDENTITY_LINK_REQUEST_TYPE) =>
            {
                has_link_request_type = true;
            }
            "p" => {
                let Some(value) = parts.get(1) else {
                    bail!("identity link request p tag is missing pubkey");
                };
                let pubkey = require_pubkey(value, "identity link request invite")?;
                if !invite_pubkeys.contains(&pubkey) {
                    invite_pubkeys.push(pubkey);
                }
            }
            _ => {}
        }
    }
    if !has_link_request_type {
        bail!("identity link request type tag missing");
    }
    if invite_pubkeys.is_empty() {
        bail!("identity link request invite p tag missing");
    }
    Ok(IdentityLinkRequestPublicHeader {
        identity: identity.ok_or_else(|| anyhow!("identity link request subject missing"))?,
        invite_pubkeys,
    })
}

pub fn identity_link_request_invite_pubkeys(event: &Event) -> Result<Vec<String>> {
    Ok(parse_identity_link_request_public_header(event)?.invite_pubkeys)
}

pub fn parse_identity_roster_op_event(event: &Event) -> Result<SignedIdentityRosterOp> {
    let op = parse_fact_op_event(event)?;
    let content = roster_op_content_from_facts(&op)?;
    if content.actor_pubkey != event.pubkey.to_hex() {
        bail!("identity roster actor signer mismatch");
    }
    if content.created_at != event.created_at.as_secs() {
        bail!("identity roster created_at mismatch");
    }
    Ok(SignedIdentityRosterOp {
        op_id: event.id.to_hex(),
        signer_pubkey: event.pubkey.to_hex(),
        content,
    })
}

pub fn parse_identity_key_acceptance_event(event: &Event) -> Result<SignedIdentityKeyAcceptance> {
    let op = parse_fact_op_event(event)?;
    let content = key_acceptance_content_from_facts(&op)?;
    if content.key_pubkey != event.pubkey.to_hex() {
        bail!("identity key acceptance signer mismatch");
    }
    if content.accepted_at != event.created_at.as_secs() {
        bail!("identity key acceptance accepted_at mismatch");
    }
    Ok(SignedIdentityKeyAcceptance {
        acceptance_id: event.id.to_hex(),
        signer_pubkey: event.pubkey.to_hex(),
        content,
    })
}

pub fn identity_roster_parent_ids(ops: &[SignedIdentityRosterOp]) -> Vec<String> {
    ops.first()
        .map(|op| project_identity_roster(op.content.identity, ops.iter().cloned()).accepted_op_ids)
        .unwrap_or_default()
}

pub fn project_identity_roster(
    identity: Uuid,
    ops: impl IntoIterator<Item = SignedIdentityRosterOp>,
) -> IdentityRosterProjection {
    let mut projection = IdentityRosterProjection {
        identity,
        active_keys: BTreeMap::new(),
        tombstones: BTreeMap::new(),
        secret_epochs: BTreeMap::new(),
        accepted_op_ids: Vec::new(),
        rejected_op_ids: Vec::new(),
    };
    let mut sorted = ops
        .into_iter()
        .filter(|op| op.content.identity == identity)
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        left.content
            .created_at
            .cmp(&right.content.created_at)
            .then_with(|| left.op_id.cmp(&right.op_id))
    });

    for signed in sorted {
        let op_id = signed.op_id.clone();
        if apply_identity_roster_op(&mut projection, &signed) {
            projection.accepted_op_ids.push(op_id);
        } else {
            projection.rejected_op_ids.push(op_id);
        }
    }
    projection
}

pub fn apply_identity_roster_op(
    projection: &mut IdentityRosterProjection,
    signed: &SignedIdentityRosterOp,
) -> bool {
    let signer = &signed.signer_pubkey;
    let is_bootstrap = projection.accepted_op_ids.is_empty()
        && matches!(&signed.content.op, IdentityRosterOp::AddKey { key } if key.pubkey == *signer && key_has_capability(key, IDENTITY_CAPABILITY_ADMIN));
    let can_admin = is_bootstrap || identity_key_can_admin(projection, signer);
    let can_recover = identity_key_can_recover(projection, signer);
    let can_decrypt_secret_epochs = projection
        .active_keys
        .get(signer)
        .is_some_and(|key| key_has_capability(key, IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS));
    let can_recover_roster = can_recover
        && match &signed.content.op {
            IdentityRosterOp::AddKey { key } => key_has_purpose(key, IDENTITY_PURPOSE_APP),
            IdentityRosterOp::TombstoneKey { .. }
            | IdentityRosterOp::RotateSecretEpoch { .. }
            | IdentityRosterOp::RepairSecretWraps { .. } => can_decrypt_secret_epochs,
            IdentityRosterOp::SetKeyCapabilities { .. } => false,
        };
    let can_repair_epoch = match &signed.content.op {
        IdentityRosterOp::RepairSecretWraps { epoch, .. } => projection
            .secret_epochs
            .get(epoch)
            .is_some_and(|secret_epoch| secret_epoch.signed_by_pubkey == *signer),
        _ => false,
    };
    if !can_admin && !can_recover_roster && !can_repair_epoch {
        return false;
    }

    match &signed.content.op {
        IdentityRosterOp::AddKey { key } => {
            projection.tombstones.remove(&key.pubkey);
            projection
                .active_keys
                .entry(key.pubkey.clone())
                .or_insert_with(|| key.clone());
            true
        }
        IdentityRosterOp::TombstoneKey { pubkey, reason } => {
            let subject = projection
                .active_keys
                .remove(pubkey)
                .and_then(|key| key.subject);
            projection.tombstones.insert(
                pubkey.clone(),
                IdentityKeyTombstone {
                    pubkey: pubkey.clone(),
                    subject,
                    removed_by_pubkey: signer.clone(),
                    removed_at: signed.content.created_at,
                    reason: reason.clone(),
                },
            );
            true
        }
        IdentityRosterOp::SetKeyCapabilities {
            pubkey,
            capabilities,
        } => {
            let Some(key) = projection.active_keys.get_mut(pubkey) else {
                return false;
            };
            if projection.tombstones.contains_key(pubkey) {
                return false;
            }
            key.capabilities = capabilities.clone();
            true
        }
        IdentityRosterOp::RotateSecretEpoch {
            epoch,
            wrapped_secrets,
        } => {
            projection.secret_epochs.insert(
                *epoch,
                IdentitySecretEpoch {
                    epoch: *epoch,
                    created_at: signed.content.created_at,
                    signed_by_pubkey: signer.clone(),
                    wrapped_secrets: wrapped_secrets.clone(),
                },
            );
            true
        }
        IdentityRosterOp::RepairSecretWraps {
            epoch,
            wrapped_secrets,
        } => {
            let Some(secret_epoch) = projection.secret_epochs.get_mut(epoch) else {
                return false;
            };
            if secret_epoch.signed_by_pubkey != *signer {
                return false;
            }
            secret_epoch.wrapped_secrets.extend(
                wrapped_secrets
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
            true
        }
    }
}

pub fn project_identity_key_acceptances(
    identity: Uuid,
    acceptances: impl IntoIterator<Item = SignedIdentityKeyAcceptance>,
) -> IdentityKeyAcceptanceProjection {
    let mut projection = IdentityKeyAcceptanceProjection {
        identity,
        accepted_keys: BTreeMap::new(),
        accepted_acceptance_ids: Vec::new(),
        rejected_acceptance_ids: Vec::new(),
    };
    let mut sorted = acceptances
        .into_iter()
        .filter(|acceptance| acceptance.content.identity == identity)
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        left.content
            .accepted_at
            .cmp(&right.content.accepted_at)
            .then_with(|| left.acceptance_id.cmp(&right.acceptance_id))
    });
    for signed in sorted {
        if signed.signer_pubkey != signed.content.key_pubkey || signed.content.purposes.is_empty() {
            projection
                .rejected_acceptance_ids
                .push(signed.acceptance_id);
            continue;
        }
        projection
            .accepted_keys
            .insert(signed.content.key_pubkey.clone(), signed.content.clone());
        projection
            .accepted_acceptance_ids
            .push(signed.acceptance_id);
    }
    projection
}

pub fn identity_key_can_admin(projection: &IdentityRosterProjection, pubkey: &str) -> bool {
    normalize_hex_pubkey(pubkey)
        .and_then(|normalized| projection.active_keys.get(&normalized))
        .is_some_and(|key| key_has_capability(key, IDENTITY_CAPABILITY_ADMIN))
}

pub fn identity_key_can_recover(projection: &IdentityRosterProjection, pubkey: &str) -> bool {
    normalize_hex_pubkey(pubkey)
        .and_then(|normalized| projection.active_keys.get(&normalized))
        .is_some_and(|key| key_has_capability(key, IDENTITY_CAPABILITY_RECOVER))
}

pub fn normalize_identity_capabilities(
    capabilities: impl IntoIterator<Item = String>,
) -> Result<Vec<String>> {
    normalize_tokens(capabilities, "capability")
}

pub fn normalize_identity_purposes(
    purposes: impl IntoIterator<Item = String>,
) -> Result<Vec<String>> {
    normalize_tokens(purposes, "purpose")
}

pub fn is_nostr_identity_id(value: &str) -> bool {
    parse_identity_id(value).is_ok()
}

fn roster_op_content_facts(content: &IdentityRosterOpContent) -> Vec<Fact> {
    let mut facts = vec![
        fact("type", &[NOSTR_IDENTITY_ROSTER_TYPE]),
        fact("schema", &[&content.schema.to_string()]),
        fact("actor_pubkey", &[&content.actor_pubkey]),
        fact("client_nonce", &[&content.client_nonce]),
        fact("created_at", &[&content.created_at.to_string()]),
        fact("op", &[roster_op_name(&content.op)]),
    ];
    if let Some(actor_seq) = content.actor_seq {
        facts.push(fact("actor_seq", &[&actor_seq.to_string()]));
    }
    facts.extend(roster_op_facts(&content.op));
    facts
}

fn roster_op_facts(op: &IdentityRosterOp) -> Vec<Fact> {
    match op {
        IdentityRosterOp::AddKey { key } => {
            let mut facts = vec![
                fact("key_pubkey", &[&key.pubkey]),
                fact("key_added_at", &[&key.added_at.to_string()]),
            ];
            if let Some(subject) = key.subject {
                facts.push(fact("key_subject", &[&subject.to_string()]));
            }
            facts.extend(
                key.purposes
                    .iter()
                    .map(|purpose| fact("key_purpose", &[purpose])),
            );
            facts.extend(
                key.capabilities
                    .iter()
                    .map(|capability| fact("key_capability", &[capability])),
            );
            if let Some(label) = &key.label {
                facts.push(fact("key_label", &[label]));
            }
            facts
        }
        IdentityRosterOp::TombstoneKey { pubkey, reason } => {
            let mut facts = vec![fact("target_pubkey", &[pubkey])];
            if let Some(reason) = reason {
                facts.push(fact("reason", &[reason]));
            }
            facts
        }
        IdentityRosterOp::SetKeyCapabilities {
            pubkey,
            capabilities,
        } => {
            let mut facts = vec![fact("target_pubkey", &[pubkey])];
            facts.extend(
                capabilities
                    .iter()
                    .map(|capability| fact("capability", &[capability])),
            );
            facts
        }
        IdentityRosterOp::RotateSecretEpoch {
            epoch,
            wrapped_secrets,
        }
        | IdentityRosterOp::RepairSecretWraps {
            epoch,
            wrapped_secrets,
        } => {
            let mut facts = vec![fact("secret_epoch", &[&epoch.to_string()])];
            facts.extend(wrapped_secrets.iter().map(|(pubkey, wrapped)| {
                Fact::new("wrapped_secret", [pubkey.clone(), wrapped.clone()])
            }));
            facts
        }
    }
}

fn key_acceptance_content_facts(content: &IdentityKeyAcceptanceContent) -> Vec<Fact> {
    let mut facts = vec![
        fact("type", &[NOSTR_IDENTITY_KEY_ACCEPTANCE_TYPE]),
        fact("schema", &[&content.schema.to_string()]),
        fact("key_pubkey", &[&content.key_pubkey]),
        fact("client_nonce", &[&content.client_nonce]),
        fact("accepted_at", &[&content.accepted_at.to_string()]),
    ];
    facts.extend(
        content
            .purposes
            .iter()
            .map(|purpose| fact("purpose", &[purpose])),
    );
    if let Some(roster_op_id) = &content.roster_op_id {
        facts.push(fact("roster_op_id", &[roster_op_id]));
    }
    facts
}

fn roster_op_content_from_facts(op: &FactOp) -> Result<IdentityRosterOpContent> {
    require_type(op, NOSTR_IDENTITY_ROSTER_TYPE)?;
    let schema = required_integer(op, "schema")?;
    if schema != NOSTR_IDENTITY_ROSTER_SCHEMA {
        bail!("unsupported Nostr identity roster schema {schema}");
    }
    Ok(IdentityRosterOpContent {
        schema,
        identity: op.subject,
        actor_pubkey: required_pubkey(op, "actor_pubkey")?,
        actor_seq: optional_integer(op, "actor_seq")?,
        parents: op.prev.clone(),
        client_nonce: required_non_empty_scalar(op, "client_nonce")?,
        created_at: required_integer(op, "created_at")?,
        op: roster_op_from_facts(op)?,
    })
}

fn roster_op_from_facts(op: &FactOp) -> Result<IdentityRosterOp> {
    match required_scalar(op, "op")?.as_str() {
        "add_key" => Ok(IdentityRosterOp::AddKey {
            key: normalize_identity_key(IdentityKey {
                pubkey: required_pubkey(op, "key_pubkey")?,
                subject: optional_scalar(op, "key_subject")?
                    .map(|subject| parse_identity_id(&subject))
                    .transpose()?,
                purposes: scalar_values(op, "key_purpose")?,
                capabilities: scalar_values(op, "key_capability")?,
                added_at: required_integer(op, "key_added_at")?,
                label: optional_scalar(op, "key_label")?,
            })?,
        }),
        "tombstone_key" => Ok(IdentityRosterOp::TombstoneKey {
            pubkey: required_pubkey(op, "target_pubkey")?,
            reason: optional_scalar(op, "reason")?,
        }),
        "set_key_capabilities" => Ok(IdentityRosterOp::SetKeyCapabilities {
            pubkey: required_pubkey(op, "target_pubkey")?,
            capabilities: normalize_tokens(scalar_values(op, "capability")?, "capability")?,
        }),
        "rotate_secret_epoch" => Ok(IdentityRosterOp::RotateSecretEpoch {
            epoch: required_integer(op, "secret_epoch")?,
            wrapped_secrets: wrapped_secrets_from_facts(op)?,
        }),
        "repair_secret_wraps" => Ok(IdentityRosterOp::RepairSecretWraps {
            epoch: required_integer(op, "secret_epoch")?,
            wrapped_secrets: wrapped_secrets_from_facts(op)?,
        }),
        kind => bail!("unsupported Nostr identity roster op {kind}"),
    }
}

fn key_acceptance_content_from_facts(op: &FactOp) -> Result<IdentityKeyAcceptanceContent> {
    require_type(op, NOSTR_IDENTITY_KEY_ACCEPTANCE_TYPE)?;
    let schema = required_integer(op, "schema")?;
    if schema != NOSTR_IDENTITY_KEY_ACCEPTANCE_SCHEMA {
        bail!("unsupported Nostr identity key acceptance schema {schema}");
    }
    let content = IdentityKeyAcceptanceContent {
        schema,
        identity: op.subject,
        key_pubkey: required_pubkey(op, "key_pubkey")?,
        purposes: normalize_tokens(scalar_values(op, "purpose")?, "purpose")?,
        roster_op_id: optional_scalar(op, "roster_op_id")?
            .map(|id| require_event_id(&id, "roster_op_id"))
            .transpose()?,
        client_nonce: required_non_empty_scalar(op, "client_nonce")?,
        accepted_at: required_integer(op, "accepted_at")?,
    };
    if content.purposes.is_empty() {
        bail!("identity key acceptance purposes must not be empty");
    }
    Ok(content)
}

fn normalize_identity_roster_op(op: IdentityRosterOp) -> Result<IdentityRosterOp> {
    match op {
        IdentityRosterOp::AddKey { key } => Ok(IdentityRosterOp::AddKey {
            key: normalize_identity_key(key)?,
        }),
        IdentityRosterOp::TombstoneKey { pubkey, reason } => Ok(IdentityRosterOp::TombstoneKey {
            pubkey: require_pubkey(&pubkey, "target")?,
            reason: reason.map(|value| value.trim().to_owned()),
        }),
        IdentityRosterOp::SetKeyCapabilities {
            pubkey,
            capabilities,
        } => Ok(IdentityRosterOp::SetKeyCapabilities {
            pubkey: require_pubkey(&pubkey, "target")?,
            capabilities: normalize_tokens(capabilities, "capability")?,
        }),
        IdentityRosterOp::RotateSecretEpoch {
            epoch,
            wrapped_secrets,
        } => Ok(IdentityRosterOp::RotateSecretEpoch {
            epoch,
            wrapped_secrets: normalize_wrapped_secrets(wrapped_secrets)?,
        }),
        IdentityRosterOp::RepairSecretWraps {
            epoch,
            wrapped_secrets,
        } => Ok(IdentityRosterOp::RepairSecretWraps {
            epoch,
            wrapped_secrets: normalize_wrapped_secrets(wrapped_secrets)?,
        }),
    }
}

fn normalize_identity_key(key: IdentityKey) -> Result<IdentityKey> {
    Ok(IdentityKey {
        pubkey: require_pubkey(&key.pubkey, "identity key")?,
        subject: key.subject,
        purposes: normalize_tokens(key.purposes, "purpose")?,
        capabilities: normalize_tokens(key.capabilities, "capability")?,
        added_at: key.added_at,
        label: key
            .label
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
    })
}

fn normalize_wrapped_secrets(
    wrapped_secrets: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    wrapped_secrets
        .into_iter()
        .map(|(pubkey, wrapped)| {
            Ok((
                require_pubkey(&pubkey, "wrapped secret recipient")?,
                require_non_empty(wrapped, "wrapped secret")?,
            ))
        })
        .collect()
}

fn wrapped_secrets_from_facts(op: &FactOp) -> Result<BTreeMap<String, String>> {
    value_tuples(op, "wrapped_secret")
        .into_iter()
        .map(|values| {
            if values.len() != 2 {
                bail!("wrapped_secret fact must have pubkey and wrapped value");
            }
            Ok((
                require_pubkey(&values[0], "wrapped secret recipient")?,
                require_non_empty(values[1].clone(), "wrapped secret")?,
            ))
        })
        .collect()
}

fn roster_op_name(op: &IdentityRosterOp) -> &'static str {
    match op {
        IdentityRosterOp::AddKey { .. } => "add_key",
        IdentityRosterOp::TombstoneKey { .. } => "tombstone_key",
        IdentityRosterOp::SetKeyCapabilities { .. } => "set_key_capabilities",
        IdentityRosterOp::RotateSecretEpoch { .. } => "rotate_secret_epoch",
        IdentityRosterOp::RepairSecretWraps { .. } => "repair_secret_wraps",
    }
}

fn key_has_capability(key: &IdentityKey, capability: &str) -> bool {
    key.capabilities.iter().any(|value| value == capability)
}

fn key_has_purpose(key: &IdentityKey, purpose: &str) -> bool {
    key.purposes.iter().any(|value| value == purpose)
}

fn require_type(op: &FactOp, expected: &str) -> Result<()> {
    let value = required_scalar(op, "type")?;
    if value != expected {
        bail!("unexpected Nostr identity fact event type {value}");
    }
    Ok(())
}

fn required_pubkey(op: &FactOp, predicate: &str) -> Result<String> {
    require_pubkey(&required_scalar(op, predicate)?, predicate)
}

fn required_scalar(op: &FactOp, predicate: &str) -> Result<String> {
    optional_scalar(op, predicate)?
        .ok_or_else(|| anyhow!("missing Nostr identity fact {predicate}"))
}

fn required_non_empty_scalar(op: &FactOp, predicate: &str) -> Result<String> {
    require_non_empty(required_scalar(op, predicate)?, predicate)
}

fn optional_scalar(op: &FactOp, predicate: &str) -> Result<Option<String>> {
    let values = value_tuples(op, predicate);
    if values.is_empty() {
        return Ok(None);
    }
    if values.len() != 1 || values[0].len() != 1 {
        bail!("Nostr identity fact {predicate} must be a single scalar");
    }
    Ok(Some(values[0][0].clone()))
}

fn scalar_values(op: &FactOp, predicate: &str) -> Result<Vec<String>> {
    value_tuples(op, predicate)
        .into_iter()
        .map(|values| {
            if values.len() != 1 {
                bail!("Nostr identity fact {predicate} must be scalar");
            }
            Ok(values[0].clone())
        })
        .collect()
}

fn value_tuples(op: &FactOp, predicate: &str) -> Vec<Vec<String>> {
    op.facts
        .iter()
        .filter(|fact| fact.predicate == predicate)
        .map(|fact| fact.values.clone())
        .collect()
}

fn required_integer(op: &FactOp, predicate: &str) -> Result<u64> {
    require_integer(&required_scalar(op, predicate)?, predicate)
}

fn optional_integer(op: &FactOp, predicate: &str) -> Result<Option<u64>> {
    optional_scalar(op, predicate)?
        .map(|value| require_integer(&value, predicate))
        .transpose()
}

fn require_integer(value: &str, label: &str) -> Result<u64> {
    let parsed = value.parse::<u64>().map_err(|error| {
        anyhow!("Nostr identity {label} must be a non-negative integer: {error}")
    })?;
    if parsed.to_string() != value {
        bail!("Nostr identity {label} must be a non-negative integer");
    }
    Ok(parsed)
}

fn parse_identity_id(value: &str) -> Result<Uuid> {
    let trimmed = value.trim();
    let uuid = Uuid::parse_str(trimmed)
        .map_err(|error| anyhow!("invalid Nostr identity id {value}: {error}"))?;
    if uuid.to_string() != trimmed.to_lowercase() || trimmed != trimmed.to_lowercase() {
        bail!("Nostr identity id must be a canonical UUID: {value}");
    }
    Ok(uuid)
}

fn require_pubkey(value: &str, label: &str) -> Result<String> {
    normalize_hex_pubkey(value).ok_or_else(|| anyhow!("{label} pubkey must be 64-char hex"))
}

fn require_event_id(value: &str, label: &str) -> Result<String> {
    normalize_hex_pubkey(value).ok_or_else(|| anyhow!("{label} must be 64-char hex"))
}

fn require_non_empty(value: String, label: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("Nostr identity {label} must not be empty");
    }
    Ok(trimmed.to_owned())
}

fn normalize_tokens(values: impl IntoIterator<Item = String>, label: &str) -> Result<Vec<String>> {
    let mut tokens = values
        .into_iter()
        .map(|value| normalize_token(value, label))
        .collect::<Result<Vec<_>>>()?;
    tokens.sort();
    tokens.dedup();
    Ok(tokens)
}

fn normalize_token(value: String, label: &str) -> Result<String> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        bail!("Nostr identity {label} must not be empty");
    }
    if normalized.chars().any(char::is_whitespace) {
        bail!("Nostr identity {label} must not contain whitespace: {value}");
    }
    Ok(normalized)
}

fn normalize_event_ids(
    values: impl IntoIterator<Item = String>,
    label: &str,
) -> Result<Vec<String>> {
    let mut ids = values
        .into_iter()
        .map(|value| require_event_id(&value, label))
        .collect::<Result<Vec<_>>>()?;
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn normalize_hex_pubkey(value: &str) -> Option<String> {
    let trimmed = value.trim().to_lowercase();
    is_lower_hex(&trimmed, 64).then_some(trimmed)
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{JsonUtil, SecretKey};

    const SUBJECT: &str = "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a";

    fn subject() -> Uuid {
        Uuid::parse_str(SUBJECT).unwrap()
    }

    fn capabilities(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn fixed_keys(byte: u8) -> Keys {
        Keys::new(SecretKey::from_slice(&[byte; 32]).unwrap())
    }

    #[test]
    fn builds_and_parses_neutral_roster_fact_events() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let event = build_identity_roster_op_event(
            &keys,
            subject(),
            IdentityRosterOp::AddKey {
                key: identity_key(
                    pubkey.clone(),
                    10,
                    [IDENTITY_PURPOSE_APP.to_owned()],
                    capabilities(IDENTITY_ADMIN_CAPABILITIES),
                    Some("Admin key".to_owned()),
                )
                .unwrap(),
            },
            Vec::<String>::new(),
            None,
            "nonce-1",
            10,
        )
        .unwrap();

        let parsed = parse_identity_roster_op_event(&event).unwrap();
        assert_eq!(parsed.content.identity, subject());
        assert_eq!(parsed.content.actor_pubkey, pubkey);
        assert_eq!(
            parsed.content.op,
            IdentityRosterOp::AddKey {
                key: IdentityKey {
                    pubkey,
                    subject: None,
                    purposes: vec![IDENTITY_PURPOSE_APP.to_owned()],
                    capabilities: capabilities(IDENTITY_ADMIN_CAPABILITIES),
                    added_at: 10,
                    label: Some("Admin key".to_owned()),
                }
            }
        );
    }

    #[test]
    fn builds_and_parses_encrypted_identity_link_request_events() {
        let admin_keys = Keys::generate();
        let device_keys = fixed_keys(1);
        let invite_keys = fixed_keys(2);
        let admin_pubkey = admin_keys.public_key().to_hex();
        let device_pubkey = device_keys.public_key().to_hex();
        let invite_pubkey = invite_keys.public_key().to_hex();
        let event = build_identity_link_request_event(
            &device_keys,
            subject(),
            admin_pubkey.clone(),
            invite_pubkey.clone(),
            "nonce-link",
            Some("Phone".to_owned()),
            21,
        )
        .unwrap();

        assert_eq!(event.kind.as_u16(), crate::FACT_OP_KIND);
        assert!(!event.content.is_empty());
        let event_json = event.as_json();
        assert!(event_json.contains(NOSTR_IDENTITY_LINK_REQUEST_TYPE));
        assert!(event_json.contains(&invite_pubkey));
        assert!(!event_json.contains(&admin_pubkey));
        assert!(!event_json.contains("hash-from-invite"));
        assert!(!event.content.contains(&admin_pubkey));
        assert!(!event.content.contains(&device_pubkey));

        let parsed = parse_identity_link_request_event(&event, &invite_keys).unwrap();
        assert_eq!(parsed.signer_pubkey, device_pubkey);
        assert_eq!(parsed.content.identity, subject());
        assert_eq!(parsed.content.admin_pubkey, admin_pubkey);
        assert_eq!(parsed.content.invite_pubkey, invite_pubkey);
        assert_eq!(parsed.content.joining_pubkey, device_pubkey);
        assert_eq!(parsed.content.client_nonce, "nonce-link");
        assert_eq!(parsed.content.requested_at, 21);
        assert_eq!(parsed.content.label, Some("Phone".to_owned()));
        assert!(parse_identity_link_request_event(&event, &fixed_keys(3)).is_err());
    }

    #[test]
    fn parses_shared_ts_rust_identity_link_request_fixture() {
        let device_keys = fixed_keys(1);
        let invite_keys = fixed_keys(2);
        let device_pubkey = device_keys.public_key().to_hex();
        let invite_pubkey = invite_keys.public_key().to_hex();
        let event = Event::from_json(include_str!(
            "../../../../testdata/identity-link-request.json"
        ))
        .unwrap();
        let parsed = parse_identity_link_request_event(&event, &invite_keys).unwrap();

        assert_eq!(parsed.signer_pubkey, device_pubkey);
        assert_eq!(parsed.content.identity, subject());
        assert_eq!(
            parsed.content.admin_pubkey,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(parsed.content.invite_pubkey, invite_pubkey);
        assert_eq!(parsed.content.joining_pubkey, device_pubkey);
        assert_eq!(parsed.content.client_nonce, "fixture-link-request");
        assert_eq!(parsed.content.requested_at, 1_720_000_021);
        assert_eq!(parsed.content.label, Some("Fixture Phone".to_owned()));
    }

    #[test]
    fn projects_admin_authorized_keys_and_rejects_non_admin_edits() {
        let admin_keys = Keys::generate();
        let app_keys = Keys::generate();
        let admin_pubkey = admin_keys.public_key().to_hex();
        let app_pubkey = app_keys.public_key().to_hex();

        let bootstrap = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &admin_keys,
                subject(),
                IdentityRosterOp::AddKey {
                    key: identity_key(
                        admin_pubkey.clone(),
                        10,
                        [IDENTITY_PURPOSE_APP.to_owned()],
                        capabilities(IDENTITY_ADMIN_CAPABILITIES),
                        None,
                    )
                    .unwrap(),
                },
                Vec::<String>::new(),
                None,
                "nonce-1",
                10,
            )
            .unwrap(),
        )
        .unwrap();
        let add_app = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &admin_keys,
                subject(),
                IdentityRosterOp::AddKey {
                    key: identity_key(
                        app_pubkey.clone(),
                        11,
                        [IDENTITY_PURPOSE_APP.to_owned()],
                        capabilities(IDENTITY_APP_KEY_CAPABILITIES),
                        Some("Phone".to_owned()),
                    )
                    .unwrap(),
                },
                [bootstrap.op_id.clone()],
                None,
                "nonce-2",
                11,
            )
            .unwrap(),
        )
        .unwrap();
        let rejected = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &app_keys,
                subject(),
                IdentityRosterOp::SetKeyCapabilities {
                    pubkey: app_pubkey.clone(),
                    capabilities: vec![IDENTITY_CAPABILITY_ADMIN.to_owned()],
                },
                [add_app.op_id.clone()],
                None,
                "nonce-3",
                12,
            )
            .unwrap(),
        )
        .unwrap();

        let projection = project_identity_roster(subject(), [rejected.clone(), add_app, bootstrap]);
        assert_eq!(projection.accepted_op_ids.len(), 2);
        assert_eq!(projection.rejected_op_ids, vec![rejected.op_id]);
        assert!(identity_key_can_admin(&projection, &admin_pubkey));
        assert_eq!(
            projection.active_keys[&app_pubkey].capabilities,
            capabilities(IDENTITY_APP_KEY_CAPABILITIES)
        );
    }

    #[test]
    fn projects_secret_epochs_and_same_signer_repairs() {
        let keys = Keys::generate();
        let app_keys = Keys::generate();
        let admin_pubkey = keys.public_key().to_hex();
        let app_pubkey = app_keys.public_key().to_hex();
        let bootstrap = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &keys,
                subject(),
                IdentityRosterOp::AddKey {
                    key: identity_key(
                        admin_pubkey.clone(),
                        10,
                        [IDENTITY_PURPOSE_APP.to_owned()],
                        capabilities(IDENTITY_ADMIN_CAPABILITIES),
                        None,
                    )
                    .unwrap(),
                },
                Vec::<String>::new(),
                None,
                "nonce-1",
                10,
            )
            .unwrap(),
        )
        .unwrap();
        let rotate = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &keys,
                subject(),
                IdentityRosterOp::RotateSecretEpoch {
                    epoch: 1,
                    wrapped_secrets: BTreeMap::from([(
                        admin_pubkey.clone(),
                        "wrap-admin".to_owned(),
                    )]),
                },
                [bootstrap.op_id.clone()],
                None,
                "nonce-2",
                11,
            )
            .unwrap(),
        )
        .unwrap();
        let repair = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &keys,
                subject(),
                IdentityRosterOp::RepairSecretWraps {
                    epoch: 1,
                    wrapped_secrets: BTreeMap::from([(app_pubkey.clone(), "wrap-app".to_owned())]),
                },
                [rotate.op_id.clone()],
                None,
                "nonce-3",
                12,
            )
            .unwrap(),
        )
        .unwrap();

        let projection = project_identity_roster(subject(), [bootstrap, rotate, repair]);
        assert_eq!(
            projection.secret_epochs[&1].wrapped_secrets,
            BTreeMap::from([
                (admin_pubkey, "wrap-admin".to_owned()),
                (app_pubkey, "wrap-app".to_owned()),
            ])
        );
    }

    #[test]
    fn allows_recovery_keys_to_add_and_remove_app_keys_and_rewrap_secrets() {
        let admin_keys = Keys::generate();
        let recovery_keys = Keys::generate();
        let app_keys = Keys::generate();
        let admin_pubkey = admin_keys.public_key().to_hex();
        let recovery_pubkey = recovery_keys.public_key().to_hex();
        let app_pubkey = app_keys.public_key().to_hex();
        let bootstrap = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &admin_keys,
                subject(),
                IdentityRosterOp::AddKey {
                    key: identity_key(
                        admin_pubkey.clone(),
                        10,
                        [IDENTITY_PURPOSE_APP.to_owned()],
                        capabilities(IDENTITY_ADMIN_CAPABILITIES),
                        None,
                    )
                    .unwrap(),
                },
                Vec::<String>::new(),
                None,
                "nonce-1",
                10,
            )
            .unwrap(),
        )
        .unwrap();
        let add_recovery = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &admin_keys,
                subject(),
                IdentityRosterOp::AddKey {
                    key: identity_key(
                        recovery_pubkey.clone(),
                        11,
                        [IDENTITY_PURPOSE_RECOVERY.to_owned()],
                        [
                            IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS.to_owned(),
                            IDENTITY_CAPABILITY_RECOVER.to_owned(),
                        ],
                        Some("Recovery phrase".to_owned()),
                    )
                    .unwrap(),
                },
                [bootstrap.op_id.clone()],
                None,
                "nonce-2",
                11,
            )
            .unwrap(),
        )
        .unwrap();
        let recover_app = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &recovery_keys,
                subject(),
                IdentityRosterOp::AddKey {
                    key: identity_key(
                        app_pubkey.clone(),
                        12,
                        [IDENTITY_PURPOSE_APP.to_owned()],
                        capabilities(IDENTITY_APP_KEY_CAPABILITIES),
                        Some("Recovered app".to_owned()),
                    )
                    .unwrap(),
                },
                [add_recovery.op_id.clone()],
                None,
                "nonce-3",
                12,
            )
            .unwrap(),
        )
        .unwrap();
        let recover_admin = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &recovery_keys,
                subject(),
                IdentityRosterOp::AddKey {
                    key: identity_key(
                        Keys::generate().public_key().to_hex(),
                        13,
                        [IDENTITY_PURPOSE_APP.to_owned()],
                        capabilities(IDENTITY_ADMIN_CAPABILITIES),
                        Some("Recovered admin".to_owned()),
                    )
                    .unwrap(),
                },
                [recover_app.op_id.clone()],
                None,
                "nonce-4",
                13,
            )
            .unwrap(),
        )
        .unwrap();
        let rotate = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &recovery_keys,
                subject(),
                IdentityRosterOp::RotateSecretEpoch {
                    epoch: 2,
                    wrapped_secrets: BTreeMap::from([(app_pubkey.clone(), "wrap-app".to_owned())]),
                },
                [recover_app.op_id.clone()],
                None,
                "nonce-5",
                14,
            )
            .unwrap(),
        )
        .unwrap();
        let repair = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &recovery_keys,
                subject(),
                IdentityRosterOp::RepairSecretWraps {
                    epoch: 2,
                    wrapped_secrets: BTreeMap::from([(
                        admin_pubkey.clone(),
                        "wrap-admin".to_owned(),
                    )]),
                },
                [rotate.op_id.clone()],
                None,
                "nonce-6",
                15,
            )
            .unwrap(),
        )
        .unwrap();
        let remove_app_key = parse_identity_roster_op_event(
            &build_identity_roster_op_event(
                &recovery_keys,
                subject(),
                IdentityRosterOp::TombstoneKey {
                    pubkey: app_pubkey.clone(),
                    reason: Some("recovered".to_owned()),
                },
                [repair.op_id.clone()],
                None,
                "nonce-7",
                16,
            )
            .unwrap(),
        )
        .unwrap();

        let projection = project_identity_roster(
            subject(),
            [
                bootstrap,
                add_recovery,
                recover_app.clone(),
                recover_admin.clone(),
                rotate.clone(),
                repair.clone(),
                remove_app_key.clone(),
            ],
        );
        assert_eq!(projection.accepted_op_ids.len(), 7);
        assert!(projection.rejected_op_ids.is_empty());
        assert!(!projection.active_keys.contains_key(&app_pubkey));
        assert!(
            projection
                .active_keys
                .values()
                .any(|key| key_has_capability(key, IDENTITY_CAPABILITY_ADMIN))
        );
        assert_eq!(
            projection.tombstones[&app_pubkey].reason,
            Some("recovered".to_owned())
        );
        assert_eq!(
            projection.secret_epochs[&2].wrapped_secrets,
            BTreeMap::from([
                (admin_pubkey, "wrap-admin".to_owned()),
                (app_pubkey, "wrap-app".to_owned()),
            ])
        );
        assert!(projection.accepted_op_ids.contains(&recover_app.op_id));
        assert!(projection.accepted_op_ids.contains(&rotate.op_id));
        assert!(projection.accepted_op_ids.contains(&repair.op_id));
        assert!(projection.accepted_op_ids.contains(&remove_app_key.op_id));
    }

    #[test]
    fn builds_key_self_acceptance_events() {
        let keys = Keys::generate();
        let event = build_identity_key_acceptance_event(
            &keys,
            subject(),
            [
                IDENTITY_PURPOSE_REMOTE_SIGNER.to_owned(),
                IDENTITY_PURPOSE_APP.to_owned(),
            ],
            Some("2".repeat(64)),
            "nonce-4",
            20,
        )
        .unwrap();
        let signed = parse_identity_key_acceptance_event(&event).unwrap();
        let projection = project_identity_key_acceptances(subject(), [signed.clone()]);

        assert_eq!(
            signed.content.purposes,
            vec![
                IDENTITY_PURPOSE_APP.to_owned(),
                IDENTITY_PURPOSE_REMOTE_SIGNER.to_owned()
            ]
        );
        assert_eq!(
            projection.accepted_acceptance_ids,
            vec![signed.acceptance_id]
        );
        assert!(projection.accepted_keys.contains_key(&signed.signer_pubkey));
    }

    #[test]
    fn rejects_invalid_identity_facts() {
        let keys = Keys::generate();
        let error = build_identity_key_acceptance_event(
            &keys,
            subject(),
            Vec::<String>::new(),
            None,
            "nonce-4",
            20,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("purposes must not be empty"));
        assert!(!is_nostr_identity_id("not-a-uuid"));
    }
}
