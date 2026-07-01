//! App-agnostic Nostr identity authority.
//!
//! An `NostrIdentity` is identified by a UUID and owns a signed, append-only
//! roster of key facets. App installs use `AppKeys` for normal CRDT/root
//! authorship; recovery phrases and NIP-46 signers may help admit or recover
//! `AppKeys` without becoming root writers themselves.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use crate::{
    BuildIdentityRosterOpEventOptions, FACT_OP_KIND, IDENTITY_CAPABILITY_ADMIN,
    IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS, IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS,
    IDENTITY_CAPABILITY_RECOVER, IDENTITY_CAPABILITY_WRITE, IDENTITY_GRAPH_KEY_ACCEPTANCE_TYPE,
    IDENTITY_GRAPH_ROSTER_TYPE, IDENTITY_PURPOSE_APP, IDENTITY_PURPOSE_PROFILE,
    IDENTITY_PURPOSE_RECOVERY, IDENTITY_PURPOSE_REMOTE_SIGNER, IdentityKey,
    IdentityKeyAcceptanceContent, IdentityKeyTombstone, IdentityRosterOp, IdentityRosterOpContent,
    IdentityRosterProjection, IdentitySecretEpoch, SignedIdentityRosterOp,
    build_identity_key_acceptance_event, build_identity_roster_op_event_with_options, fact,
    parse_identity_key_acceptance_event, parse_identity_roster_op_event, project_identity_roster,
};
use nostr_sdk::ToBech32;
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{
    Alphabet, Event, EventId, Filter, JsonUtil, Keys, PublicKey, SingleLetterTag, TagKind,
};
use nostr_sdk::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const NOSTR_IDENTITY_ROSTER_SCHEMA: u32 = 1;
pub const KIND_NOSTR_IDENTITY_ROSTER_OP: u16 = FACT_OP_KIND;
pub const NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA: u32 = 1;
pub const KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE: u16 = FACT_OP_KIND;
pub const NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT: &str = "encrypted_device_labels";
pub const NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA: u32 = 1;
pub const NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX: &str = "nostr-identity://device-link/";
pub const NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION: u32 = 1;
pub const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE: &str =
    "nostr_identity_device_approval_receipt";
pub const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA: u32 = 1;
pub const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX: &str = "nostr-identity://device-approval/";
pub const NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX: &str =
    "nostr-identity://device-approval";
pub const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION: u32 = 1;
pub const NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE: &str = "nostr_identity_device_approval_proof";
pub const NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX: &str =
    "nostr_identity_device_approval:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NostrIdentityId(Uuid);

impl NostrIdentityId {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NostrIdentityEncryptedDeviceLabelsPayload {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
    pub secret_epoch: u64,
    pub labels: BTreeMap<String, String>,
    pub updated_at: i64,
}

impl fmt::Display for NostrIdentityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for NostrIdentityId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Error)]
pub enum NostrIdentityError {
    #[error("nostr event: {0}")]
    Event(String),
    #[error("invalid kind: expected {expected}, got {got}")]
    WrongKind { expected: u16, got: u16 },
    #[error("missing d tag")]
    MissingDTag,
    #[error("d tag malformed: {0}")]
    DTagMalformed(String),
    #[error("content not JSON-decodable: {0}")]
    BadContent(String),
    #[error("unsupported NostrIdentity schema {0}")]
    UnsupportedSchema(u32),
    #[error("signature verification failed: {0}")]
    SignatureFailed(String),
    #[error("invalid pubkey hex: {0}")]
    InvalidPubkey(String),
    #[error("invalid event id hex: {0}")]
    InvalidEventId(String),
    #[error("invalid facet acceptance: {0}")]
    InvalidFacetAcceptance(String),
    #[error("event signer {signer} does not match op actor {actor}")]
    ActorSignerMismatch { signer: String, actor: String },
    #[error("event signer {signer} does not match accepted facet {facet}")]
    FacetSignerMismatch { signer: String, facet: String },
    #[error("d-tag profile {d_tag_profile} does not match content profile {content_profile}")]
    ProfileMismatch {
        d_tag_profile: NostrIdentityId,
        content_profile: NostrIdentityId,
    },
    #[error("d-tag nonce {d_tag_nonce} does not match content nonce {content_nonce}")]
    NonceMismatch {
        d_tag_nonce: String,
        content_nonce: String,
    },
    #[error(
        "event created_at {event_created_at} does not match content created_at {content_created_at}"
    )]
    CreatedAtMismatch {
        event_created_at: i64,
        content_created_at: i64,
    },
    #[error("op profile {op_profile} does not match log profile {log_profile}")]
    LogProfileMismatch {
        log_profile: NostrIdentityId,
        op_profile: NostrIdentityId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NostrIdentityKeyPurpose {
    AppKey,
    RecoveryPhrase,
    Nip46Signer,
    SocialProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct NostrIdentityCapabilities {
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_write_roots: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_admin_profile: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_recover_app_keys: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_receive_secret_wraps: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_decrypt_secret_epochs: bool,
}

impl NostrIdentityCapabilities {
    #[must_use]
    pub fn app_admin() -> Self {
        Self {
            can_write_roots: true,
            can_admin_profile: true,
            can_recover_app_keys: false,
            can_receive_secret_wraps: true,
            can_decrypt_secret_epochs: true,
        }
    }

    #[must_use]
    pub fn app_writer() -> Self {
        Self {
            can_write_roots: true,
            can_admin_profile: false,
            can_recover_app_keys: false,
            can_receive_secret_wraps: true,
            can_decrypt_secret_epochs: true,
        }
    }

    #[must_use]
    pub fn app_reader() -> Self {
        Self {
            can_write_roots: false,
            can_admin_profile: false,
            can_recover_app_keys: false,
            can_receive_secret_wraps: true,
            can_decrypt_secret_epochs: true,
        }
    }

    #[must_use]
    pub fn recovery_phrase() -> Self {
        Self {
            can_write_roots: false,
            can_admin_profile: false,
            can_recover_app_keys: true,
            can_receive_secret_wraps: true,
            can_decrypt_secret_epochs: true,
        }
    }

    #[must_use]
    pub fn nip46_recovery(can_decrypt_secret_epochs: bool) -> Self {
        Self {
            can_write_roots: false,
            can_admin_profile: false,
            can_recover_app_keys: true,
            can_receive_secret_wraps: can_decrypt_secret_epochs,
            can_decrypt_secret_epochs,
        }
    }

    #[must_use]
    pub fn social_profile() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn can_change_secret_epochs(&self) -> bool {
        self.can_decrypt_secret_epochs && (self.can_admin_profile || self.can_recover_app_keys)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NostrIdentityFacet {
    pub pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<NostrIdentityId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub purposes: BTreeSet<NostrIdentityKeyPurpose>,
    #[serde(default)]
    pub capabilities: NostrIdentityCapabilities,
    pub added_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl NostrIdentityFacet {
    #[must_use]
    pub fn app_key(
        pubkey: impl Into<String>,
        added_at: i64,
        _label: Option<String>,
        capabilities: NostrIdentityCapabilities,
    ) -> Self {
        Self::with_purposes(
            pubkey,
            [NostrIdentityKeyPurpose::AppKey],
            capabilities,
            added_at,
            None,
        )
    }

    #[must_use]
    pub fn recovery_phrase(pubkey: impl Into<String>, added_at: i64) -> Self {
        Self::with_purposes(
            pubkey,
            [NostrIdentityKeyPurpose::RecoveryPhrase],
            NostrIdentityCapabilities::recovery_phrase(),
            added_at,
            Some("Recovery key".to_string()),
        )
    }

    #[must_use]
    pub fn nip46(
        pubkey: impl Into<String>,
        added_at: i64,
        label: Option<String>,
        can_decrypt_secret_epochs: bool,
    ) -> Self {
        Self::with_purposes(
            pubkey,
            [NostrIdentityKeyPurpose::Nip46Signer],
            NostrIdentityCapabilities::nip46_recovery(can_decrypt_secret_epochs),
            added_at,
            label,
        )
    }

    #[must_use]
    pub fn social_profile(pubkey: impl Into<String>, added_at: i64, label: Option<String>) -> Self {
        Self::with_purposes(
            pubkey,
            [NostrIdentityKeyPurpose::SocialProfile],
            NostrIdentityCapabilities::social_profile(),
            added_at,
            label,
        )
    }

    #[must_use]
    pub fn with_purposes<I>(
        pubkey: impl Into<String>,
        purposes: I,
        capabilities: NostrIdentityCapabilities,
        added_at: i64,
        label: Option<String>,
    ) -> Self
    where
        I: IntoIterator<Item = NostrIdentityKeyPurpose>,
    {
        Self {
            pubkey: pubkey.into(),
            profile_id: None,
            purposes: purposes.into_iter().collect(),
            capabilities,
            added_at,
            label,
        }
    }

    #[must_use]
    pub fn with_profile_id(mut self, profile_id: NostrIdentityId) -> Self {
        self.profile_id = Some(profile_id);
        self
    }

    #[must_use]
    pub fn has_purpose(&self, purpose: NostrIdentityKeyPurpose) -> bool {
        self.purposes.contains(&purpose)
    }

    #[must_use]
    pub fn is_app_key(&self) -> bool {
        self.has_purpose(NostrIdentityKeyPurpose::AppKey)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NostrIdentitySecretEpoch {
    pub epoch: u64,
    pub created_at: i64,
    pub signed_by_pubkey: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wrapped_secrets: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NostrIdentityTombstone {
    pub pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<NostrIdentityId>,
    pub removed_by_pubkey: String,
    pub removed_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NostrIdentityRosterOp {
    AddFacet {
        facet: NostrIdentityFacet,
    },
    TombstoneFacet {
        pubkey: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    SetCapabilities {
        pubkey: String,
        capabilities: NostrIdentityCapabilities,
    },
    RotateSecretEpoch {
        epoch: u64,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        wrapped_secrets: BTreeMap<String, String>,
    },
    RepairSecretWraps {
        epoch: u64,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        wrapped_secrets: BTreeMap<String, String>,
    },
}

impl NostrIdentityRosterOp {
    #[must_use]
    pub fn target_pubkey(&self) -> Option<&str> {
        match self {
            Self::AddFacet { facet } => Some(&facet.pubkey),
            Self::TombstoneFacet { pubkey, .. } | Self::SetCapabilities { pubkey, .. } => {
                Some(pubkey)
            }
            Self::RotateSecretEpoch { .. } | Self::RepairSecretWraps { .. } => None,
        }
    }

    #[must_use]
    pub fn mentioned_pubkeys(&self) -> BTreeSet<&str> {
        let mut pubkeys = BTreeSet::new();
        match self {
            Self::AddFacet { facet } => {
                pubkeys.insert(facet.pubkey.as_str());
            }
            Self::TombstoneFacet { pubkey, .. } | Self::SetCapabilities { pubkey, .. } => {
                pubkeys.insert(pubkey.as_str());
            }
            Self::RotateSecretEpoch {
                wrapped_secrets, ..
            }
            | Self::RepairSecretWraps {
                wrapped_secrets, ..
            } => {
                pubkeys.extend(wrapped_secrets.keys().map(String::as_str));
            }
        }
        pubkeys
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NostrIdentityRosterOpContent {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
    pub actor_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    pub client_nonce: String,
    pub created_at: i64,
    pub op: NostrIdentityRosterOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedNostrIdentityRosterOp {
    pub op_id: String,
    pub signer_pubkey: String,
    pub content: NostrIdentityRosterOpContent,
    pub event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NostrIdentityDeviceApprovalReceipt {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
    pub request_pubkey: String,
    pub device_app_key_pubkey: String,
    pub approved_by_pubkey: String,
    pub approved_at: i64,
    pub request_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster_op_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_roster_event: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NostrIdentityDeviceLinkInvite {
    pub profile_id: NostrIdentityId,
    pub admin_app_key_pubkey: String,
    pub invite_pubkey: String,
}

#[derive(Debug, Clone)]
pub struct CreateNostrIdentityDeviceLinkInviteOptions {
    pub profile_id: NostrIdentityId,
    pub admin_app_key_pubkey: String,
    pub invite_keys: Option<Keys>,
}

#[derive(Debug, Clone)]
pub struct LocalNostrIdentityDeviceLinkInvite {
    pub invite: NostrIdentityDeviceLinkInvite,
    pub invite_keys: Keys,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NostrIdentityDeviceLinkInvitePayload {
    v: u32,
    profile_id: NostrIdentityId,
    admin_app_key_npub: String,
    invite_npub: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NostrIdentityDeviceApprovalRequest {
    pub request_pubkey: String,
    pub device_app_key_pubkey: String,
    pub request_secret: String,
    pub device_app_key_proof: String,
    pub requested_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<NostrIdentityDeviceApprovalRequestedResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<NostrIdentityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_app_key_pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityCompactDeviceApprovalRequest {
    pub device_app_key_pubkey: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NostrIdentityDeviceApprovalRequestedResource {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateNostrIdentityDeviceApprovalRequestOptions {
    pub request_keys: Option<Keys>,
    pub request_secret: Option<String>,
    pub requested_at: i64,
    pub request_type: Option<String>,
    pub resources: Vec<NostrIdentityDeviceApprovalRequestedResource>,
    pub expires_at: Option<i64>,
    pub profile_id: Option<NostrIdentityId>,
    pub admin_app_key_pubkey: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LocalNostrIdentityDeviceApprovalRequest {
    pub request: NostrIdentityDeviceApprovalRequest,
    pub request_keys: Keys,
}

#[derive(Debug, Clone)]
pub struct ApproveNostrIdentityDeviceApprovalRequestOptions {
    pub request: NostrIdentityDeviceApprovalRequest,
    pub profile_id: NostrIdentityId,
    pub roster_ops: Vec<SignedNostrIdentityRosterOp>,
    pub approved_by_pubkey: String,
    pub approved_at: i64,
    pub client_nonce: Option<String>,
    pub capabilities: Option<NostrIdentityCapabilities>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NostrIdentityDeviceApprovalRequestPayload {
    v: u32,
    request_npub: String,
    device_app_key_npub: String,
    request_secret: String,
    device_app_key_proof: String,
    requested_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    resources: Vec<NostrIdentityDeviceApprovalRequestedResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile_id: Option<NostrIdentityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    admin_app_key_npub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NostrIdentityFacetAcceptanceContent {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
    pub facet_pubkey: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub purposes: BTreeSet<NostrIdentityKeyPurpose>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster_op_id: Option<String>,
    pub client_nonce: String,
    pub accepted_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedNostrIdentityFacetAcceptance {
    pub acceptance_id: String,
    pub signer_pubkey: String,
    pub content: NostrIdentityFacetAcceptanceContent,
    pub event_json: String,
}

impl SignedNostrIdentityFacetAcceptance {
    #[must_use]
    pub fn is_active_in_roster(&self, projection: &NostrIdentityRosterProjection) -> bool {
        if self.content.profile_id != projection.profile_id {
            return false;
        }
        projection
            .active_facets
            .get(&self.content.facet_pubkey)
            .is_some_and(|facet| self.content.purposes.is_subset(&facet.purposes))
    }
}

pub fn build_nostr_identity_roster_op_event(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    parents: Vec<String>,
    actor_seq: Option<u64>,
    op: NostrIdentityRosterOp,
    created_at: i64,
) -> Result<Event, NostrIdentityError> {
    build_nostr_identity_roster_op_event_with_encrypted_device_labels(
        signer_keys,
        profile_id,
        parents,
        actor_seq,
        op,
        created_at,
        None,
    )
}

pub fn build_nostr_identity_roster_op_event_with_encrypted_device_labels(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    parents: Vec<String>,
    actor_seq: Option<u64>,
    op: NostrIdentityRosterOp,
    created_at: i64,
    encrypted_device_labels: Option<String>,
) -> Result<Event, NostrIdentityError> {
    build_nostr_identity_roster_op_event_with_client_nonce(
        signer_keys,
        profile_id,
        parents,
        actor_seq,
        op,
        created_at,
        Uuid::new_v4().to_string(),
        encrypted_device_labels,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_nostr_identity_roster_op_event_with_client_nonce(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    parents: Vec<String>,
    actor_seq: Option<u64>,
    op: NostrIdentityRosterOp,
    created_at: i64,
    client_nonce: impl Into<String>,
    encrypted_device_labels: Option<String>,
) -> Result<Event, NostrIdentityError> {
    let client_nonce = require_non_empty(client_nonce.into(), "client_nonce")?;
    let content = NostrIdentityRosterOpContent {
        schema: NOSTR_IDENTITY_ROSTER_SCHEMA,
        profile_id,
        actor_pubkey: signer_keys.public_key().to_hex(),
        actor_seq,
        parents,
        client_nonce: client_nonce.clone(),
        created_at,
        op,
    };
    build_identity_roster_op_event_with_options(
        signer_keys,
        profile_id.as_uuid(),
        nostr_identity_roster_op_to_identity(content.op)?,
        BuildIdentityRosterOpEventOptions {
            parents: content.parents,
            actor_seq: content.actor_seq,
            client_nonce: content.client_nonce,
            created_at: non_negative_u64(content.created_at, "created_at")?,
            extension_facts: encrypted_device_labels_extension_facts(encrypted_device_labels),
        },
    )
    .map_err(|e| NostrIdentityError::Event(e.to_string()))
}

pub fn build_nostr_identity_facet_acceptance_event<I>(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    purposes: I,
    roster_op_id: Option<String>,
    accepted_at: i64,
) -> Result<Event, NostrIdentityError>
where
    I: IntoIterator<Item = NostrIdentityKeyPurpose>,
{
    let client_nonce = Uuid::new_v4().to_string();
    let content = NostrIdentityFacetAcceptanceContent {
        schema: NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA,
        profile_id,
        facet_pubkey: signer_keys.public_key().to_hex(),
        purposes: purposes.into_iter().collect(),
        roster_op_id,
        client_nonce: client_nonce.clone(),
        accepted_at,
    };
    validate_facet_acceptance_content(&content)?;
    build_identity_key_acceptance_event(
        signer_keys,
        profile_id.as_uuid(),
        content
            .purposes
            .into_iter()
            .map(nostr_identity_purpose_to_identity),
        content.roster_op_id,
        content.client_nonce,
        non_negative_u64(content.accepted_at, "accepted_at")?,
    )
    .map_err(|e| NostrIdentityError::Event(e.to_string()))
}

#[must_use]
pub fn nostr_identity_roster_parent_ids(ops: &[SignedNostrIdentityRosterOp]) -> Vec<String> {
    let Some(first) = ops.first() else {
        return Vec::new();
    };
    project_nostr_identity_roster(first.content.profile_id, ops.to_vec()).accepted_op_ids
}

#[must_use]
pub fn encrypted_device_label_payloads_from_nostr_identity_roster_op_event(
    event: &Event,
) -> Vec<String> {
    encrypted_profile_payloads_from_nostr_identity_roster_op_event(
        event,
        NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT,
    )
}

#[must_use]
pub fn encrypted_profile_payloads_from_nostr_identity_roster_op_event(
    event: &Event,
    fact_name: &str,
) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().is_some_and(|kind| kind == fact_name))
                .then(|| parts.get(1).map(|value| value.trim().to_string()))
                .flatten()
        })
        .filter(|value| !value.is_empty())
        .collect()
}

#[must_use]
pub fn nostr_identity_roster_op_d_tag(profile_id: NostrIdentityId, client_nonce: &str) -> String {
    format!("nostr_identity-profile/{profile_id}/roster-op/{client_nonce}")
}

#[must_use]
pub fn nostr_identity_facet_acceptance_d_tag(
    profile_id: NostrIdentityId,
    client_nonce: &str,
) -> String {
    format!("nostr_identity-profile/{profile_id}/facet-acceptance/{client_nonce}")
}

#[must_use]
pub fn nostr_identity_tag_kind() -> TagKind<'static> {
    TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::I))
}

#[must_use]
pub fn is_nostr_identity_roster_op_event_coordinate(event: &Event) -> bool {
    event.kind.as_u16() == KIND_NOSTR_IDENTITY_ROSTER_OP
        && fact_event_has_type(event, IDENTITY_GRAPH_ROSTER_TYPE)
}

#[must_use]
pub fn is_nostr_identity_facet_acceptance_event_coordinate(event: &Event) -> bool {
    event.kind.as_u16() == KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE
        && fact_event_has_type(event, IDENTITY_GRAPH_KEY_ACCEPTANCE_TYPE)
}

pub fn parse_nostr_identity_roster_op_event(
    event: &Event,
) -> Result<SignedNostrIdentityRosterOp, NostrIdentityError> {
    let kind = event.kind.as_u16();
    if kind != KIND_NOSTR_IDENTITY_ROSTER_OP {
        return Err(NostrIdentityError::WrongKind {
            expected: KIND_NOSTR_IDENTITY_ROSTER_OP,
            got: kind,
        });
    }
    let signed = parse_identity_roster_op_event(event)
        .map_err(|e| NostrIdentityError::BadContent(format!("Nostr identity roster op: {e}")))?;
    let content = identity_roster_content_to_nostr_identity(&signed.content)?;
    let event_created_at = i64::try_from(event.created_at.as_secs()).unwrap_or(i64::MAX);
    if content.created_at != event_created_at {
        return Err(NostrIdentityError::CreatedAtMismatch {
            event_created_at,
            content_created_at: content.created_at,
        });
    }
    let signer_pubkey = event.pubkey.to_hex();
    if signer_pubkey != content.actor_pubkey {
        return Err(NostrIdentityError::ActorSignerMismatch {
            signer: signer_pubkey,
            actor: content.actor_pubkey,
        });
    }
    validate_pubkey(&content.actor_pubkey)?;
    for pubkey in content.op.mentioned_pubkeys() {
        validate_pubkey(pubkey)?;
    }
    Ok(SignedNostrIdentityRosterOp {
        op_id: event.id.to_hex(),
        signer_pubkey,
        content,
        event_json: event.as_json(),
    })
}

pub fn create_nostr_identity_device_link_invite(
    options: CreateNostrIdentityDeviceLinkInviteOptions,
) -> Result<LocalNostrIdentityDeviceLinkInvite, NostrIdentityError> {
    let invite_keys = options.invite_keys.unwrap_or_else(Keys::generate);
    let invite = NostrIdentityDeviceLinkInvite {
        profile_id: options.profile_id,
        admin_app_key_pubkey: normalize_nostr_pubkey(
            &options.admin_app_key_pubkey,
            "admin AppKey",
        )?,
        invite_pubkey: invite_keys.public_key().to_hex(),
    };
    Ok(LocalNostrIdentityDeviceLinkInvite {
        invite: normalize_device_link_invite(invite)?,
        invite_keys,
    })
}

pub fn encode_nostr_identity_device_link_invite(
    invite: &NostrIdentityDeviceLinkInvite,
    prefix: Option<&str>,
) -> Result<String, NostrIdentityError> {
    let invite = normalize_device_link_invite(invite.clone())?;
    let payload = NostrIdentityDeviceLinkInvitePayload {
        v: NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION,
        profile_id: invite.profile_id,
        admin_app_key_npub: pubkey_to_npub(&invite.admin_app_key_pubkey)?,
        invite_npub: pubkey_to_npub(&invite.invite_pubkey)?,
    };
    let json = serde_json::to_string(&payload)
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    Ok(format!(
        "{}{}",
        prefix.unwrap_or(NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX),
        base64_url_encode(json.as_bytes())
    ))
}

pub fn parse_nostr_identity_device_link_invite(
    input: &str,
    prefixes: &[&str],
) -> Result<Option<NostrIdentityDeviceLinkInvite>, NostrIdentityError> {
    let Some(payload) =
        payload_from_prefixed_url(input, prefixes, NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX)
    else {
        return Ok(None);
    };
    let payload: NostrIdentityDeviceLinkInvitePayload =
        serde_json::from_str(&base64_url_decode_utf8(payload)?)
            .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    if payload.v != NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION {
        return Err(NostrIdentityError::UnsupportedSchema(payload.v));
    }
    let invite = NostrIdentityDeviceLinkInvite {
        profile_id: payload.profile_id,
        admin_app_key_pubkey: npub_or_hex_to_pubkey(&payload.admin_app_key_npub, "admin AppKey")?,
        invite_pubkey: npub_or_hex_to_pubkey(&payload.invite_npub, "invite")?,
    };
    Ok(Some(normalize_device_link_invite(invite)?))
}

pub fn create_nostr_identity_device_approval_request(
    device_app_key_keys: &Keys,
    options: CreateNostrIdentityDeviceApprovalRequestOptions,
) -> Result<LocalNostrIdentityDeviceApprovalRequest, NostrIdentityError> {
    let request_keys = options.request_keys.unwrap_or_else(Keys::generate);
    let request_secret = require_request_secret(
        options
            .request_secret
            .unwrap_or_else(random_device_approval_secret),
    )?;
    let requested_at = non_negative_u64(options.requested_at, "requested_at")?;
    let request_type =
        normalize_optional_device_approval_string(options.request_type, "request_type", 128)?;
    let resources = normalize_device_approval_resources(options.resources)?;
    let admin_app_key_pubkey = options
        .admin_app_key_pubkey
        .map(|value| normalize_nostr_pubkey(&value, "admin AppKey"))
        .transpose()?;
    let label = normalize_optional_label(options.label);
    let proof = build_nostr_identity_device_approval_proof_event(
        device_app_key_keys,
        NostrIdentityDeviceApprovalProofOptions {
            request_pubkey: request_keys.public_key().to_hex(),
            requested_at: options.requested_at,
            request_type: request_type.clone(),
            resources: resources.clone(),
            expires_at: options.expires_at,
            profile_id: options.profile_id,
            admin_app_key_pubkey: admin_app_key_pubkey.clone(),
            label: label.clone(),
        },
    )?;
    let request = NostrIdentityDeviceApprovalRequest {
        request_pubkey: request_keys.public_key().to_hex(),
        device_app_key_pubkey: device_app_key_keys.public_key().to_hex(),
        request_secret,
        device_app_key_proof: proof.as_json(),
        requested_at: i64::try_from(requested_at).unwrap_or(i64::MAX),
        request_type,
        resources,
        expires_at: options.expires_at,
        profile_id: options.profile_id,
        admin_app_key_pubkey,
        label,
    };
    Ok(LocalNostrIdentityDeviceApprovalRequest {
        request: normalize_device_approval_request(request)?,
        request_keys,
    })
}

pub fn encode_nostr_identity_device_approval_request(
    request: &NostrIdentityDeviceApprovalRequest,
    prefix: Option<&str>,
) -> Result<String, NostrIdentityError> {
    let request = normalize_device_approval_request(request.clone())?;
    let payload = NostrIdentityDeviceApprovalRequestPayload {
        v: NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION,
        request_npub: pubkey_to_npub(&request.request_pubkey)?,
        device_app_key_npub: pubkey_to_npub(&request.device_app_key_pubkey)?,
        request_secret: request.request_secret,
        device_app_key_proof: request.device_app_key_proof,
        requested_at: request.requested_at,
        request_type: request.request_type,
        resources: request.resources,
        expires_at: request.expires_at,
        profile_id: request.profile_id,
        admin_app_key_npub: request
            .admin_app_key_pubkey
            .as_deref()
            .map(pubkey_to_npub)
            .transpose()?,
        label: request.label,
    };
    let json = serde_json::to_string(&payload)
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    Ok(format!(
        "{}{}",
        prefix.unwrap_or(NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX),
        base64_url_encode(json.as_bytes())
    ))
}

pub fn encode_compact_nostr_identity_device_approval_request(
    device_app_key_pubkey: &str,
    prefix: Option<&str>,
) -> Result<String, NostrIdentityError> {
    let device_app_key_pubkey = normalize_nostr_pubkey(device_app_key_pubkey, "device AppKey")?;
    let prefix = prefix
        .unwrap_or(NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX)
        .trim()
        .trim_end_matches('?');
    if prefix.is_empty() {
        return Err(NostrIdentityError::BadContent(
            "compact device approval prefix is empty".to_string(),
        ));
    }
    Ok(format!("{prefix}?app_key={device_app_key_pubkey}"))
}

pub fn parse_compact_nostr_identity_device_approval_request(
    input: &str,
    prefixes: &[&str],
) -> Result<Option<NostrIdentityCompactDeviceApprovalRequest>, NostrIdentityError> {
    let Some(query) = query_from_prefixed_url(
        input,
        prefixes,
        NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX,
    ) else {
        return Ok(None);
    };
    let app_key = query_value(query, "app_key")
        .or_else(|| query_value(query, "device"))
        .ok_or_else(|| {
            NostrIdentityError::BadContent("device request is missing app_key".to_string())
        })?;
    Ok(Some(NostrIdentityCompactDeviceApprovalRequest {
        device_app_key_pubkey: normalize_nostr_pubkey(&app_key, "device AppKey")?,
    }))
}

#[must_use]
pub fn compact_nostr_identity_device_approval_request_has_prefix(
    input: &str,
    prefixes: &[&str],
) -> bool {
    query_from_prefixed_url(
        input,
        prefixes,
        NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX,
    )
    .is_some()
}

pub fn parse_nostr_identity_device_approval_request(
    input: &str,
    prefixes: &[&str],
) -> Result<Option<NostrIdentityDeviceApprovalRequest>, NostrIdentityError> {
    let Some(payload) = payload_from_prefixed_url(
        input,
        prefixes,
        NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX,
    ) else {
        return Ok(None);
    };
    let payload: NostrIdentityDeviceApprovalRequestPayload =
        serde_json::from_str(&base64_url_decode_utf8(payload)?)
            .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    if payload.v != NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION {
        return Err(NostrIdentityError::UnsupportedSchema(payload.v));
    }
    let request = NostrIdentityDeviceApprovalRequest {
        request_pubkey: npub_or_hex_to_pubkey(&payload.request_npub, "request")?,
        device_app_key_pubkey: npub_or_hex_to_pubkey(
            &payload.device_app_key_npub,
            "device AppKey",
        )?,
        request_secret: payload.request_secret,
        device_app_key_proof: payload.device_app_key_proof,
        requested_at: payload.requested_at,
        request_type: payload.request_type,
        resources: payload.resources,
        expires_at: payload.expires_at,
        profile_id: payload.profile_id,
        admin_app_key_pubkey: payload
            .admin_app_key_npub
            .map(|value| npub_or_hex_to_pubkey(&value, "admin AppKey"))
            .transpose()?,
        label: payload.label,
    };
    Ok(Some(normalize_device_approval_request(request)?))
}

pub fn approve_nostr_identity_device_approval_request(
    options: ApproveNostrIdentityDeviceApprovalRequestOptions,
) -> Result<NostrIdentityRosterOpContent, NostrIdentityError> {
    let request = normalize_device_approval_request(options.request)?;
    if let Some(request_profile_id) = request.profile_id
        && request_profile_id != options.profile_id
    {
        return Err(NostrIdentityError::BadContent(
            "device approval request profile mismatch".to_string(),
        ));
    }
    let approved_by_pubkey =
        normalize_nostr_pubkey(&options.approved_by_pubkey, "approving AppKey")?;
    if let Some(admin_app_key_pubkey) = &request.admin_app_key_pubkey
        && admin_app_key_pubkey != &approved_by_pubkey
    {
        return Err(NostrIdentityError::BadContent(
            "device approval request admin mismatch".to_string(),
        ));
    }
    let client_nonce = match options.client_nonce {
        Some(value) => require_non_empty(value, "client_nonce")?,
        None => nostr_identity_device_approval_client_nonce(&random_device_approval_secret())?,
    };
    let capabilities = options
        .capabilities
        .unwrap_or_else(NostrIdentityCapabilities::app_writer);
    Ok(NostrIdentityRosterOpContent {
        schema: NOSTR_IDENTITY_ROSTER_SCHEMA,
        profile_id: options.profile_id,
        actor_pubkey: approved_by_pubkey,
        actor_seq: None,
        parents: nostr_identity_roster_parent_ids(&options.roster_ops),
        client_nonce,
        created_at: options.approved_at,
        op: NostrIdentityRosterOp::AddFacet {
            facet: NostrIdentityFacet::app_key(
                request.device_app_key_pubkey,
                options.approved_at,
                None,
                capabilities,
            )
            .with_profile_id(options.profile_id),
        },
    })
}

pub fn nostr_identity_device_approval_client_nonce(
    random_value: &str,
) -> Result<String, NostrIdentityError> {
    Ok(format!(
        "{NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX}{}",
        require_request_secret(random_value.to_string())?
    ))
}

pub fn build_nostr_identity_device_approval_receipt_event(
    signer_keys: &Keys,
    receipt: NostrIdentityDeviceApprovalReceipt,
) -> Result<Event, NostrIdentityError> {
    validate_device_approval_receipt(&receipt)?;
    let signer_pubkey = signer_keys.public_key().to_hex();
    if receipt.approved_by_pubkey != signer_pubkey {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt signer mismatch".to_string(),
        ));
    }
    if let Some(signed_roster_event) = &receipt.signed_roster_event {
        let event = Event::from_json(signed_roster_event)
            .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
        let signed = parse_nostr_identity_roster_op_event(&event)?;
        validate_device_approval_receipt_roster_op(&receipt, &signed)?;
    }
    let request_pubkey = PublicKey::from_hex(&receipt.request_pubkey)
        .map_err(|error| NostrIdentityError::InvalidPubkey(error.to_string()))?;
    let encrypted = nip44::encrypt(
        signer_keys.secret_key(),
        &request_pubkey,
        serde_json::to_string(&receipt)
            .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?,
        Nip44Version::V2,
    )
    .map_err(|error| NostrIdentityError::Event(error.to_string()))?;
    let profile_id = receipt.profile_id.to_string();
    EventBuilder::new(Kind::from(FACT_OP_KIND), encrypted)
        .tag(
            Tag::parse(["type", NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE])
                .map_err(|error| NostrIdentityError::Event(error.to_string()))?,
        )
        .tag(
            Tag::parse(["p", receipt.request_pubkey.as_str()])
                .map_err(|error| NostrIdentityError::Event(error.to_string()))?,
        )
        .tag(
            Tag::parse(["i", profile_id.as_str(), "subject"])
                .map_err(|error| NostrIdentityError::Event(error.to_string()))?,
        )
        .custom_created_at(nostr_sdk::Timestamp::from(non_negative_u64(
            receipt.approved_at,
            "approved_at",
        )?))
        .sign_with_keys(signer_keys)
        .map_err(|error| NostrIdentityError::Event(error.to_string()))
}

pub fn parse_nostr_identity_device_approval_receipt_event(
    event: &Event,
    request_keys: &Keys,
) -> Result<NostrIdentityDeviceApprovalReceipt, NostrIdentityError> {
    if event.kind.as_u16() != FACT_OP_KIND {
        return Err(NostrIdentityError::WrongKind {
            expected: FACT_OP_KIND,
            got: event.kind.as_u16(),
        });
    }
    if !fact_event_has_type(event, NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE) {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt type tag missing".to_string(),
        ));
    }
    let request_pubkey = request_keys.public_key().to_hex();
    if !event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().is_some_and(|name| name == "p")
            && parts.get(1).is_some_and(|value| value == &request_pubkey)
    }) {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt request pubkey mismatch".to_string(),
        ));
    }
    let plaintext = nip44::decrypt(request_keys.secret_key(), &event.pubkey, &event.content)
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    let receipt: NostrIdentityDeviceApprovalReceipt = serde_json::from_str(&plaintext)
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    validate_device_approval_receipt(&receipt)?;
    if receipt.request_pubkey != request_pubkey {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt request mismatch".to_string(),
        ));
    }
    if receipt.approved_by_pubkey != event.pubkey.to_hex() {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt signer mismatch".to_string(),
        ));
    }
    let event_created_at = i64::try_from(event.created_at.as_secs()).map_err(|_| {
        NostrIdentityError::BadContent(
            "device approval receipt timestamp overflows i64".to_string(),
        )
    })?;
    if receipt.approved_at != event_created_at {
        return Err(NostrIdentityError::CreatedAtMismatch {
            event_created_at,
            content_created_at: receipt.approved_at,
        });
    }
    if let Some(signed_roster_event) = &receipt.signed_roster_event {
        let roster_event = Event::from_json(signed_roster_event)
            .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
        let signed = parse_nostr_identity_roster_op_event(&roster_event)?;
        validate_device_approval_receipt_roster_op(&receipt, &signed)?;
    }
    Ok(receipt)
}

pub fn parse_nostr_identity_device_approval_receipt_roster_op(
    receipt: &NostrIdentityDeviceApprovalReceipt,
) -> Result<SignedNostrIdentityRosterOp, NostrIdentityError> {
    let Some(signed_roster_event) = &receipt.signed_roster_event else {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt missing signed roster event".to_string(),
        ));
    };
    let event = Event::from_json(signed_roster_event)
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    let signed = parse_nostr_identity_roster_op_event(&event)?;
    validate_device_approval_receipt_roster_op(receipt, &signed)?;
    Ok(signed)
}

pub fn validate_signed_nostr_identity_roster_op(
    signed: &SignedNostrIdentityRosterOp,
) -> Result<(), NostrIdentityError> {
    let event = Event::from_json(&signed.event_json)
        .map_err(|error| NostrIdentityError::Event(error.to_string()))?;
    let parsed = parse_nostr_identity_roster_op_event(&event)?;
    if parsed.op_id != signed.op_id
        || parsed.signer_pubkey != signed.signer_pubkey
        || parsed.content != signed.content
    {
        return Err(NostrIdentityError::Event(
            "roster op event_json does not match op fields".to_string(),
        ));
    }
    Ok(())
}

fn canonical_signed_nostr_identity_roster_op(
    signed: &SignedNostrIdentityRosterOp,
) -> Result<SignedNostrIdentityRosterOp, NostrIdentityError> {
    let event = Event::from_json(&signed.event_json)
        .map_err(|error| NostrIdentityError::Event(error.to_string()))?;
    let parsed = parse_nostr_identity_roster_op_event(&event)?;
    if parsed.op_id != signed.op_id || parsed.signer_pubkey != signed.signer_pubkey {
        return Err(NostrIdentityError::Event(
            "roster op event_json does not match op identity fields".to_string(),
        ));
    }
    Ok(parsed)
}

pub fn parse_nostr_identity_facet_acceptance_event(
    event: &Event,
) -> Result<SignedNostrIdentityFacetAcceptance, NostrIdentityError> {
    let kind = event.kind.as_u16();
    if kind != KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE {
        return Err(NostrIdentityError::WrongKind {
            expected: KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE,
            got: kind,
        });
    }
    let signed = parse_identity_key_acceptance_event(event).map_err(|e| {
        NostrIdentityError::BadContent(format!("Nostr identity key acceptance: {e}"))
    })?;
    let content = identity_key_acceptance_content_to_nostr_identity(&signed.content)?;
    let event_created_at = i64::try_from(event.created_at.as_secs()).unwrap_or(i64::MAX);
    if content.accepted_at != event_created_at {
        return Err(NostrIdentityError::CreatedAtMismatch {
            event_created_at,
            content_created_at: content.accepted_at,
        });
    }
    let signer_pubkey = event.pubkey.to_hex();
    if signer_pubkey != content.facet_pubkey {
        return Err(NostrIdentityError::FacetSignerMismatch {
            signer: signer_pubkey,
            facet: content.facet_pubkey,
        });
    }
    validate_facet_acceptance_content(&content)?;
    Ok(SignedNostrIdentityFacetAcceptance {
        acceptance_id: event.id.to_hex(),
        signer_pubkey,
        content,
        event_json: event.as_json(),
    })
}

#[must_use]
pub fn nostr_identity_ids_from_facet_acceptances<'a, I>(
    facet_pubkey: &str,
    acceptances: I,
) -> Vec<NostrIdentityId>
where
    I: IntoIterator<Item = &'a SignedNostrIdentityFacetAcceptance>,
{
    acceptances
        .into_iter()
        .filter(|acceptance| acceptance.content.facet_pubkey == facet_pubkey)
        .map(|acceptance| acceptance.content.profile_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn nostr_identity_candidate_ids_for_pubkey_from_events<'a, I>(
    pubkey: &str,
    events: I,
) -> Result<Vec<NostrIdentityId>, NostrIdentityError>
where
    I: IntoIterator<Item = &'a Event>,
{
    validate_pubkey(pubkey)?;
    let mut profile_ids = BTreeSet::new();
    for event in events {
        if is_nostr_identity_facet_acceptance_event_coordinate(event) {
            if let Ok(acceptance) = parse_nostr_identity_facet_acceptance_event(event)
                && acceptance.content.facet_pubkey == pubkey
            {
                profile_ids.insert(acceptance.content.profile_id);
            }
        } else if is_nostr_identity_roster_op_event_coordinate(event)
            && let Ok(op) = parse_nostr_identity_roster_op_event(event)
            && (op.signer_pubkey == pubkey || op.content.op.mentioned_pubkeys().contains(pubkey))
        {
            profile_ids.insert(op.content.profile_id);
        }
    }
    Ok(profile_ids.into_iter().collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityAppKeyApprovalCandidate {
    pub profile_id: NostrIdentityId,
    pub app_key_pubkey: String,
    pub admin_app_key_pubkey: String,
    pub accepted_roster_op_count: usize,
    pub active_app_key_count: usize,
    pub latest_roster_op_created_at: Option<i64>,
    pub profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
}

pub fn nostr_identity_app_key_approval_candidate_filters(
    app_key_pubkey: &str,
) -> Result<Vec<Filter>, NostrIdentityError> {
    let app_key = PublicKey::parse(app_key_pubkey).map_err(|error| {
        NostrIdentityError::BadContent(format!("invalid app key pubkey: {error}"))
    })?;
    Ok(vec![
        Filter::new()
            .kind(Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
            .pubkey(app_key),
    ])
}

pub fn nostr_identity_app_key_approval_candidates_from_events<'a, I>(
    app_key_pubkey: &str,
    events: I,
) -> Result<Vec<NostrIdentityAppKeyApprovalCandidate>, NostrIdentityError>
where
    I: IntoIterator<Item = &'a Event>,
{
    let app_key_pubkey = normalize_nostr_pubkey(app_key_pubkey, "app key")?;
    let events = events.into_iter().collect::<Vec<_>>();
    let candidate_ids: BTreeSet<_> = nostr_identity_candidate_ids_for_pubkey_from_events(
        &app_key_pubkey,
        events.iter().copied(),
    )?
    .into_iter()
    .collect();
    let mut roster_ops_by_profile =
        BTreeMap::<NostrIdentityId, BTreeMap<String, SignedNostrIdentityRosterOp>>::new();
    for event in events {
        let Ok(op) = parse_nostr_identity_roster_op_event(event) else {
            continue;
        };
        if candidate_ids.contains(&op.content.profile_id) {
            roster_ops_by_profile
                .entry(op.content.profile_id)
                .or_default()
                .insert(op.op_id.clone(), op);
        }
    }

    let mut candidates = Vec::new();
    for profile_id in candidate_ids {
        let profile_roster_ops = roster_ops_by_profile
            .remove(&profile_id)
            .unwrap_or_default()
            .into_values()
            .collect::<Vec<_>>();
        let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.clone());
        let Some(joining_facet) = projection.active_facets.get(&app_key_pubkey) else {
            continue;
        };
        if !joining_facet.is_app_key() || !joining_facet.capabilities.can_write_roots {
            continue;
        }
        let Some(admin_app_key_pubkey) =
            nostr_identity_projection_admin_app_key_pubkey(&projection)
        else {
            continue;
        };
        let accepted_op_ids = projection
            .accepted_op_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let latest_roster_op_created_at = profile_roster_ops
            .iter()
            .filter(|op| accepted_op_ids.contains(op.op_id.as_str()))
            .map(|op| op.content.created_at)
            .max();
        candidates.push(NostrIdentityAppKeyApprovalCandidate {
            profile_id,
            app_key_pubkey: app_key_pubkey.clone(),
            admin_app_key_pubkey,
            accepted_roster_op_count: projection.accepted_op_ids.len(),
            active_app_key_count: projection.active_app_key_pubkeys().len(),
            latest_roster_op_created_at,
            profile_roster_ops,
        });
    }
    candidates.sort_by(|left, right| {
        right
            .latest_roster_op_created_at
            .cmp(&left.latest_roster_op_created_at)
            .then_with(|| {
                right
                    .accepted_roster_op_count
                    .cmp(&left.accepted_roster_op_count)
            })
            .then_with(|| right.active_app_key_count.cmp(&left.active_app_key_count))
            .then_with(|| left.profile_id.cmp(&right.profile_id))
    });
    Ok(candidates)
}

fn nostr_identity_roster_op_to_identity(
    op: NostrIdentityRosterOp,
) -> Result<IdentityRosterOp, NostrIdentityError> {
    Ok(match op {
        NostrIdentityRosterOp::AddFacet { facet } => IdentityRosterOp::AddKey {
            key: nostr_identity_facet_to_identity(facet)?,
        },
        NostrIdentityRosterOp::TombstoneFacet { pubkey, reason } => {
            validate_pubkey(&pubkey)?;
            IdentityRosterOp::TombstoneKey { pubkey, reason }
        }
        NostrIdentityRosterOp::SetCapabilities {
            pubkey,
            capabilities,
        } => {
            validate_pubkey(&pubkey)?;
            IdentityRosterOp::SetKeyCapabilities {
                pubkey,
                capabilities: nostr_identity_capabilities_to_identity(capabilities),
            }
        }
        NostrIdentityRosterOp::RotateSecretEpoch {
            epoch,
            wrapped_secrets,
        } => IdentityRosterOp::RotateSecretEpoch {
            epoch,
            wrapped_secrets: normalize_wrapped_secrets(wrapped_secrets)?,
        },
        NostrIdentityRosterOp::RepairSecretWraps {
            epoch,
            wrapped_secrets,
        } => IdentityRosterOp::RepairSecretWraps {
            epoch,
            wrapped_secrets: normalize_wrapped_secrets(wrapped_secrets)?,
        },
    })
}

fn nostr_identity_facet_to_identity(
    facet: NostrIdentityFacet,
) -> Result<IdentityKey, NostrIdentityError> {
    validate_pubkey(&facet.pubkey)?;
    let is_app_key = facet.purposes.contains(&NostrIdentityKeyPurpose::AppKey);
    Ok(IdentityKey {
        pubkey: facet.pubkey,
        subject: facet.profile_id.map(|profile_id| profile_id.as_uuid()),
        purposes: facet
            .purposes
            .into_iter()
            .map(nostr_identity_purpose_to_identity)
            .collect(),
        capabilities: nostr_identity_capabilities_to_identity(facet.capabilities),
        added_at: non_negative_u64(facet.added_at, "facet added_at")?,
        label: (!is_app_key).then_some(facet.label).flatten(),
    })
}

fn identity_roster_content_to_nostr_identity(
    content: &IdentityRosterOpContent,
) -> Result<NostrIdentityRosterOpContent, NostrIdentityError> {
    if content.schema != u64::from(NOSTR_IDENTITY_ROSTER_SCHEMA) {
        return Err(NostrIdentityError::UnsupportedSchema(
            u32::try_from(content.schema).unwrap_or(u32::MAX),
        ));
    }
    Ok(NostrIdentityRosterOpContent {
        schema: NOSTR_IDENTITY_ROSTER_SCHEMA,
        profile_id: NostrIdentityId::from_uuid(content.identity),
        actor_pubkey: content.actor_pubkey.clone(),
        actor_seq: content.actor_seq,
        parents: content.parents.clone(),
        client_nonce: content.client_nonce.clone(),
        created_at: i64::try_from(content.created_at).map_err(|_| {
            NostrIdentityError::BadContent(
                "NostrIdentity roster created_at overflows i64".to_string(),
            )
        })?,
        op: identity_roster_op_to_nostr_identity(&content.op)?,
    })
}

fn identity_roster_op_to_nostr_identity(
    op: &IdentityRosterOp,
) -> Result<NostrIdentityRosterOp, NostrIdentityError> {
    Ok(match op {
        IdentityRosterOp::AddKey { key } => NostrIdentityRosterOp::AddFacet {
            facet: identity_key_to_nostr_identity_facet(key)?,
        },
        IdentityRosterOp::TombstoneKey { pubkey, reason } => {
            validate_pubkey(pubkey)?;
            NostrIdentityRosterOp::TombstoneFacet {
                pubkey: pubkey.clone(),
                reason: reason.clone(),
            }
        }
        IdentityRosterOp::SetKeyCapabilities {
            pubkey,
            capabilities,
        } => {
            validate_pubkey(pubkey)?;
            NostrIdentityRosterOp::SetCapabilities {
                pubkey: pubkey.clone(),
                capabilities: identity_capabilities_to_nostr_identity(capabilities)?,
            }
        }
        IdentityRosterOp::RotateSecretEpoch {
            epoch,
            wrapped_secrets,
        } => NostrIdentityRosterOp::RotateSecretEpoch {
            epoch: *epoch,
            wrapped_secrets: wrapped_secrets.clone(),
        },
        IdentityRosterOp::RepairSecretWraps {
            epoch,
            wrapped_secrets,
        } => NostrIdentityRosterOp::RepairSecretWraps {
            epoch: *epoch,
            wrapped_secrets: wrapped_secrets.clone(),
        },
    })
}

fn identity_key_to_nostr_identity_facet(
    key: &IdentityKey,
) -> Result<NostrIdentityFacet, NostrIdentityError> {
    validate_pubkey(&key.pubkey)?;
    let purposes = key
        .purposes
        .iter()
        .map(|purpose| identity_purpose_to_nostr_identity(purpose))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let is_app_key = purposes.contains(&NostrIdentityKeyPurpose::AppKey);
    Ok(NostrIdentityFacet {
        pubkey: key.pubkey.clone(),
        profile_id: key.subject.map(NostrIdentityId::from_uuid),
        purposes,
        capabilities: identity_capabilities_to_nostr_identity(&key.capabilities)?,
        added_at: i64::try_from(key.added_at).map_err(|_| {
            NostrIdentityError::BadContent("NostrIdentity key added_at overflows i64".to_string())
        })?,
        label: (!is_app_key).then_some(key.label.clone()).flatten(),
    })
}

fn signed_nostr_identity_roster_op_to_identity(
    signed: &SignedNostrIdentityRosterOp,
) -> Result<SignedIdentityRosterOp, NostrIdentityError> {
    Ok(SignedIdentityRosterOp {
        op_id: signed.op_id.clone(),
        signer_pubkey: signed.signer_pubkey.clone(),
        content: IdentityRosterOpContent {
            schema: u64::from(signed.content.schema),
            identity: signed.content.profile_id.as_uuid(),
            actor_pubkey: signed.content.actor_pubkey.clone(),
            actor_seq: signed.content.actor_seq,
            parents: signed.content.parents.clone(),
            client_nonce: signed.content.client_nonce.clone(),
            created_at: non_negative_u64(signed.content.created_at, "created_at")?,
            op: nostr_identity_roster_op_to_identity(signed.content.op.clone())?,
        },
    })
}

fn identity_roster_projection_to_nostr_identity(
    projection: IdentityRosterProjection,
) -> Result<NostrIdentityRosterProjection, NostrIdentityError> {
    Ok(NostrIdentityRosterProjection {
        profile_id: NostrIdentityId::from_uuid(projection.identity),
        active_facets: projection
            .active_keys
            .into_iter()
            .map(|(pubkey, key)| Ok((pubkey, identity_key_to_nostr_identity_facet(&key)?)))
            .collect::<Result<BTreeMap<_, _>, NostrIdentityError>>()?,
        tombstones: projection
            .tombstones
            .into_iter()
            .map(|(pubkey, tombstone)| {
                Ok((pubkey, identity_tombstone_to_nostr_identity(tombstone)?))
            })
            .collect::<Result<BTreeMap<_, _>, NostrIdentityError>>()?,
        secret_epochs: projection
            .secret_epochs
            .into_iter()
            .map(|(epoch, secret_epoch)| {
                Ok((
                    epoch,
                    identity_secret_epoch_to_nostr_identity(secret_epoch)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, NostrIdentityError>>()?,
        accepted_op_ids: projection.accepted_op_ids,
        rejected_op_ids: projection.rejected_op_ids,
    })
}

fn identity_tombstone_to_nostr_identity(
    tombstone: IdentityKeyTombstone,
) -> Result<NostrIdentityTombstone, NostrIdentityError> {
    validate_pubkey(&tombstone.pubkey)?;
    validate_pubkey(&tombstone.removed_by_pubkey)?;
    Ok(NostrIdentityTombstone {
        pubkey: tombstone.pubkey,
        profile_id: tombstone.subject.map(NostrIdentityId::from_uuid),
        removed_by_pubkey: tombstone.removed_by_pubkey,
        removed_at: i64::try_from(tombstone.removed_at).map_err(|_| {
            NostrIdentityError::BadContent(
                "NostrIdentity tombstone removed_at overflows i64".to_string(),
            )
        })?,
        reason: tombstone.reason,
    })
}

fn identity_secret_epoch_to_nostr_identity(
    secret_epoch: IdentitySecretEpoch,
) -> Result<NostrIdentitySecretEpoch, NostrIdentityError> {
    validate_pubkey(&secret_epoch.signed_by_pubkey)?;
    Ok(NostrIdentitySecretEpoch {
        epoch: secret_epoch.epoch,
        created_at: i64::try_from(secret_epoch.created_at).map_err(|_| {
            NostrIdentityError::BadContent(
                "NostrIdentity key epoch created_at overflows i64".to_string(),
            )
        })?,
        signed_by_pubkey: secret_epoch.signed_by_pubkey,
        wrapped_secrets: secret_epoch.wrapped_secrets,
    })
}

fn identity_key_acceptance_content_to_nostr_identity(
    content: &IdentityKeyAcceptanceContent,
) -> Result<NostrIdentityFacetAcceptanceContent, NostrIdentityError> {
    if content.schema != u64::from(NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA) {
        return Err(NostrIdentityError::UnsupportedSchema(
            u32::try_from(content.schema).unwrap_or(u32::MAX),
        ));
    }
    Ok(NostrIdentityFacetAcceptanceContent {
        schema: NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA,
        profile_id: NostrIdentityId::from_uuid(content.identity),
        facet_pubkey: content.key_pubkey.clone(),
        purposes: content
            .purposes
            .iter()
            .map(|purpose| identity_purpose_to_nostr_identity(purpose))
            .collect::<Result<BTreeSet<_>, _>>()?,
        roster_op_id: content.roster_op_id.clone(),
        client_nonce: content.client_nonce.clone(),
        accepted_at: i64::try_from(content.accepted_at).map_err(|_| {
            NostrIdentityError::BadContent(
                "NostrIdentity acceptance accepted_at overflows i64".to_string(),
            )
        })?,
    })
}

fn nostr_identity_capabilities_to_identity(capabilities: NostrIdentityCapabilities) -> Vec<String> {
    [
        (
            capabilities.can_write_roots,
            IDENTITY_CAPABILITY_WRITE.to_string(),
        ),
        (
            capabilities.can_admin_profile,
            IDENTITY_CAPABILITY_ADMIN.to_string(),
        ),
        (
            capabilities.can_recover_app_keys,
            IDENTITY_CAPABILITY_RECOVER.to_string(),
        ),
        (
            capabilities.can_receive_secret_wraps,
            IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS.to_string(),
        ),
        (
            capabilities.can_decrypt_secret_epochs,
            IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS.to_string(),
        ),
    ]
    .into_iter()
    .filter_map(|(enabled, capability)| enabled.then_some(capability))
    .collect()
}

fn identity_capabilities_to_nostr_identity(
    capabilities: &[String],
) -> Result<NostrIdentityCapabilities, NostrIdentityError> {
    let mut nostr_identity = NostrIdentityCapabilities::default();
    for capability in capabilities {
        match capability.as_str() {
            IDENTITY_CAPABILITY_WRITE => nostr_identity.can_write_roots = true,
            IDENTITY_CAPABILITY_ADMIN => nostr_identity.can_admin_profile = true,
            IDENTITY_CAPABILITY_RECOVER => nostr_identity.can_recover_app_keys = true,
            IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS => {
                nostr_identity.can_receive_secret_wraps = true
            }
            IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS => {
                nostr_identity.can_decrypt_secret_epochs = true
            }
            other => {
                return Err(NostrIdentityError::BadContent(format!(
                    "unsupported NostrIdentity capability {other}"
                )));
            }
        }
    }
    Ok(nostr_identity)
}

fn nostr_identity_purpose_to_identity(purpose: NostrIdentityKeyPurpose) -> String {
    match purpose {
        NostrIdentityKeyPurpose::AppKey => IDENTITY_PURPOSE_APP,
        NostrIdentityKeyPurpose::RecoveryPhrase => IDENTITY_PURPOSE_RECOVERY,
        NostrIdentityKeyPurpose::Nip46Signer => IDENTITY_PURPOSE_REMOTE_SIGNER,
        NostrIdentityKeyPurpose::SocialProfile => IDENTITY_PURPOSE_PROFILE,
    }
    .to_string()
}

fn identity_purpose_to_nostr_identity(
    purpose: &str,
) -> Result<NostrIdentityKeyPurpose, NostrIdentityError> {
    match purpose {
        IDENTITY_PURPOSE_APP => Ok(NostrIdentityKeyPurpose::AppKey),
        IDENTITY_PURPOSE_RECOVERY => Ok(NostrIdentityKeyPurpose::RecoveryPhrase),
        IDENTITY_PURPOSE_REMOTE_SIGNER => Ok(NostrIdentityKeyPurpose::Nip46Signer),
        IDENTITY_PURPOSE_PROFILE => Ok(NostrIdentityKeyPurpose::SocialProfile),
        other => Err(NostrIdentityError::BadContent(format!(
            "unsupported NostrIdentity purpose {other}"
        ))),
    }
}

fn normalize_wrapped_secrets(
    wrapped_secrets: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, NostrIdentityError> {
    wrapped_secrets
        .into_iter()
        .map(|(pubkey, wrapped)| {
            validate_pubkey(&pubkey)?;
            Ok((pubkey, wrapped))
        })
        .collect()
}

fn encrypted_device_labels_extension_facts(payload: Option<String>) -> Vec<crate::Fact> {
    payload
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty())
                .then(|| fact(NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT, &[trimmed]))
        })
        .into_iter()
        .collect()
}

fn normalize_device_link_invite(
    invite: NostrIdentityDeviceLinkInvite,
) -> Result<NostrIdentityDeviceLinkInvite, NostrIdentityError> {
    Ok(NostrIdentityDeviceLinkInvite {
        profile_id: invite.profile_id,
        admin_app_key_pubkey: normalize_nostr_pubkey(&invite.admin_app_key_pubkey, "admin AppKey")?,
        invite_pubkey: normalize_nostr_pubkey(&invite.invite_pubkey, "invite")?,
    })
}

#[derive(Debug, Clone)]
struct NostrIdentityDeviceApprovalProofOptions {
    request_pubkey: String,
    requested_at: i64,
    request_type: Option<String>,
    resources: Vec<NostrIdentityDeviceApprovalRequestedResource>,
    expires_at: Option<i64>,
    profile_id: Option<NostrIdentityId>,
    admin_app_key_pubkey: Option<String>,
    label: Option<String>,
}

fn build_nostr_identity_device_approval_proof_event(
    device_app_key_keys: &Keys,
    options: NostrIdentityDeviceApprovalProofOptions,
) -> Result<Event, NostrIdentityError> {
    let request_pubkey = normalize_nostr_pubkey(&options.request_pubkey, "request")?;
    let requested_at = non_negative_u64(options.requested_at, "requested_at")?;
    let request_type =
        normalize_optional_device_approval_string(options.request_type, "request_type", 128)?;
    let resources = normalize_device_approval_resources(options.resources)?;
    let admin_app_key_pubkey = options
        .admin_app_key_pubkey
        .map(|value| normalize_nostr_pubkey(&value, "admin AppKey"))
        .transpose()?;
    let label = normalize_optional_label(options.label);
    let mut tag_parts = vec![
        vec![
            "type".to_string(),
            NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE.to_string(),
        ],
        vec!["request_pubkey".to_string(), request_pubkey],
        vec!["requested_at".to_string(), options.requested_at.to_string()],
    ];
    if let Some(request_type) = request_type {
        tag_parts.push(vec!["request_type".to_string(), request_type]);
    }
    if !resources.is_empty() {
        tag_parts.push(vec![
            "requested_resources".to_string(),
            serde_json::to_string(&resources)
                .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?,
        ]);
    }
    if let Some(expires_at) = options.expires_at {
        tag_parts.push(vec!["expires_at".to_string(), expires_at.to_string()]);
    }
    if let Some(profile_id) = options.profile_id {
        tag_parts.push(vec!["profile_id".to_string(), profile_id.to_string()]);
    }
    if let Some(admin_app_key_pubkey) = admin_app_key_pubkey {
        tag_parts.push(vec!["admin_pubkey".to_string(), admin_app_key_pubkey]);
    }
    if let Some(label) = label {
        tag_parts.push(vec!["label".to_string(), label]);
    }
    let tags = tag_parts
        .into_iter()
        .map(tag_from_parts)
        .collect::<Result<Vec<_>, _>>()?;
    EventBuilder::new(Kind::from(FACT_OP_KIND), "")
        .tags(tags)
        .custom_created_at(nostr_sdk::Timestamp::from(requested_at))
        .sign_with_keys(device_app_key_keys)
        .map_err(|error| NostrIdentityError::Event(error.to_string()))
}

fn normalize_device_approval_request(
    request: NostrIdentityDeviceApprovalRequest,
) -> Result<NostrIdentityDeviceApprovalRequest, NostrIdentityError> {
    let mut normalized = NostrIdentityDeviceApprovalRequest {
        request_pubkey: normalize_nostr_pubkey(&request.request_pubkey, "request")?,
        device_app_key_pubkey: normalize_nostr_pubkey(
            &request.device_app_key_pubkey,
            "device AppKey",
        )?,
        request_secret: require_request_secret(request.request_secret)?,
        device_app_key_proof: require_non_empty(
            request.device_app_key_proof,
            "device_app_key_proof",
        )?,
        requested_at: request.requested_at,
        request_type: normalize_optional_device_approval_string(
            request.request_type,
            "request_type",
            128,
        )?,
        resources: normalize_device_approval_resources(request.resources)?,
        expires_at: request.expires_at,
        profile_id: request.profile_id,
        admin_app_key_pubkey: request
            .admin_app_key_pubkey
            .map(|value| normalize_nostr_pubkey(&value, "admin AppKey"))
            .transpose()?,
        label: normalize_optional_label(request.label),
    };
    normalized.device_app_key_proof = require_valid_device_approval_proof(&normalized)?;
    Ok(normalized)
}

fn require_valid_device_approval_proof(
    request: &NostrIdentityDeviceApprovalRequest,
) -> Result<String, NostrIdentityError> {
    let raw = require_non_empty(request.device_app_key_proof.clone(), "device_app_key_proof")?;
    let event = Event::from_json(&raw)
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?;
    event
        .verify()
        .map_err(|error| NostrIdentityError::SignatureFailed(error.to_string()))?;
    if event.kind.as_u16() != FACT_OP_KIND {
        return Err(NostrIdentityError::WrongKind {
            expected: FACT_OP_KIND,
            got: event.kind.as_u16(),
        });
    }
    if !event.content.is_empty() {
        return Err(NostrIdentityError::BadContent(
            "device approval proof content must be empty".to_string(),
        ));
    }
    let signer = event.pubkey.to_hex();
    if signer != request.device_app_key_pubkey {
        return Err(NostrIdentityError::BadContent(
            "device approval proof signer mismatch".to_string(),
        ));
    }
    let requested_at = non_negative_u64(request.requested_at, "requested_at")?;
    if event.created_at.as_secs() != requested_at {
        return Err(NostrIdentityError::CreatedAtMismatch {
            event_created_at: i64::try_from(event.created_at.as_secs()).unwrap_or(i64::MAX),
            content_created_at: request.requested_at,
        });
    }
    require_proof_tag(&event, "type", NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE)?;
    require_proof_tag(&event, "request_pubkey", &request.request_pubkey)?;
    require_proof_tag(&event, "requested_at", &request.requested_at.to_string())?;
    require_optional_proof_tag(&event, "request_type", request.request_type.as_deref())?;
    let resources_json = if request.resources.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&request.resources)
                .map_err(|error| NostrIdentityError::BadContent(error.to_string()))?,
        )
    };
    require_optional_proof_tag(&event, "requested_resources", resources_json.as_deref())?;
    let expires_at = request.expires_at.map(|value| value.to_string());
    require_optional_proof_tag(&event, "expires_at", expires_at.as_deref())?;
    let profile_id = request.profile_id.map(|value| value.to_string());
    require_optional_proof_tag(&event, "profile_id", profile_id.as_deref())?;
    require_optional_proof_tag(
        &event,
        "admin_pubkey",
        request.admin_app_key_pubkey.as_deref(),
    )?;
    require_optional_proof_tag(&event, "label", request.label.as_deref())?;
    Ok(raw)
}

fn normalize_device_approval_resources(
    resources: Vec<NostrIdentityDeviceApprovalRequestedResource>,
) -> Result<Vec<NostrIdentityDeviceApprovalRequestedResource>, NostrIdentityError> {
    resources
        .into_iter()
        .map(|resource| {
            let mut scopes = Vec::new();
            for scope in resource.scopes {
                let scope = normalize_required_device_approval_string(scope, "scope", 256)?;
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
            Ok(NostrIdentityDeviceApprovalRequestedResource {
                resource_type: normalize_required_device_approval_string(
                    resource.resource_type,
                    "resource type",
                    256,
                )?,
                id: normalize_required_device_approval_string(resource.id, "resource id", 256)?,
                scopes,
            })
        })
        .collect()
}

fn normalize_optional_device_approval_string(
    value: Option<String>,
    label: &str,
    max_len: usize,
) -> Result<Option<String>, NostrIdentityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_string();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_len {
        return Err(NostrIdentityError::BadContent(format!(
            "NostrIdentity {label} is too long"
        )));
    }
    Ok(Some(value))
}

fn normalize_required_device_approval_string(
    value: String,
    label: &str,
    max_len: usize,
) -> Result<String, NostrIdentityError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(NostrIdentityError::BadContent(format!(
            "NostrIdentity {label} is required"
        )));
    }
    if value.len() > max_len {
        return Err(NostrIdentityError::BadContent(format!(
            "NostrIdentity {label} is too long"
        )));
    }
    Ok(value)
}

fn normalize_optional_label(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn require_request_secret(value: String) -> Result<String, NostrIdentityError> {
    let value = value.trim().to_string();
    if value.len() < 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(NostrIdentityError::BadContent(
            "device approval request secret must be at least 32 base64url characters".to_string(),
        ));
    }
    Ok(value)
}

fn random_device_approval_secret() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    base64_url_encode(&bytes)
}

fn normalize_nostr_pubkey(value: &str, label: &str) -> Result<String, NostrIdentityError> {
    npub_or_hex_to_pubkey(value, label)
}

fn npub_or_hex_to_pubkey(value: &str, label: &str) -> Result<String, NostrIdentityError> {
    PublicKey::parse(value.trim())
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| NostrIdentityError::BadContent(format!("invalid {label} pubkey: {error}")))
}

fn pubkey_to_npub(pubkey: &str) -> Result<String, NostrIdentityError> {
    PublicKey::from_hex(pubkey)
        .map_err(|error| NostrIdentityError::InvalidPubkey(error.to_string()))?
        .to_bech32()
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))
}

fn tag_from_parts(parts: Vec<String>) -> Result<Tag, NostrIdentityError> {
    Tag::parse(parts.iter().map(String::as_str))
        .map_err(|error| NostrIdentityError::Event(error.to_string()))
}

fn require_proof_tag(event: &Event, name: &str, expected: &str) -> Result<(), NostrIdentityError> {
    if proof_tag_value(event, name) == Some(expected) {
        return Ok(());
    }
    Err(NostrIdentityError::BadContent(format!(
        "device approval proof {name} mismatch"
    )))
}

fn require_optional_proof_tag(
    event: &Event,
    name: &str,
    expected: Option<&str>,
) -> Result<(), NostrIdentityError> {
    let actual = proof_tag_value(event, name);
    match (actual, expected) {
        (None, None) => Ok(()),
        (Some(actual), Some(expected)) if actual == expected => Ok(()),
        (Some(_), None) => Err(NostrIdentityError::BadContent(format!(
            "device approval proof unexpected {name}"
        ))),
        _ => Err(NostrIdentityError::BadContent(format!(
            "device approval proof {name} mismatch"
        ))),
    }
}

fn proof_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().is_some_and(|kind| kind == name))
            .then(|| parts.get(1).map(String::as_str))
            .flatten()
    })
}

fn payload_from_prefixed_url<'a>(
    input: &'a str,
    prefixes: &[&str],
    default_prefix: &str,
) -> Option<&'a str> {
    let value = strip_nostr_scheme(input.trim());
    if value.is_empty() {
        return None;
    }
    prefixes
        .iter()
        .copied()
        .filter(|prefix| !prefix.trim().is_empty())
        .chain(std::iter::once(default_prefix))
        .find_map(|prefix| {
            let prefix = prefix.trim();
            starts_with_ignore_ascii_case(value, prefix).then(|| {
                value[prefix.len()..]
                    .split(['?', '#'])
                    .next()
                    .unwrap_or("")
                    .trim()
            })
        })
        .filter(|payload| !payload.is_empty())
}

fn query_from_prefixed_url<'a>(
    input: &'a str,
    prefixes: &[&str],
    default_prefix: &str,
) -> Option<&'a str> {
    let value = strip_nostr_scheme(input.trim());
    if value.is_empty() {
        return None;
    }
    prefixes
        .iter()
        .copied()
        .filter(|prefix| !prefix.trim().is_empty())
        .chain(std::iter::once(default_prefix))
        .find_map(|prefix| {
            let prefix = prefix.trim();
            if !starts_with_ignore_ascii_case(value, prefix) {
                return None;
            }
            let rest = &value[prefix.len()..];
            if prefix.ends_with('?') {
                return Some(rest.split('#').next().unwrap_or("").trim());
            }
            rest.strip_prefix('?')
                .map(|query| query.split('#').next().unwrap_or("").trim())
        })
        .filter(|query| !query.is_empty())
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        key.eq_ignore_ascii_case(name)
            .then(|| percent_decode(value))
    })
}

fn percent_decode(value: &str) -> String {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = bytes.next();
            let lo = bytes.next();
            if let (Some(hi), Some(lo)) = (hi, lo)
                && let (Some(hi), Some(lo)) = (hex_digit(hi), hex_digit(lo))
            {
                out.push((hi << 4) | lo);
                continue;
            }
            out.push(byte);
            if let Some(hi) = hi {
                out.push(hi);
            }
            if let Some(lo) = lo {
                out.push(lo);
            }
        } else {
            out.push(byte);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn strip_nostr_scheme(value: &str) -> &str {
    if starts_with_ignore_ascii_case(value, "nostr:") {
        &value["nostr:".len()..]
    } else {
        value
    }
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn base64_url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        out.push(TABLE[(chunk[0] >> 2) as usize] as char);
        out.push(TABLE[(((chunk[0] & 0x03) << 4) | (chunk[1] >> 4)) as usize] as char);
        out.push(TABLE[(((chunk[1] & 0x0f) << 2) | (chunk[2] >> 6)) as usize] as char);
        out.push(TABLE[(chunk[2] & 0x3f) as usize] as char);
    }
    let remainder = chunks.remainder();
    if let [first] = remainder {
        out.push(TABLE[(first >> 2) as usize] as char);
        out.push(TABLE[((first & 0x03) << 4) as usize] as char);
    } else if let [first, second] = remainder {
        out.push(TABLE[(first >> 2) as usize] as char);
        out.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        out.push(TABLE[((second & 0x0f) << 2) as usize] as char);
    }
    out
}

fn base64_url_decode_utf8(value: &str) -> Result<String, NostrIdentityError> {
    String::from_utf8(base64_url_decode(value)?)
        .map_err(|error| NostrIdentityError::BadContent(error.to_string()))
}

fn base64_url_decode(value: &str) -> Result<Vec<u8>, NostrIdentityError> {
    let value = value.trim();
    if value.contains("...") || matches!(value, "<code>" | "<payload>" | "<invite>") {
        return Err(NostrIdentityError::BadContent(
            "device approval payload is a placeholder".to_string(),
        ));
    }
    if value.len() % 4 == 1 {
        return Err(NostrIdentityError::BadContent(
            "device approval payload is not base64url".to_string(),
        ));
    }
    let mut out = Vec::with_capacity(value.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in value.bytes() {
        let value = match byte {
            b'A'..=b'Z' => u32::from(byte - b'A'),
            b'a'..=b'z' => u32::from(byte - b'a' + 26),
            b'0'..=b'9' => u32::from(byte - b'0' + 52),
            b'-' => 62,
            b'_' => 63,
            _ => {
                return Err(NostrIdentityError::BadContent(
                    "device approval payload is not base64url".to_string(),
                ));
            }
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        while bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if bits > 0 && buffer != 0 {
        return Err(NostrIdentityError::BadContent(
            "device approval payload has invalid base64url padding bits".to_string(),
        ));
    }
    Ok(out)
}

fn non_negative_u64(value: i64, label: &str) -> Result<u64, NostrIdentityError> {
    u64::try_from(value).map_err(|_| {
        NostrIdentityError::BadContent(format!("NostrIdentity {label} must be non-negative"))
    })
}

fn fact_event_has_type(event: &Event, expected: &str) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() == 2 && parts[0] == "type" && parts[1] == expected
    })
}

#[derive(Debug, Clone)]
pub struct NostrIdentityRosterLog {
    pub profile_id: NostrIdentityId,
    ops: BTreeMap<String, SignedNostrIdentityRosterOp>,
}

impl NostrIdentityRosterLog {
    #[must_use]
    pub fn new(profile_id: NostrIdentityId) -> Self {
        Self {
            profile_id,
            ops: BTreeMap::new(),
        }
    }

    pub fn insert_event(&mut self, event: &Event) -> Result<bool, NostrIdentityError> {
        self.insert_signed_op(parse_nostr_identity_roster_op_event(event)?)
    }

    pub fn insert_signed_op(
        &mut self,
        op: SignedNostrIdentityRosterOp,
    ) -> Result<bool, NostrIdentityError> {
        if op.content.profile_id != self.profile_id {
            return Err(NostrIdentityError::LogProfileMismatch {
                log_profile: self.profile_id,
                op_profile: op.content.profile_id,
            });
        }
        let existed = self.ops.contains_key(&op.op_id);
        self.ops.insert(op.op_id.clone(), op);
        Ok(!existed)
    }

    pub fn merge(&mut self, other: &Self) -> Result<(), NostrIdentityError> {
        if other.profile_id != self.profile_id {
            return Err(NostrIdentityError::LogProfileMismatch {
                log_profile: self.profile_id,
                op_profile: other.profile_id,
            });
        }
        self.ops.extend(other.ops.clone());
        Ok(())
    }

    #[must_use]
    pub fn project(&self) -> NostrIdentityRosterProjection {
        project_nostr_identity_roster(self.profile_id, self.ops.values().cloned())
    }

    #[must_use]
    pub fn op_ids(&self) -> Vec<String> {
        self.ops.keys().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityRosterProjection {
    pub profile_id: NostrIdentityId,
    pub active_facets: BTreeMap<String, NostrIdentityFacet>,
    pub tombstones: BTreeMap<String, NostrIdentityTombstone>,
    pub secret_epochs: BTreeMap<u64, NostrIdentitySecretEpoch>,
    pub accepted_op_ids: Vec<String>,
    pub rejected_op_ids: Vec<String>,
}

impl NostrIdentityRosterProjection {
    #[must_use]
    pub fn can_write_roots(&self, pubkey: &str) -> bool {
        self.active_facets
            .get(pubkey)
            .is_some_and(|facet| facet.is_app_key() && facet.capabilities.can_write_roots)
    }

    #[must_use]
    pub fn can_admin_profile(&self, pubkey: &str) -> bool {
        self.active_facets
            .get(pubkey)
            .is_some_and(|facet| facet.capabilities.can_admin_profile)
    }

    #[must_use]
    pub fn active_app_key_pubkeys(&self) -> Vec<String> {
        self.active_facets
            .values()
            .filter(|facet| facet.is_app_key())
            .map(|facet| facet.pubkey.clone())
            .collect()
    }

    #[must_use]
    pub fn active_key_recipients_missing_wraps(&self, epoch: u64) -> Vec<String> {
        let Some(key_epoch) = self.secret_epochs.get(&epoch) else {
            return Vec::new();
        };
        self.active_facets
            .values()
            .filter(|facet| facet.capabilities.can_receive_secret_wraps)
            .filter(|facet| !key_epoch.wrapped_secrets.contains_key(&facet.pubkey))
            .map(|facet| facet.pubkey.clone())
            .collect()
    }

    #[must_use]
    pub fn secret_wrap_status(&self, pubkey: &str, epoch: u64) -> SecretWrapStatus {
        if self.tombstones.contains_key(pubkey) {
            return SecretWrapStatus::Tombstoned;
        }
        let Some(facet) = self.active_facets.get(pubkey) else {
            return SecretWrapStatus::NoSuchFacet;
        };
        if !facet.capabilities.can_receive_secret_wraps {
            return SecretWrapStatus::NotAKeyRecipient;
        }
        let Some(key_epoch) = self.secret_epochs.get(&epoch) else {
            return SecretWrapStatus::NoSuchEpoch;
        };
        if key_epoch.wrapped_secrets.contains_key(pubkey) {
            SecretWrapStatus::Available
        } else {
            SecretWrapStatus::RepairNeeded
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretWrapStatus {
    Available,
    RepairNeeded,
    NotAKeyRecipient,
    Tombstoned,
    NoSuchFacet,
    NoSuchEpoch,
}

#[must_use]
pub fn project_nostr_identity_roster<I>(
    profile_id: NostrIdentityId,
    ops: I,
) -> NostrIdentityRosterProjection
where
    I: IntoIterator<Item = SignedNostrIdentityRosterOp>,
{
    let mut rejected_op_ids = Vec::new();
    let mut identity_ops = Vec::new();
    let mut ops: Vec<_> = ops
        .into_iter()
        .filter(|op| op.content.profile_id == profile_id)
        .collect();
    ops.sort_by(|left, right| {
        left.content
            .created_at
            .cmp(&right.content.created_at)
            .then_with(|| left.op_id.cmp(&right.op_id))
    });

    for op in ops {
        let op_id = op.op_id.clone();
        let Ok(canonical_op) = canonical_signed_nostr_identity_roster_op(&op) else {
            rejected_op_ids.push(op_id);
            continue;
        };
        if let Ok(identity_op) = signed_nostr_identity_roster_op_to_identity(&canonical_op) {
            identity_ops.push(identity_op);
        } else {
            rejected_op_ids.push(op.op_id);
        }
    }
    let identity_projection = project_identity_roster(profile_id.as_uuid(), identity_ops);
    let mut projection = identity_roster_projection_to_nostr_identity(identity_projection)
        .unwrap_or_else(|_| NostrIdentityRosterProjection {
            profile_id,
            active_facets: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            secret_epochs: BTreeMap::new(),
            accepted_op_ids: Vec::new(),
            rejected_op_ids: Vec::new(),
        });
    projection.rejected_op_ids = rejected_op_ids
        .into_iter()
        .chain(projection.rejected_op_ids)
        .collect();
    projection
}

fn nostr_identity_projection_admin_app_key_pubkey(
    projection: &NostrIdentityRosterProjection,
) -> Option<String> {
    projection
        .active_facets
        .values()
        .find(|facet| facet.is_app_key() && facet.capabilities.can_admin_profile)
        .map(|facet| facet.pubkey.clone())
}

fn validate_pubkey(pubkey: &str) -> Result<(), NostrIdentityError> {
    PublicKey::from_hex(pubkey).map_err(|e| NostrIdentityError::InvalidPubkey(e.to_string()))?;
    Ok(())
}

fn validate_device_approval_receipt(
    receipt: &NostrIdentityDeviceApprovalReceipt,
) -> Result<(), NostrIdentityError> {
    if receipt.schema != NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA {
        return Err(NostrIdentityError::UnsupportedSchema(receipt.schema));
    }
    validate_pubkey(&receipt.request_pubkey)?;
    validate_pubkey(&receipt.device_app_key_pubkey)?;
    validate_pubkey(&receipt.approved_by_pubkey)?;
    if let Some(subject_pubkey) = &receipt.subject_pubkey {
        validate_pubkey(subject_pubkey)?;
    }
    if let Some(roster_op_id) = &receipt.roster_op_id {
        event_id_from_hex(roster_op_id)?;
    }
    require_non_empty(receipt.request_secret.clone(), "request_secret")?;
    non_negative_u64(receipt.approved_at, "approved_at")?;
    Ok(())
}

fn validate_device_approval_receipt_roster_op(
    receipt: &NostrIdentityDeviceApprovalReceipt,
    signed: &SignedNostrIdentityRosterOp,
) -> Result<(), NostrIdentityError> {
    if signed.content.profile_id != receipt.profile_id {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt roster profile mismatch".to_string(),
        ));
    }
    if signed.content.actor_pubkey != receipt.approved_by_pubkey {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt roster signer mismatch".to_string(),
        ));
    }
    if let Some(roster_op_id) = &receipt.roster_op_id
        && &signed.op_id != roster_op_id
    {
        return Err(NostrIdentityError::BadContent(
            "device approval receipt roster op id mismatch".to_string(),
        ));
    }
    match &signed.content.op {
        NostrIdentityRosterOp::AddFacet { facet }
            if facet.pubkey == receipt.device_app_key_pubkey =>
        {
            Ok(())
        }
        _ => Err(NostrIdentityError::BadContent(
            "device approval receipt roster does not add device".to_string(),
        )),
    }
}

fn require_non_empty(value: String, label: &str) -> Result<String, NostrIdentityError> {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        return Err(NostrIdentityError::BadContent(format!(
            "NostrIdentity {label} is required"
        )));
    }
    Ok(trimmed)
}

fn event_id_from_hex(event_id: &str) -> Result<EventId, NostrIdentityError> {
    EventId::from_hex(event_id).map_err(|e| NostrIdentityError::InvalidEventId(e.to_string()))
}

fn validate_facet_acceptance_content(
    content: &NostrIdentityFacetAcceptanceContent,
) -> Result<(), NostrIdentityError> {
    validate_pubkey(&content.facet_pubkey)?;
    if content.purposes.is_empty() {
        return Err(NostrIdentityError::InvalidFacetAcceptance(
            "purposes must not be empty".to_string(),
        ));
    }
    if let Some(roster_op_id) = &content.roster_op_id {
        event_id_from_hex(roster_op_id)?;
    }
    Ok(())
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::filter::MatchEventOptions;
    use nostr_sdk::{EventBuilder, Kind, Tag};

    fn signed_op(
        signer: &Keys,
        profile_id: NostrIdentityId,
        op: NostrIdentityRosterOp,
        created_at: i64,
    ) -> SignedNostrIdentityRosterOp {
        signed_op_with_parents(signer, profile_id, Vec::new(), op, created_at)
    }

    fn signed_op_with_parents(
        signer: &Keys,
        profile_id: NostrIdentityId,
        parents: Vec<String>,
        op: NostrIdentityRosterOp,
        created_at: i64,
    ) -> SignedNostrIdentityRosterOp {
        let event =
            build_nostr_identity_roster_op_event(signer, profile_id, parents, None, op, created_at)
                .unwrap();
        parse_nostr_identity_roster_op_event(&event).unwrap()
    }

    fn bootstrap_op(
        signer: &Keys,
        profile_id: NostrIdentityId,
        created_at: i64,
    ) -> SignedNostrIdentityRosterOp {
        signed_op(
            signer,
            profile_id,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    signer.public_key().to_hex(),
                    created_at,
                    Some("native app".to_string()),
                    NostrIdentityCapabilities::app_admin(),
                ),
            },
            created_at,
        )
    }

    fn project(
        profile_id: NostrIdentityId,
        ops: Vec<SignedNostrIdentityRosterOp>,
    ) -> NostrIdentityRosterProjection {
        project_nostr_identity_roster(profile_id, ops)
    }

    #[test]
    fn profile_id_is_standard_uuid_v4() {
        let profile_id = NostrIdentityId::new_v4();
        assert_eq!(profile_id.as_uuid().get_version_num(), 4);
        assert_eq!(profile_id.to_string().len(), 36);
    }

    #[test]
    fn signed_roster_op_roundtrips_as_verified_nostr_event() {
        let profile_id = NostrIdentityId::new_v4();
        let app = Keys::generate();
        let op = bootstrap_op(&app, profile_id, 10);

        assert_eq!(op.signer_pubkey, app.public_key().to_hex());
        assert_eq!(op.content.profile_id, profile_id);
        assert_eq!(op.content.actor_pubkey, app.public_key().to_hex());
        assert!(!op.op_id.is_empty());
        let event = Event::from_json(&op.event_json).unwrap();
        assert_eq!(event.kind, Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP));
        assert!(event.content.is_empty());
        assert!(
            !event
                .tags
                .iter()
                .any(|tag| tag.as_slice().first().is_some_and(|kind| kind == "d"))
        );
        assert!(event.tags.iter().any(|tag| {
            tag.as_slice()
                == [
                    "i".to_string(),
                    profile_id.to_string(),
                    "subject".to_string(),
                ]
        }));
        assert!(event.tags.iter().any(|tag| {
            tag.as_slice() == ["type".to_string(), IDENTITY_GRAPH_ROSTER_TYPE.to_string()]
        }));
    }

    #[test]
    fn device_approval_receipt_encrypts_secret_and_signed_roster_event() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let device = Keys::generate();
        let request = Keys::generate();
        let bootstrap = bootstrap_op(&admin, profile_id, 40);
        let approval_event = build_nostr_identity_roster_op_event_with_client_nonce(
            &admin,
            profile_id,
            vec![bootstrap.op_id],
            None,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    device.public_key().to_hex(),
                    42,
                    None,
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            42,
            "approval-public-nonce",
            None,
        )
        .unwrap();
        let approval = parse_nostr_identity_roster_op_event(&approval_event).unwrap();
        let request_secret = "secret_abcdefghijklmnopqrstuvwxyz123456".to_string();
        let receipt = NostrIdentityDeviceApprovalReceipt {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
            profile_id,
            request_pubkey: request.public_key().to_hex(),
            device_app_key_pubkey: device.public_key().to_hex(),
            approved_by_pubkey: admin.public_key().to_hex(),
            approved_at: 42,
            request_secret: request_secret.clone(),
            subject_pubkey: Some(admin.public_key().to_hex()),
            roster_op_id: Some(approval.op_id.clone()),
            signed_roster_event: Some(approval_event.as_json()),
        };

        let receipt_event =
            build_nostr_identity_device_approval_receipt_event(&admin, receipt).unwrap();
        assert!(fact_event_has_type(
            &receipt_event,
            NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE
        ));
        assert!(!receipt_event.content.contains(&request_secret));
        assert!(
            !receipt_event
                .tags
                .iter()
                .flat_map(|tag| tag.as_slice())
                .any(|value| value == &request_secret)
        );

        let parsed =
            parse_nostr_identity_device_approval_receipt_event(&receipt_event, &request).unwrap();
        assert_eq!(parsed.request_secret, request_secret);
        assert_eq!(parsed.subject_pubkey, Some(admin.public_key().to_hex()));
        let receipt_roster_op =
            parse_nostr_identity_device_approval_receipt_roster_op(&parsed).unwrap();
        assert_eq!(receipt_roster_op.op_id, approval.op_id);
    }

    #[test]
    fn device_link_invite_url_roundtrips_with_custom_or_default_prefix() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let invite_keys = Keys::generate();
        let local =
            create_nostr_identity_device_link_invite(CreateNostrIdentityDeviceLinkInviteOptions {
                profile_id,
                admin_app_key_pubkey: admin.public_key().to_bech32().unwrap(),
                invite_keys: Some(invite_keys.clone()),
            })
            .unwrap();

        assert_eq!(local.invite_keys.public_key(), invite_keys.public_key());
        assert_eq!(local.invite.profile_id, profile_id);
        assert_eq!(
            local.invite.admin_app_key_pubkey,
            admin.public_key().to_hex()
        );
        assert_eq!(
            local.invite.invite_pubkey,
            invite_keys.public_key().to_hex()
        );

        let default_encoded =
            encode_nostr_identity_device_link_invite(&local.invite, None).unwrap();
        let default_parsed = parse_nostr_identity_device_link_invite(&default_encoded, &[])
            .unwrap()
            .unwrap();
        assert_eq!(default_parsed, local.invite);

        let prefix = "https://drive.iris.to/invite/";
        let encoded =
            encode_nostr_identity_device_link_invite(&local.invite, Some(prefix)).unwrap();
        let parsed = parse_nostr_identity_device_link_invite(&encoded, &[prefix])
            .unwrap()
            .unwrap();
        assert_eq!(parsed, local.invite);
        assert!(encoded.starts_with(prefix));
        assert!(
            parse_nostr_identity_device_link_invite("https://example.com/nope", &[prefix])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn device_approval_request_url_roundtrips_and_rejects_tampering() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let device = Keys::generate();
        let request_keys = Keys::generate();
        let request_secret = "secret_abcdefghijklmnopqrstuvwxyz123456".to_string();
        let local = create_nostr_identity_device_approval_request(
            &device,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: Some(request_keys.clone()),
                request_secret: Some(request_secret.clone()),
                requested_at: 41,
                request_type: Some("device_link".to_string()),
                resources: vec![NostrIdentityDeviceApprovalRequestedResource {
                    resource_type: "chat_group".to_string(),
                    id: profile_id.to_string(),
                    scopes: vec!["admin".to_string()],
                }],
                expires_at: Some(101),
                profile_id: Some(profile_id),
                admin_app_key_pubkey: Some(admin.public_key().to_hex()),
                label: Some(" This device ".to_string()),
            },
        )
        .unwrap();
        let request = local.request;

        assert_eq!(local.request_keys.public_key(), request_keys.public_key());
        assert_eq!(request.request_pubkey, request_keys.public_key().to_hex());
        assert_eq!(request.device_app_key_pubkey, device.public_key().to_hex());
        assert_eq!(request.request_secret, request_secret);
        assert_eq!(request.label.as_deref(), Some("This device"));
        assert!(
            !request
                .device_app_key_proof
                .contains(&request.request_secret)
        );

        let prefix = "https://chat.iris.to/approve-device/";
        let encoded =
            encode_nostr_identity_device_approval_request(&request, Some(prefix)).unwrap();
        let parsed = parse_nostr_identity_device_approval_request(&encoded, &[prefix])
            .unwrap()
            .unwrap();
        assert_eq!(parsed, request);
        assert!(
            parse_nostr_identity_device_approval_request("https://example.com/nope", &[prefix])
                .unwrap()
                .is_none()
        );

        let proof_event = Event::from_json(&request.device_app_key_proof).unwrap();
        assert_eq!(proof_event.kind, Kind::from(FACT_OP_KIND));
        assert_eq!(proof_event.pubkey, device.public_key());
        assert!(proof_event.content.is_empty());
        assert!(!proof_event.as_json().contains(&request.request_secret));
        assert!(proof_event.tags.iter().any(|tag| {
            tag.as_slice() == ["request_pubkey".to_string(), request.request_pubkey.clone()]
        }));

        let payload = encoded.trim_start_matches(prefix);
        let mut tampered_payload: serde_json::Value =
            serde_json::from_str(&base64_url_decode_utf8(payload).unwrap()).unwrap();
        tampered_payload["resources"][0]["scopes"][0] = "read".into();
        let tampered = format!(
            "{prefix}{}",
            base64_url_encode(tampered_payload.to_string().as_bytes())
        );
        assert!(parse_nostr_identity_device_approval_request(&tampered, &[prefix]).is_err());

        let bootstrap = bootstrap_op(&admin, profile_id, 40);
        let approval_content = approve_nostr_identity_device_approval_request(
            ApproveNostrIdentityDeviceApprovalRequestOptions {
                request: parsed,
                profile_id,
                roster_ops: vec![bootstrap],
                approved_by_pubkey: admin.public_key().to_hex(),
                approved_at: 42,
                client_nonce: Some(
                    nostr_identity_device_approval_client_nonce(
                        "public_nonce_abcdefghijklmnopqrstuvwxyz123456",
                    )
                    .unwrap(),
                ),
                capabilities: None,
            },
        )
        .unwrap();
        assert!(
            approval_content
                .client_nonce
                .starts_with(NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX)
        );
        match approval_content.op {
            NostrIdentityRosterOp::AddFacet { facet } => {
                assert_eq!(facet.pubkey, device.public_key().to_hex());
                assert_eq!(facet.profile_id, Some(profile_id));
                assert_eq!(facet.capabilities, NostrIdentityCapabilities::app_writer());
            }
            other => panic!("expected add facet, got {other:?}"),
        }
    }

    #[test]
    fn compact_device_approval_request_url_contains_only_joining_app_key() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let device = Keys::generate();
        let prefix = "iris-drive://app-key-link";

        let encoded = encode_compact_nostr_identity_device_approval_request(
            &device.public_key().to_bech32().unwrap(),
            Some(prefix),
        )
        .unwrap();
        let parsed = parse_compact_nostr_identity_device_approval_request(&encoded, &[prefix])
            .unwrap()
            .unwrap();

        assert_eq!(parsed.device_app_key_pubkey, device.public_key().to_hex());
        assert_eq!(
            encoded,
            format!("{prefix}?app_key={}", device.public_key().to_hex())
        );
        assert!(encoded.len() < 120, "compact URL was {}", encoded.len());
        assert!(!encoded.contains(&profile_id.to_string()));
        assert!(!encoded.contains(&admin.public_key().to_hex()));
        assert!(compact_nostr_identity_device_approval_request_has_prefix(
            &format!("nostr:{encoded}"),
            &[prefix]
        ));

        let one_slash = format!(
            "iris-drive:/app-key-link?device={}",
            device.public_key().to_bech32().unwrap()
        );
        let parsed_one_slash = parse_compact_nostr_identity_device_approval_request(
            &one_slash,
            &[prefix, "iris-drive:/app-key-link?"],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            parsed_one_slash.device_app_key_pubkey,
            device.public_key().to_hex()
        );
        assert!(
            parse_compact_nostr_identity_device_approval_request(
                "iris-drive://app-key-link?app_key=not-a-key",
                &[prefix],
            )
            .is_err()
        );
    }

    #[test]
    fn app_key_approval_candidates_project_rosters_that_mention_joining_key() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let device = Keys::generate();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 40)];
        let approval = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    device.public_key().to_hex(),
                    41,
                    None,
                    NostrIdentityCapabilities::app_writer(),
                )
                .with_profile_id(profile_id),
            },
            41,
        );
        let approval_event = Event::from_json(&approval.event_json).unwrap();
        ops.push(approval);
        let events = ops
            .iter()
            .map(|op| Event::from_json(&op.event_json).unwrap())
            .collect::<Vec<_>>();

        let filters = nostr_identity_app_key_approval_candidate_filters(
            &device.public_key().to_bech32().unwrap(),
        )
        .unwrap();
        assert!(
            filters
                .iter()
                .any(|filter| filter.match_event(&approval_event, MatchEventOptions::default()))
        );

        let candidates = nostr_identity_app_key_approval_candidates_from_events(
            &device.public_key().to_hex(),
            &events,
        )
        .unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].profile_id, profile_id);
        assert_eq!(candidates[0].app_key_pubkey, device.public_key().to_hex());
        assert_eq!(
            candidates[0].admin_app_key_pubkey,
            admin.public_key().to_hex()
        );
        assert_eq!(candidates[0].profile_roster_ops.len(), events.len());
        assert_eq!(candidates[0].accepted_roster_op_count, 2);
        assert_eq!(candidates[0].active_app_key_count, 2);
        assert_eq!(candidates[0].latest_roster_op_created_at, Some(41));
    }

    #[test]
    fn encrypted_device_labels_are_signed_extension_facts_without_public_app_key_labels() {
        let profile_id = NostrIdentityId::new_v4();
        let app = Keys::generate();
        let event = build_nostr_identity_roster_op_event_with_encrypted_device_labels(
            &app,
            profile_id,
            Vec::new(),
            None,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    app.public_key().to_hex(),
                    10,
                    Some("Private laptop".to_string()),
                    NostrIdentityCapabilities::app_admin(),
                ),
            },
            10,
            Some("v1.encrypted-label-payload".to_string()),
        )
        .unwrap();

        assert_eq!(
            encrypted_device_label_payloads_from_nostr_identity_roster_op_event(&event),
            vec!["v1.encrypted-label-payload".to_string()]
        );
        assert!(
            !event.tags.iter().any(
                |tag| tag.as_slice() == ["key_label".to_string(), "Private laptop".to_string()]
            )
        );
        assert!(!event.as_json().contains("Private laptop"));

        let signed = parse_nostr_identity_roster_op_event(&event).unwrap();
        let projection = project(profile_id, vec![signed]);
        let facet = projection
            .active_facets
            .get(&app.public_key().to_hex())
            .expect("app key facet");
        assert_eq!(facet.label.as_deref(), None);
    }

    #[test]
    fn profile_roster_projection_uses_signed_event_over_cached_fields() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let mut op = bootstrap_op(&admin, profile_id, 10);
        let op_id = op.op_id.clone();
        let admin_pubkey = admin.public_key().to_hex();
        if let NostrIdentityRosterOp::AddFacet { facet } = &mut op.content.op {
            facet.label = Some("forged label".to_string());
        }

        let projection = project(profile_id, vec![op]);

        assert_eq!(projection.accepted_op_ids, vec![op_id]);
        assert!(projection.rejected_op_ids.is_empty());
        let facet = projection
            .active_facets
            .get(&admin_pubkey)
            .expect("signed roster event should project");
        assert_eq!(facet.label.as_deref(), None);
    }

    #[test]
    fn roster_ops_tag_mentioned_pubkeys_for_restore_discovery() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let recovery = Keys::generate();
        let recovery_pubkey = recovery.public_key();
        let event = build_nostr_identity_roster_op_event(
            &admin,
            profile_id,
            Vec::new(),
            None,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::recovery_phrase(recovery_pubkey.to_hex(), 11),
            },
            11,
        )
        .unwrap();

        let recovery_hex = recovery_pubkey.to_hex();
        assert!(event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() >= 2 && parts[0] == "p" && parts[1] == recovery_hex
        }));
    }

    #[test]
    fn facet_acceptance_breadcrumb_roundtrips_without_granting_authority() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let phone_pubkey = phone.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let add_phone = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    phone_pubkey.clone(),
                    11,
                    Some("phone".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            11,
        );
        let acceptance_event = build_nostr_identity_facet_acceptance_event(
            &phone,
            profile_id,
            [NostrIdentityKeyPurpose::AppKey],
            Some(add_phone.op_id.clone()),
            12,
        )
        .unwrap();
        let acceptance = parse_nostr_identity_facet_acceptance_event(&acceptance_event).unwrap();

        assert_eq!(acceptance.signer_pubkey, phone_pubkey);
        assert_eq!(acceptance.content.profile_id, profile_id);
        assert_eq!(
            nostr_identity_ids_from_facet_acceptances(&phone_pubkey, [&acceptance]),
            vec![profile_id]
        );
        assert!(!acceptance.is_active_in_roster(&project(profile_id, Vec::new())));

        ops.push(add_phone);
        let accepted_projection = project(profile_id, ops.clone());
        assert!(acceptance.is_active_in_roster(&accepted_projection));

        let remove_phone = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::TombstoneFacet {
                pubkey: phone_pubkey.clone(),
                reason: Some("lost".to_string()),
            },
            13,
        );
        ops.push(remove_phone);
        assert!(!acceptance.is_active_in_roster(&project(profile_id, ops)));
    }

    #[test]
    fn facet_acceptance_rejects_signer_mismatch() {
        let profile_id = NostrIdentityId::new_v4();
        let signer = Keys::generate();
        let other = Keys::generate();
        let client_nonce = Uuid::new_v4().to_string();
        let content = NostrIdentityFacetAcceptanceContent {
            schema: NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA,
            profile_id,
            facet_pubkey: other.public_key().to_hex(),
            purposes: [NostrIdentityKeyPurpose::AppKey].into_iter().collect(),
            roster_op_id: None,
            client_nonce: client_nonce.clone(),
            accepted_at: 12,
        };
        let event = EventBuilder::new(
            Kind::from(KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE),
            serde_json::to_string(&content).unwrap(),
        )
        .tags([
            Tag::identifier(nostr_identity_facet_acceptance_d_tag(
                profile_id,
                &client_nonce,
            )),
            Tag::custom(nostr_identity_tag_kind(), [profile_id.to_string()]),
            Tag::public_key(other.public_key()),
        ])
        .custom_created_at(nostr_sdk::Timestamp::from(12))
        .sign_with_keys(&signer)
        .unwrap();

        assert!(matches!(
            parse_nostr_identity_facet_acceptance_event(&event),
            Err(NostrIdentityError::BadContent(message))
                if message.contains("identity key acceptance signer mismatch")
                    || message.contains("fact events must have empty content")
        ));
    }

    #[test]
    fn candidate_profile_ids_are_discovered_from_roster_and_acceptance_events() {
        let profile_a = NostrIdentityId::new_v4();
        let profile_b = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let recovery = Keys::generate();
        let app = Keys::generate();
        let recovery_pubkey = recovery.public_key().to_hex();
        let mentioned_event = build_nostr_identity_roster_op_event(
            &admin,
            profile_a,
            Vec::new(),
            None,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::recovery_phrase(recovery_pubkey.clone(), 11),
            },
            11,
        )
        .unwrap();
        let acceptance_event = build_nostr_identity_facet_acceptance_event(
            &recovery,
            profile_a,
            [NostrIdentityKeyPurpose::RecoveryPhrase],
            None,
            12,
        )
        .unwrap();
        let signer_event = build_nostr_identity_roster_op_event(
            &recovery,
            profile_b,
            Vec::new(),
            None,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    app.public_key().to_hex(),
                    13,
                    None,
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            13,
        )
        .unwrap();
        let unrelated_event = EventBuilder::new(Kind::from(1_u16), "hello")
            .custom_created_at(nostr_sdk::Timestamp::from(14))
            .sign_with_keys(&admin)
            .unwrap();

        let candidates = nostr_identity_candidate_ids_for_pubkey_from_events(
            &recovery_pubkey,
            [
                &mentioned_event,
                &acceptance_event,
                &signer_event,
                &unrelated_event,
            ],
        )
        .unwrap()
        .into_iter()
        .collect::<BTreeSet<_>>();

        assert_eq!(candidates, BTreeSet::from([profile_a, profile_b]));
    }

    #[test]
    fn bootstrap_creates_first_app_key_admin() {
        let profile_id = NostrIdentityId::new_v4();
        let app = Keys::generate();
        let projection = project(profile_id, vec![bootstrap_op(&app, profile_id, 10)]);
        let app_pubkey = app.public_key().to_hex();

        assert!(projection.can_write_roots(&app_pubkey));
        assert!(projection.can_admin_profile(&app_pubkey));
        assert_eq!(projection.active_app_key_pubkeys(), vec![app_pubkey]);
        assert_eq!(projection.accepted_op_ids.len(), 1);
        assert!(projection.rejected_op_ids.is_empty());
    }

    #[test]
    fn non_admin_app_key_can_write_roots_but_cannot_mutate_roster() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let member = Keys::generate();
        let stranger = Keys::generate();
        let member_pubkey = member.public_key().to_hex();
        let stranger_pubkey = stranger.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let member_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    member_pubkey.clone(),
                    11,
                    Some("web app".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            11,
        );
        ops.push(member_op);
        let stranger_op = signed_op_with_parents(
            &member,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    stranger_pubkey.clone(),
                    12,
                    None,
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            12,
        );
        ops.push(stranger_op);

        let projection = project(profile_id, ops);

        assert!(projection.can_write_roots(&member_pubkey));
        assert!(!projection.can_admin_profile(&member_pubkey));
        assert!(!projection.active_facets.contains_key(&stranger_pubkey));
        assert_eq!(projection.accepted_op_ids.len(), 2);
        assert_eq!(projection.rejected_op_ids.len(), 1);
    }

    #[test]
    fn recovery_phrase_authorizes_fresh_app_key_without_becoming_roster_admin_or_root_writer() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let recovery = Keys::generate();
        let recovered_app = Keys::generate();
        let phone_pubkey = phone.public_key().to_hex();
        let recovery_pubkey = recovery.public_key().to_hex();
        let recovered_pubkey = recovered_app.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let phone_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    phone_pubkey.clone(),
                    11,
                    Some("phone".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            11,
        );
        ops.push(phone_op);
        let recovery_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::recovery_phrase(recovery_pubkey.clone(), 12),
            },
            12,
        );
        ops.push(recovery_op);
        let forbidden_tombstone = signed_op_with_parents(
            &recovery,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::TombstoneFacet {
                pubkey: phone_pubkey.clone(),
                reason: Some("recovery removed app actor".to_string()),
            },
            13,
        );
        ops.push(forbidden_tombstone);
        let recovered_op = signed_op_with_parents(
            &recovery,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    recovered_pubkey.clone(),
                    14,
                    Some("restored laptop".to_string()),
                    NostrIdentityCapabilities::app_admin(),
                ),
            },
            14,
        );
        ops.push(recovered_op);

        let projection = project(profile_id, ops);

        assert!(!projection.can_write_roots(&recovery_pubkey));
        assert!(!projection.can_admin_profile(&recovery_pubkey));
        assert!(!projection.active_facets.contains_key(&phone_pubkey));
        assert_eq!(
            projection
                .tombstones
                .get(&phone_pubkey)
                .and_then(|tombstone| tombstone.reason.as_deref()),
            Some("recovery removed app actor")
        );
        assert!(projection.can_write_roots(&recovered_pubkey));
        assert!(projection.can_admin_profile(&recovered_pubkey));
        assert_eq!(projection.accepted_op_ids.len(), 5);
        assert!(projection.rejected_op_ids.is_empty());
    }

    #[test]
    fn nip46_can_be_recovery_capable_and_receive_epoch_wraps() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let nip46 = Keys::generate();
        let nip46_pubkey = nip46.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let nip46_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::nip46(
                    nip46_pubkey.clone(),
                    11,
                    Some("bunker".to_string()),
                    true,
                ),
            },
            11,
        );
        ops.push(nip46_op);
        let epoch_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::RotateSecretEpoch {
                epoch: 1,
                wrapped_secrets: BTreeMap::from([
                    (admin.public_key().to_hex(), "wrap-admin".to_string()),
                    (nip46_pubkey.clone(), "wrap-nip46".to_string()),
                ]),
            },
            12,
        );
        ops.push(epoch_op);

        let projection = project(profile_id, ops);

        assert!(!projection.can_write_roots(&nip46_pubkey));
        assert!(!projection.can_admin_profile(&nip46_pubkey));
        assert_eq!(
            projection.secret_wrap_status(&nip46_pubkey, 1),
            SecretWrapStatus::Available
        );
    }

    #[test]
    fn signer_only_nip46_can_admit_app_key_but_not_rotate_epochs() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let nip46 = Keys::generate();
        let recovered_app = Keys::generate();
        let nip46_pubkey = nip46.public_key().to_hex();
        let recovered_pubkey = recovered_app.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let nip46_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::nip46(
                    nip46_pubkey.clone(),
                    11,
                    Some("signer only".to_string()),
                    false,
                ),
            },
            11,
        );
        ops.push(nip46_op);
        let recovered_op = signed_op_with_parents(
            &nip46,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    recovered_pubkey.clone(),
                    12,
                    Some("restored app".to_string()),
                    NostrIdentityCapabilities::app_admin(),
                ),
            },
            12,
        );
        ops.push(recovered_op);
        let epoch_op = signed_op_with_parents(
            &nip46,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::RotateSecretEpoch {
                epoch: 1,
                wrapped_secrets: BTreeMap::from([(
                    recovered_pubkey.clone(),
                    "wrap-recovered".to_string(),
                )]),
            },
            13,
        );
        ops.push(epoch_op);

        let projection = project(profile_id, ops);

        assert!(projection.can_write_roots(&recovered_pubkey));
        assert_eq!(
            projection.secret_wrap_status(&recovered_pubkey, 1),
            SecretWrapStatus::NoSuchEpoch
        );
        assert_eq!(projection.accepted_op_ids.len(), 3);
        assert_eq!(projection.rejected_op_ids.len(), 1);
    }

    #[test]
    fn repair_key_wraps_must_match_existing_epoch_signer() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let recovery = Keys::generate();
        let recovery_pubkey = recovery.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let recovery_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::recovery_phrase(recovery_pubkey.clone(), 11),
            },
            11,
        );
        ops.push(recovery_op);
        let epoch_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::RotateSecretEpoch {
                epoch: 1,
                wrapped_secrets: BTreeMap::from([(
                    admin.public_key().to_hex(),
                    "wrap-admin".to_string(),
                )]),
            },
            12,
        );
        ops.push(epoch_op);
        let repair_op = signed_op_with_parents(
            &recovery,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::RepairSecretWraps {
                epoch: 1,
                wrapped_secrets: BTreeMap::from([(
                    recovery_pubkey.clone(),
                    "wrap-recovery".to_string(),
                )]),
            },
            13,
        );
        ops.push(repair_op);

        let projection = project(profile_id, ops);

        assert_eq!(
            projection.secret_wrap_status(&recovery_pubkey, 1),
            SecretWrapStatus::RepairNeeded
        );
        assert_eq!(projection.accepted_op_ids.len(), 3);
        assert_eq!(projection.rejected_op_ids.len(), 1);
    }

    #[test]
    fn social_profile_facet_cannot_authorize_drive_access() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let social = Keys::generate();
        let attempted_app = Keys::generate();
        let social_pubkey = social.public_key().to_hex();
        let attempted_pubkey = attempted_app.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let social_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::social_profile(
                    social_pubkey.clone(),
                    11,
                    Some("nostr profile".to_string()),
                ),
            },
            11,
        );
        ops.push(social_op);
        let attempted_op = signed_op_with_parents(
            &social,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    attempted_pubkey.clone(),
                    12,
                    None,
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            12,
        );
        ops.push(attempted_op);

        let projection = project(profile_id, ops);

        assert!(projection.active_facets.contains_key(&social_pubkey));
        assert!(!projection.can_admin_profile(&social_pubkey));
        assert!(!projection.can_write_roots(&social_pubkey));
        assert!(!projection.active_facets.contains_key(&attempted_pubkey));
    }

    #[test]
    fn roster_ops_are_authorized_by_neutral_graph_state() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let writer = Keys::generate();
        let stale_invitee = Keys::generate();
        let valid_invitee = Keys::generate();
        let writer_pubkey = writer.public_key().to_hex();
        let stale_invitee_pubkey = stale_invitee.public_key().to_hex();
        let valid_invitee_pubkey = valid_invitee.public_key().to_hex();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let writer_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    writer_pubkey.clone(),
                    11,
                    Some("writer".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            11,
        );
        ops.push(writer_op);
        let stale_writer_view = nostr_identity_roster_parent_ids(&ops);
        let promote_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::SetCapabilities {
                pubkey: writer_pubkey.clone(),
                capabilities: NostrIdentityCapabilities::app_admin(),
            },
            12,
        );
        ops.push(promote_op);
        let stale_op = signed_op_with_parents(
            &writer,
            profile_id,
            stale_writer_view,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    stale_invitee_pubkey.clone(),
                    13,
                    Some("stale invite".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            13,
        );
        let stale_op_id = stale_op.op_id.clone();
        ops.push(stale_op);
        let parent_ids_after_stale_branch = nostr_identity_roster_parent_ids(&ops);
        assert!(parent_ids_after_stale_branch.contains(&stale_op_id));
        let valid_op = signed_op_with_parents(
            &writer,
            profile_id,
            parent_ids_after_stale_branch,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    valid_invitee_pubkey.clone(),
                    14,
                    Some("valid invite".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            14,
        );
        ops.push(valid_op);

        let projection = project(profile_id, ops);

        assert!(projection.can_admin_profile(&writer_pubkey));
        assert!(projection.can_write_roots(&stale_invitee_pubkey));
        assert!(projection.can_write_roots(&valid_invitee_pubkey));
        assert_eq!(projection.accepted_op_ids.len(), 5);
        assert!(projection.rejected_op_ids.is_empty());
    }

    #[test]
    fn roster_parent_projection_scales_with_dense_accepted_history() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let mut ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let mut parent_ids = vec![ops[0].op_id.clone()];

        for index in 0..32 {
            let device = Keys::generate();
            let op = signed_op_with_parents(
                &admin,
                profile_id,
                parent_ids.clone(),
                NostrIdentityRosterOp::AddFacet {
                    facet: NostrIdentityFacet::app_key(
                        device.public_key().to_hex(),
                        11 + index,
                        Some(format!("device {index}")),
                        NostrIdentityCapabilities::app_writer(),
                    ),
                },
                11 + index,
            );
            parent_ids.push(op.op_id.clone());
            ops.push(op);
        }

        let projection = project(profile_id, ops);

        assert_eq!(projection.accepted_op_ids.len(), 33);
        assert!(projection.rejected_op_ids.is_empty());
        assert_eq!(projection.active_facets.len(), 33);
    }

    #[test]
    fn divergent_roster_logs_merge_by_union_and_project_deterministically() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let recovery = Keys::generate();
        let bootstrap = bootstrap_op(&admin, profile_id, 10);
        let base_ops = vec![bootstrap.clone()];
        let phone_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&base_ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    phone.public_key().to_hex(),
                    11,
                    Some("phone".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            11,
        );
        let recovery_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&base_ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::recovery_phrase(recovery.public_key().to_hex(), 11),
            },
            11,
        );
        let mut left = NostrIdentityRosterLog::new(profile_id);
        left.insert_signed_op(bootstrap.clone()).unwrap();
        left.insert_signed_op(phone_op).unwrap();
        let mut right = NostrIdentityRosterLog::new(profile_id);
        right.insert_signed_op(bootstrap).unwrap();
        right.insert_signed_op(recovery_op).unwrap();

        left.merge(&right).unwrap();
        let projection = left.project();

        assert!(projection.can_write_roots(&phone.public_key().to_hex()));
        assert!(!projection.can_admin_profile(&recovery.public_key().to_hex()));
        assert_eq!(projection.accepted_op_ids.len(), 3);
    }

    #[test]
    fn tombstone_and_readd_follow_timestamp_order() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let phone_pubkey = phone.public_key().to_hex();
        let bootstrap = bootstrap_op(&admin, profile_id, 10);
        let mut ops = vec![bootstrap];
        let add_phone = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    phone_pubkey.clone(),
                    11,
                    Some("phone".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            11,
        );
        ops.push(add_phone);
        let remove_phone = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::TombstoneFacet {
                pubkey: phone_pubkey.clone(),
                reason: Some("lost".to_string()),
            },
            12,
        );
        ops.push(remove_phone);

        let tombstoned_projection = project(profile_id, ops.clone());

        assert!(tombstoned_projection.tombstones.contains_key(&phone_pubkey));
        assert!(
            !tombstoned_projection
                .active_facets
                .contains_key(&phone_pubkey)
        );
        assert_eq!(
            tombstoned_projection.secret_wrap_status(&phone_pubkey, 1),
            SecretWrapStatus::Tombstoned
        );

        let later_readd = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    phone_pubkey.clone(),
                    13,
                    Some("same key approved again".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            13,
        );
        ops.push(later_readd);
        let readded_projection = project(profile_id, ops);

        assert!(!readded_projection.tombstones.contains_key(&phone_pubkey));
        assert!(readded_projection.active_facets.contains_key(&phone_pubkey));
        assert_eq!(
            readded_projection
                .active_facets
                .get(&phone_pubkey)
                .unwrap()
                .label
                .as_deref(),
            None
        );
    }

    #[test]
    fn active_key_without_epoch_wrap_is_repair_needed_until_wrap_repair() {
        let profile_id = NostrIdentityId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let admin_pubkey = admin.public_key().to_hex();
        let phone_pubkey = phone.public_key().to_hex();
        let mut base_ops = vec![bootstrap_op(&admin, profile_id, 10)];
        let phone_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&base_ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    phone_pubkey.clone(),
                    11,
                    Some("phone".to_string()),
                    NostrIdentityCapabilities::app_writer(),
                ),
            },
            11,
        );
        base_ops.push(phone_op);
        let epoch_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&base_ops),
            NostrIdentityRosterOp::RotateSecretEpoch {
                epoch: 1,
                wrapped_secrets: BTreeMap::from([(admin_pubkey, "wrap-admin".to_string())]),
            },
            12,
        );
        base_ops.push(epoch_op);
        let needs_repair = project(profile_id, base_ops.clone());

        assert_eq!(
            needs_repair.secret_wrap_status(&phone_pubkey, 1),
            SecretWrapStatus::RepairNeeded
        );
        assert_eq!(
            needs_repair.active_key_recipients_missing_wraps(1),
            vec![phone_pubkey.clone()]
        );

        let mut repaired_ops = base_ops;
        let repair_op = signed_op_with_parents(
            &admin,
            profile_id,
            nostr_identity_roster_parent_ids(&repaired_ops),
            NostrIdentityRosterOp::RepairSecretWraps {
                epoch: 1,
                wrapped_secrets: BTreeMap::from([(phone_pubkey.clone(), "wrap-phone".to_string())]),
            },
            13,
        );
        repaired_ops.push(repair_op);
        let repaired = project(profile_id, repaired_ops);

        assert_eq!(
            repaired.secret_wrap_status(&phone_pubkey, 1),
            SecretWrapStatus::Available
        );
        assert!(repaired.active_key_recipients_missing_wraps(1).is_empty());
    }
}
