export const NOSTR_IDENTITY_ROSTER_SCHEMA = 1;
export const NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA = 1;
export const KIND_NOSTR_IDENTITY_ROSTER_OP = 7_368;
export const KIND_NOSTR_IDENTITY_FACET_ACCEPTANCE = 7_368;
export const NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT = 'encrypted_device_labels';
export const NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA = 1;

export type NostrIdentityId = string;
export type NostrIdentityKeyPurpose =
  | 'app_key'
  | 'recovery_phrase'
  | 'nip46_signer'
  | 'social_profile';

export interface NostrIdentityCapabilities {
  can_write_roots?: boolean;
  can_admin_profile?: boolean;
  can_recover_app_keys?: boolean;
  can_receive_secret_wraps?: boolean;
  can_decrypt_secret_epochs?: boolean;
}

export interface NostrIdentityFacet {
  pubkey: string;
  profile_id?: NostrIdentityId;
  purposes?: NostrIdentityKeyPurpose[];
  capabilities?: NostrIdentityCapabilities;
  added_at: number;
  label?: string;
}

export type NostrIdentityRosterOp =
  | { op: 'add_facet'; facet: NostrIdentityFacet }
  | { op: 'tombstone_facet'; pubkey: string; reason?: string }
  | { op: 'set_capabilities'; pubkey: string; capabilities: NostrIdentityCapabilities }
  | { op: 'rotate_secret_epoch'; epoch: number; wrapped_secrets?: Record<string, string> }
  | { op: 'repair_secret_wraps'; epoch: number; wrapped_secrets?: Record<string, string> };

export interface NostrIdentityRosterOpContent {
  schema: number;
  profile_id: NostrIdentityId;
  actor_pubkey: string;
  actor_seq?: number;
  parents?: string[];
  client_nonce: string;
  created_at: number;
  op: NostrIdentityRosterOp;
}

export interface SignedNostrIdentityRosterOp {
  op_id: string;
  signer_pubkey: string;
  content: NostrIdentityRosterOpContent;
  event_json: string;
}

export interface NostrIdentityFacetAcceptanceContent {
  schema: number;
  profile_id: NostrIdentityId;
  facet_pubkey: string;
  purposes: NostrIdentityKeyPurpose[];
  roster_op_id?: string;
  client_nonce: string;
  accepted_at: number;
}

export interface SignedNostrIdentityFacetAcceptance {
  acceptance_id: string;
  signer_pubkey: string;
  content: NostrIdentityFacetAcceptanceContent;
  event_json: string;
}

export interface NostrIdentitySecretEpoch {
  epoch: number;
  created_at: number;
  signed_by_pubkey: string;
  wrapped_secrets: Record<string, string>;
}

export interface NostrIdentityTombstone {
  pubkey: string;
  profile_id?: NostrIdentityId;
  removed_by_pubkey: string;
  removed_at: number;
  reason?: string;
}

export interface NostrIdentityRosterProjection {
  profile_id: NostrIdentityId;
  active_facets: Record<string, NostrIdentityFacet>;
  tombstones: Record<string, NostrIdentityTombstone>;
  secret_epochs: Record<string, NostrIdentitySecretEpoch>;
  accepted_op_ids: string[];
  rejected_op_ids: string[];
}

export interface NostrIdentityEncryptedDeviceLabelsPayload {
  schema: typeof NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_SCHEMA;
  profileId: NostrIdentityId;
  secretEpoch: number;
  labels: Record<string, string>;
  updatedAt: number;
}

export interface NostrIdentityEventDraft {
  kind: number;
  content: string;
  created_at: number;
  tags: string[][];
}

export interface BuildNostrIdentityRosterOpEventDraftOptions {
  signerPubkey: string;
  profileId: NostrIdentityId;
  op: NostrIdentityRosterOp;
  parents?: string[];
  actorSeq?: number;
  createdAt?: number;
  clientNonce?: string;
  encryptedDeviceLabels?: string;
}

export interface BuildNostrIdentityRosterOpEventOptions {
  signerSecretKey: Uint8Array;
  profileId: NostrIdentityId;
  op: NostrIdentityRosterOp;
  parents?: string[];
  actorSeq?: number;
  createdAt?: number;
  clientNonce?: string;
  encryptedDeviceLabels?: string;
}

export interface BuildNostrIdentityFacetAcceptanceEventDraftOptions {
  signerPubkey: string;
  profileId: NostrIdentityId;
  purposes: NostrIdentityKeyPurpose[];
  rosterOpId?: string;
  acceptedAt?: number;
  clientNonce?: string;
}

export interface BuildNostrIdentityFacetAcceptanceEventOptions {
  signerSecretKey: Uint8Array;
  profileId: NostrIdentityId;
  purposes: NostrIdentityKeyPurpose[];
  rosterOpId?: string;
  acceptedAt?: number;
  clientNonce?: string;
}

export const APP_KEY_ADMIN_CAPABILITIES: NostrIdentityCapabilities = {
  can_write_roots: true,
  can_admin_profile: true,
  can_receive_secret_wraps: true,
  can_decrypt_secret_epochs: true,
};

export const APP_KEY_WRITER_CAPABILITIES: NostrIdentityCapabilities = {
  can_write_roots: true,
  can_receive_secret_wraps: true,
  can_decrypt_secret_epochs: true,
};

function normalizeHexPubkey(value: string): string | null {
  const trimmed = value.trim();
  return /^[0-9a-f]{64}$/i.test(trimmed) ? trimmed.toLowerCase() : null;
}

export function isNostrIdentityId(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value.trim());
}

export function appKeyFacet(
  pubkey: string,
  options: {
    addedAt: number;
    capabilities?: NostrIdentityCapabilities;
  },
): NostrIdentityFacet {
  const normalized = normalizeHexPubkey(pubkey);
  if (!normalized) throw new Error('app key pubkey must be 64-char hex');
  return {
    pubkey: normalized,
    purposes: ['app_key'],
    capabilities: normalizeCapabilities(options.capabilities ?? APP_KEY_WRITER_CAPABILITIES),
    added_at: options.addedAt,
  };
}

export function createBootstrapRosterOp(options: {
  profileId: NostrIdentityId;
  adminAppKeyPubkey: string;
  createdAt: number;
  clientNonce: string;
}): NostrIdentityRosterOpContent {
  requireProfileId(options.profileId);
  const actorPubkey = requireHexPubkey(options.adminAppKeyPubkey, 'admin AppKey');
  return {
    schema: NOSTR_IDENTITY_ROSTER_SCHEMA,
    profile_id: options.profileId,
    actor_pubkey: actorPubkey,
    client_nonce: requireNonce(options.clientNonce),
    created_at: options.createdAt,
    op: {
      op: 'add_facet',
      facet: appKeyFacet(actorPubkey, {
        addedAt: options.createdAt,
        capabilities: APP_KEY_ADMIN_CAPABILITIES,
      }),
    },
  };
}

export function createAddAppKeyRosterOp(options: {
  profileId: NostrIdentityId;
  actorPubkey: string;
  devicePubkey: string;
  createdAt: number;
  clientNonce: string;
  parents?: string[];
  capabilities?: NostrIdentityCapabilities;
}): NostrIdentityRosterOpContent {
  requireProfileId(options.profileId);
  return {
    schema: NOSTR_IDENTITY_ROSTER_SCHEMA,
    profile_id: options.profileId,
    actor_pubkey: requireHexPubkey(options.actorPubkey, 'actor'),
    ...(options.parents?.length ? { parents: options.parents.slice() } : {}),
    client_nonce: requireNonce(options.clientNonce),
    created_at: options.createdAt,
    op: {
      op: 'add_facet',
      facet: appKeyFacet(options.devicePubkey, {
        addedAt: options.createdAt,
        capabilities: options.capabilities ?? APP_KEY_WRITER_CAPABILITIES,
      }),
    },
  };
}

export function canAdminProfile(projection: NostrIdentityRosterProjection, pubkey: string): boolean {
  const normalized = normalizeHexPubkey(pubkey);
  return Boolean(normalized && projection.active_facets[normalized]?.capabilities?.can_admin_profile);
}

export function normalizeCapabilities(capabilities: NostrIdentityCapabilities): NostrIdentityCapabilities {
  return {
    ...(capabilities.can_write_roots ? { can_write_roots: true } : {}),
    ...(capabilities.can_admin_profile ? { can_admin_profile: true } : {}),
    ...(capabilities.can_recover_app_keys ? { can_recover_app_keys: true } : {}),
    ...(capabilities.can_receive_secret_wraps ? { can_receive_secret_wraps: true } : {}),
    ...(capabilities.can_decrypt_secret_epochs ? { can_decrypt_secret_epochs: true } : {}),
  };
}

function requireProfileId(profileId: NostrIdentityId): void {
  if (!isNostrIdentityId(profileId)) throw new Error('NostrIdentity id must be a UUID');
}

function requireHexPubkey(value: string, label: string): string {
  const normalized = normalizeHexPubkey(value);
  if (!normalized) throw new Error(`${label} pubkey must be 64-char hex`);
  return normalized;
}

function requireNonce(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) throw new Error('client nonce is required');
  return trimmed;
}
