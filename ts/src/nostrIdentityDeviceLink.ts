import { sha256 } from '@noble/hashes/sha2.js';
import { finalizeEvent, generateSecretKey, getPublicKey, nip19, nip44, type Event, type Filter } from 'nostr-tools';
import { FACT_OP_KIND } from './factEvents';
import {
  buildIdentityLinkRequestEvent,
  parseIdentityLinkRequestEvent,
  normalizeHexPubkey,
} from './identityGraph';
import {
  APP_KEY_WRITER_CAPABILITIES,
  createAddAppKeyRosterOp,
  type NostrIdentityCapabilities,
  type NostrIdentityFacet,
  type NostrIdentityId,
  type NostrIdentityRosterProjection,
  type NostrIdentityRosterOpContent,
  type SignedNostrIdentityRosterOp,
} from './nostrIdentity';
import { parseNostrIdentityRosterOpEvent } from './nostrIdentityEvents';
import { requireValidSignature } from './nostrIdentityJson';
import { nostrIdentityRosterParentIds, projectNostrIdentityRoster } from './nostrIdentityProjection';

export const NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX = 'nostr-identity://device-link/';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_PREFIX = 'nostr-identity://device-approval/';
export const NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION = 1;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE = 'nostr_identity_device_approval_proof';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_EVENT_TYPE = 'nostr_identity_device_approval_request';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE = 'nostr_identity_device_approval_receipt';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA = 1;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_BYTE_LENGTH = 32;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH = 360;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX = 'nostr_identity_device_approval:';
export const NOSTR_IDENTITY_MANUAL_DEVICE_ADD_CLIENT_NONCE_PREFIX = 'nostr_identity_manual_device_add:';
export const KIND_NOSTR_IDENTITY_DEVICE_LINK_REQUEST = FACT_OP_KIND;

export interface NostrIdentityDeviceLinkInvite {
  profileId: NostrIdentityId;
  adminAppKeyPubkey: string;
  invitePubkey: string;
}

export interface AdminNostrIdentityDeviceLinkInvite extends NostrIdentityDeviceLinkInvite {
  inviteSecretKey: Uint8Array;
}

export interface NostrIdentityDeviceLinkRequest {
  profileId: NostrIdentityId;
  adminAppKeyPubkey: string;
  invitePubkey: string;
  deviceAppKeyPubkey: string;
  label?: string;
  requestedAt: number;
}

export interface SignedNostrIdentityDeviceLinkRequest {
  requestId: string;
  signerPubkey: string;
  request: NostrIdentityDeviceLinkRequest;
  event_json: string;
}

export interface NostrIdentityDeviceLinkRequestScope {
  profileId: NostrIdentityId;
  adminAppKeyPubkey: string;
  inviteSecretKey: Uint8Array;
  invitePubkey?: string;
}

export interface NostrIdentityDeviceApprovalRequest {
  requestPubkey: string;
  deviceAppKeyPubkey: string;
  requestSecret: string;
  deviceAppKeyProof: string;
  requestedAt: number;
  requestType?: string;
  resources?: NostrIdentityDeviceApprovalRequestedResource[];
  expiresAt?: number;
  profileId?: NostrIdentityId;
  adminAppKeyPubkey?: string;
  label?: string;
}

export interface NostrIdentityDeviceApprovalBootstrap {
  deviceAppKeyNpub: string;
  requestNpub: string;
  requestSecret: string;
}

export interface NostrIdentityDeviceApprovalRequestEventContent {
  requestSecretCommitment: string;
  deviceAppKeyProof: string;
  requestedAt: number;
  requestType?: string;
  resources?: NostrIdentityDeviceApprovalRequestedResource[];
  expiresAt?: number;
  profileId?: NostrIdentityId;
  adminAppKeyNpub?: string;
  label?: string;
}

export interface NostrIdentityDeviceApprovalRequestedResource {
  type: string;
  id: string;
  scopes?: string[];
}

export interface LocalNostrIdentityDeviceApprovalRequest extends NostrIdentityDeviceApprovalRequest {
  requestSecretKey: Uint8Array;
}

export interface NostrIdentityAppKeyApprovalCandidate {
  profileId: NostrIdentityId;
  appKeyPubkey: string;
  adminAppKeyPubkey: string;
  acceptedRosterOpCount: number;
  activeAppKeyCount: number;
  latestRosterOpCreatedAt?: number;
  profileRosterOps: SignedNostrIdentityRosterOp[];
}

export interface NostrIdentityDeviceApprovalReceipt {
  schema: typeof NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA;
  profileId: NostrIdentityId;
  requestPubkey: string;
  deviceAppKeyPubkey: string;
  approvedByPubkey: string;
  approvedAt: number;
  requestSecret: string;
  subjectPubkey?: string;
  rosterOpId?: string;
  signedRosterEvent?: string;
}

interface NostrIdentityDeviceLinkInvitePayload {
  v: number;
  profileId: string;
  adminAppKeyNpub: string;
  inviteNpub: string;
}

const DEVICE_LINK_INVITE_PAYLOAD_FIELDS = new Set([
  'v',
  'profileId',
  'adminAppKeyNpub',
  'inviteNpub',
]);
const DEVICE_APPROVAL_BOOTSTRAP_FIELDS = new Set([
  'deviceAppKeyNpub',
  'requestNpub',
  'requestSecret',
]);
const DEVICE_APPROVAL_REQUEST_EVENT_CONTENT_FIELDS = new Set([
  'deviceAppKeyProof',
  'requestedAt',
  'requestSecretCommitment',
  'requestType',
  'resources',
  'expiresAt',
  'profileId',
  'adminAppKeyNpub',
  'label',
]);
const DEVICE_APPROVAL_RESOURCE_FIELDS = new Set(['type', 'id', 'scopes']);
const DEVICE_APPROVAL_RELAY_RESOURCE_TYPE = 'nostr_relay';
const DEVICE_APPROVAL_RELAY_SCOPE = 'device_approval';
const DEVICE_APPROVAL_RELAY_LIMIT = 1;
const DEVICE_APPROVAL_RECEIPT_FIELDS = new Set([
  'schema',
  'profileId',
  'requestPubkey',
  'deviceAppKeyPubkey',
  'approvedByPubkey',
  'approvedAt',
  'requestSecret',
  'subjectPubkey',
  'rosterOpId',
  'signedRosterEvent',
]);
const DEVICE_APPROVAL_PROOF_TAGS = new Set([
  'type',
  'request_pubkey',
  'requested_at',
  'request_type',
  'requested_resources',
  'expires_at',
  'profile_id',
  'admin_pubkey',
  'label',
]);

export interface EncodeNostrIdentityDeviceLinkOptions {
  prefix?: string;
}

export interface ParseNostrIdentityDeviceLinkOptions {
  prefixes?: string[];
}

export function createNostrIdentityDeviceLinkInvite(options: {
  profileId: NostrIdentityId;
  adminAppKeyPubkey: string;
  inviteSecretKey?: Uint8Array;
}): AdminNostrIdentityDeviceLinkInvite {
  const inviteSecretKey = options.inviteSecretKey ?? generateSecretKey();
  return {
    profileId: requireProfileId(options.profileId),
    adminAppKeyPubkey: requirePubkey(options.adminAppKeyPubkey, 'admin AppKey'),
    inviteSecretKey,
    invitePubkey: getPublicKey(inviteSecretKey),
  };
}

export function encodeNostrIdentityDeviceLinkInvite(
  invite: NostrIdentityDeviceLinkInvite,
  options: EncodeNostrIdentityDeviceLinkOptions = {},
): string {
  const payload: NostrIdentityDeviceLinkInvitePayload = {
    v: NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION,
    profileId: requireProfileId(invite.profileId),
    adminAppKeyNpub: pubkeyToNpub(invite.adminAppKeyPubkey),
    inviteNpub: pubkeyToNpub(invite.invitePubkey),
  };
  return `${options.prefix ?? NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX}${base64UrlEncode(JSON.stringify(payload))}`;
}

export function parseNostrIdentityDeviceLinkInvite(
  input: string,
  options: ParseNostrIdentityDeviceLinkOptions = {},
): NostrIdentityDeviceLinkInvite | null {
  const payload = payloadFromPrefixedUrl(input, [
    ...(options.prefixes ?? []),
    NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX,
  ]);
  if (payload === null) return null;
  try {
    return normalizeDeviceLinkInvitePayload(JSON.parse(base64UrlDecode(payload)) as unknown);
  } catch {
    return null;
  }
}

export function isCompleteNostrIdentityDeviceLinkInviteInput(
  input: string,
  options: ParseNostrIdentityDeviceLinkOptions = {},
): boolean {
  const value = input.trim().replace(/^nostr:/i, '');
  if (!value || /\s/.test(value)) return false;
  return parseNostrIdentityDeviceLinkInvite(value, options) !== null;
}

export function createNostrIdentityDeviceLinkRequest(options: {
  invite: NostrIdentityDeviceLinkInvite;
  deviceAppKeyPubkey: string;
  requestedAt: number;
  label?: string;
}): NostrIdentityDeviceLinkRequest {
  return {
    profileId: requireProfileId(options.invite.profileId),
    adminAppKeyPubkey: requirePubkey(options.invite.adminAppKeyPubkey, 'admin AppKey'),
    invitePubkey: requirePubkey(options.invite.invitePubkey, 'invite'),
    deviceAppKeyPubkey: requirePubkey(options.deviceAppKeyPubkey, 'device AppKey'),
    requestedAt: requireInteger(options.requestedAt, 'requestedAt'),
    ...(options.label?.trim() ? { label: options.label.trim() } : {}),
  };
}

export function signNostrIdentityDeviceLinkRequestEvent(options: {
  signerSecretKey: Uint8Array;
  request: NostrIdentityDeviceLinkRequest;
  clientNonce?: string;
}): Event {
  const signerPubkey = getPublicKey(options.signerSecretKey);
  const deviceAppKeyPubkey = requirePubkey(options.request.deviceAppKeyPubkey, 'device AppKey');
  if (signerPubkey !== deviceAppKeyPubkey) {
    throw new Error('device link request must be signed by the requesting AppKey');
  }
  return buildIdentityLinkRequestEvent({
    signerSecretKey: options.signerSecretKey,
    identity: options.request.profileId,
    adminPubkey: options.request.adminAppKeyPubkey,
    invitePubkey: options.request.invitePubkey,
    requestedAt: options.request.requestedAt,
    ...(options.clientNonce !== undefined ? { clientNonce: options.clientNonce } : {}),
    ...(options.request.label !== undefined ? { label: options.request.label } : {}),
  });
}

export function parseNostrIdentityDeviceLinkRequestEvent(
  event: Event,
  scope: NostrIdentityDeviceLinkRequestScope,
): SignedNostrIdentityDeviceLinkRequest {
  requireValidSignature(event);
  const signed = parseIdentityLinkRequestEvent(event, {
    inviteSecretKey: scope.inviteSecretKey,
    identity: scope.profileId,
    adminPubkey: scope.adminAppKeyPubkey,
    ...(scope.invitePubkey !== undefined ? { invitePubkey: scope.invitePubkey } : {}),
  });
  const content = signed.content;
  return {
    requestId: signed.requestId,
    signerPubkey: signed.signerPubkey,
    request: {
      profileId: content.identity,
      adminAppKeyPubkey: content.adminPubkey,
      invitePubkey: content.invitePubkey,
      deviceAppKeyPubkey: content.joiningPubkey,
      requestedAt: content.requestedAt,
      ...(content.label !== undefined ? { label: content.label } : {}),
    },
    event_json: JSON.stringify(event),
  };
}

export function approveNostrIdentityDeviceLinkRequest(options: {
  request: NostrIdentityDeviceLinkRequest;
  rosterOps: SignedNostrIdentityRosterOp[];
  approvedByPubkey: string;
  approvedAt: number;
  clientNonce: string;
  capabilities?: NostrIdentityCapabilities;
}): NostrIdentityRosterOpContent {
  const approvedByPubkey = requirePubkey(options.approvedByPubkey, 'approving AppKey');
  if (approvedByPubkey !== requirePubkey(options.request.adminAppKeyPubkey, 'request admin AppKey')) {
    throw new Error('device link request must be approved by its invited admin AppKey');
  }
  return createAddAppKeyRosterOp({
    profileId: options.request.profileId,
    actorPubkey: approvedByPubkey,
    devicePubkey: options.request.deviceAppKeyPubkey,
    createdAt: options.approvedAt,
    clientNonce: requireNonEmpty(options.clientNonce, 'clientNonce'),
    parents: nostrIdentityRosterParentIds(options.rosterOps),
    capabilities: options.capabilities ?? APP_KEY_WRITER_CAPABILITIES,
  });
}

export function createNostrIdentityDeviceApprovalRequest(options: {
  deviceAppKeySecretKey: Uint8Array;
  requestSecretKey?: Uint8Array;
  requestSecret?: string;
  requestedAt: number;
  requestType?: string;
  resources?: NostrIdentityDeviceApprovalRequestedResource[];
  expiresAt?: number;
  profileId?: NostrIdentityId;
  adminAppKeyPubkey?: string;
  label?: string;
}): LocalNostrIdentityDeviceApprovalRequest {
  const deviceAppKeyPubkey = getPublicKey(options.deviceAppKeySecretKey);
  const requestSecretKey = options.requestSecretKey ?? generateSecretKey();
  const requestPubkey = getPublicKey(requestSecretKey);
  if (requestPubkey === deviceAppKeyPubkey) {
    throw new Error('device approval stable and ephemeral keys must be distinct');
  }
  const requestSecret = requireRequestSecret(options.requestSecret ?? randomDeviceApprovalSecret());
  const requestedAt = requireInteger(options.requestedAt, 'requestedAt');
  const requestType = normalizeOptionalDeviceApprovalString(options.requestType, 'requestType');
  const resources = normalizeDeviceApprovalResources(options.resources);
  const expiresAt = options.expiresAt !== undefined ? requireInteger(options.expiresAt, 'expiresAt') : undefined;
  const profileId = options.profileId !== undefined ? requireProfileId(options.profileId) : undefined;
  const adminAppKeyPubkey = options.adminAppKeyPubkey !== undefined
    ? requirePubkey(options.adminAppKeyPubkey, 'admin AppKey')
    : undefined;
  const label = options.label?.trim() ? options.label.trim() : undefined;
  const proof = buildNostrIdentityDeviceApprovalProofEvent({
    deviceAppKeySecretKey: options.deviceAppKeySecretKey,
    requestPubkey,
    requestedAt,
    ...(requestType !== undefined ? { requestType } : {}),
    ...(resources !== undefined ? { resources } : {}),
    ...(expiresAt !== undefined ? { expiresAt } : {}),
    ...(profileId !== undefined ? { profileId } : {}),
    ...(adminAppKeyPubkey !== undefined ? { adminAppKeyPubkey } : {}),
    ...(label !== undefined ? { label } : {}),
  });
  return {
    requestPubkey,
    requestSecretKey,
    deviceAppKeyPubkey,
    requestSecret,
    deviceAppKeyProof: JSON.stringify(proof),
    requestedAt,
    ...(requestType !== undefined ? { requestType } : {}),
    ...(resources !== undefined ? { resources } : {}),
    ...(expiresAt !== undefined ? { expiresAt } : {}),
    ...(profileId !== undefined ? { profileId } : {}),
    ...(adminAppKeyPubkey !== undefined ? { adminAppKeyPubkey } : {}),
    ...(label !== undefined ? { label } : {}),
  };
}

export function nostrIdentityDeviceApprovalRelayResource(
  relayUrl: string,
): NostrIdentityDeviceApprovalRequestedResource {
  return {
    type: DEVICE_APPROVAL_RELAY_RESOURCE_TYPE,
    id: normalizeDeviceApprovalRelayUrl(relayUrl),
    scopes: [DEVICE_APPROVAL_RELAY_SCOPE],
  };
}

export function nostrIdentityDeviceApprovalRequestRelays(
  request: Pick<NostrIdentityDeviceApprovalRequest, 'resources'>,
): string[] {
  const relays: string[] = [];
  for (const resource of request.resources ?? []) {
    if (
      resource.type !== DEVICE_APPROVAL_RELAY_RESOURCE_TYPE
      || !resource.scopes?.includes(DEVICE_APPROVAL_RELAY_SCOPE)
    ) {
      continue;
    }
    const relay = normalizeDeviceApprovalRelayUrl(resource.id);
    if (!relays.includes(relay)) relays.push(relay);
    if (relays.length > DEVICE_APPROVAL_RELAY_LIMIT) {
      throw new Error('device approval request must use at most one relay');
    }
  }
  return relays;
}

export function encodeNostrIdentityDeviceApprovalBootstrap(
  bootstrap: NostrIdentityDeviceApprovalBootstrap,
  options: EncodeNostrIdentityDeviceLinkOptions = {},
): string {
  const prefix = (options.prefix ?? NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_PREFIX).trim();
  if (!prefix) throw new Error('compact device approval prefix is empty');
  if (/[?#]/u.test(prefix)) throw new Error('compact device approval prefix must not contain query or fragment');
  const normalized = normalizeDeviceApprovalBootstrap(bootstrap);
  const uri = `${prefix}${base64UrlEncode(JSON.stringify(normalized))}`;
  if (uri.length > NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH) {
    throw new Error(`device approval bootstrap URI must not exceed ${NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH} characters`);
  }
  return uri;
}

export function parseNostrIdentityDeviceApprovalBootstrap(
  input: string,
  options: ParseNostrIdentityDeviceLinkOptions = {},
): NostrIdentityDeviceApprovalBootstrap | null {
  const payload = strictPayloadFromBootstrapUri(input, [
    ...(options.prefixes ?? []),
    NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_PREFIX,
  ]);
  if (payload === null) return null;
  try {
    return normalizeDeviceApprovalBootstrap(
      JSON.parse(base64UrlDecode(payload)) as unknown,
    );
  } catch {
    return null;
  }
}

export function nostrIdentityDeviceApprovalBootstrapHasPrefix(
  input: string,
  options: ParseNostrIdentityDeviceLinkOptions = {},
): boolean {
  return strictPayloadFromBootstrapUri(input, [
    ...(options.prefixes ?? []),
    NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_PREFIX,
  ]) !== null;
}

export function createNostrIdentityDeviceApprovalBootstrap(
  request: Pick<NostrIdentityDeviceApprovalRequest, 'deviceAppKeyPubkey' | 'requestPubkey' | 'requestSecret'>,
): NostrIdentityDeviceApprovalBootstrap {
  const deviceAppKeyPubkey = requirePubkey(request.deviceAppKeyPubkey, 'device AppKey');
  const requestPubkey = requirePubkey(request.requestPubkey, 'request');
  if (deviceAppKeyPubkey === requestPubkey) {
    throw new Error('device approval stable and ephemeral keys must be distinct');
  }
  return {
    deviceAppKeyNpub: pubkeyToNpub(deviceAppKeyPubkey),
    requestNpub: pubkeyToNpub(requestPubkey),
    requestSecret: requireRequestSecret(request.requestSecret),
  };
}

export function buildNostrIdentityDeviceApprovalRequestEvent(options: {
  requestSecretKey: Uint8Array;
  request: NostrIdentityDeviceApprovalRequest;
}): Event {
  const bootstrap = createNostrIdentityDeviceApprovalBootstrap(options.request);
  const requestPubkey = npubToPubkey(bootstrap.requestNpub)!;
  if (getPublicKey(options.requestSecretKey) !== requestPubkey) {
    throw new Error('device approval request event signer must be the ephemeral request key');
  }
  const content = deviceApprovalRequestEventContent(options.request, bootstrap.requestSecret);
  return finalizeEvent({
    kind: FACT_OP_KIND,
    created_at: content.requestedAt,
    tags: [
      ['type', NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_EVENT_TYPE],
      ['p', npubToPubkey(bootstrap.deviceAppKeyNpub)!],
    ],
    content: JSON.stringify(content),
  }, options.requestSecretKey);
}

export function parseNostrIdentityDeviceApprovalRequestEvent(
  event: Event,
  bootstrap: NostrIdentityDeviceApprovalBootstrap,
): NostrIdentityDeviceApprovalRequest {
  const normalizedBootstrap = normalizeDeviceApprovalBootstrap(bootstrap);
  const requestPubkey = npubToPubkey(normalizedBootstrap.requestNpub)!;
  const deviceAppKeyPubkey = npubToPubkey(normalizedBootstrap.deviceAppKeyNpub)!;

  requireFreshValidSignature(event);
  if (event.kind !== FACT_OP_KIND) throw new Error('device approval request event has invalid kind');
  if (event.pubkey !== requestPubkey) throw new Error('device approval request event signer mismatch');
  const expectedTags = [
    ['type', NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_EVENT_TYPE],
    ['p', deviceAppKeyPubkey],
  ];
  if (JSON.stringify(event.tags) !== JSON.stringify(expectedTags)) {
    throw new Error('device approval request event tags mismatch');
  }

  const content = normalizeDeviceApprovalRequestEventContent(
    JSON.parse(event.content) as unknown,
  );
  if (event.created_at !== content.requestedAt) {
    throw new Error('device approval request event requestedAt mismatch');
  }
  const expectedCommitment = requestSecretCommitment(normalizedBootstrap.requestSecret);
  if (content.requestSecretCommitment !== expectedCommitment) {
    throw new Error('device approval request secret commitment mismatch');
  }

  let adminAppKeyPubkey: string | undefined;
  if (content.adminAppKeyNpub !== undefined) {
    adminAppKeyPubkey = requireCanonicalNpub(content.adminAppKeyNpub, 'admin AppKey').pubkey;
  }
  const request: NostrIdentityDeviceApprovalRequest = {
    requestPubkey,
    deviceAppKeyPubkey,
    requestSecret: normalizedBootstrap.requestSecret,
    deviceAppKeyProof: content.deviceAppKeyProof,
    requestedAt: content.requestedAt,
    ...(content.requestType !== undefined ? { requestType: content.requestType } : {}),
    ...(content.resources !== undefined ? { resources: content.resources } : {}),
    ...(content.expiresAt !== undefined ? { expiresAt: content.expiresAt } : {}),
    ...(content.profileId !== undefined ? { profileId: content.profileId } : {}),
    ...(adminAppKeyPubkey !== undefined ? { adminAppKeyPubkey } : {}),
    ...(content.label !== undefined ? { label: content.label } : {}),
  };
  requireValidDeviceApprovalProof(request);
  return request;
}

export function approveNostrIdentityDeviceApprovalRequest(options: {
  request: NostrIdentityDeviceApprovalRequest;
  profileId: NostrIdentityId;
  rosterOps: SignedNostrIdentityRosterOp[];
  approvedByPubkey: string;
  approvedAt: number;
  clientNonce?: string;
  capabilities?: NostrIdentityCapabilities;
}): NostrIdentityRosterOpContent {
  const approvedByPubkey = requirePubkey(options.approvedByPubkey, 'approving AppKey');
  if (options.request.profileId !== undefined && options.request.profileId !== requireProfileId(options.profileId)) {
    throw new Error('device approval request profile mismatch');
  }
  if (
    options.request.adminAppKeyPubkey !== undefined
    && options.request.adminAppKeyPubkey !== approvedByPubkey
  ) {
    throw new Error('device approval request admin mismatch');
  }
  return createAddAppKeyRosterOp({
    profileId: options.profileId,
    actorPubkey: approvedByPubkey,
    devicePubkey: options.request.deviceAppKeyPubkey,
    createdAt: options.approvedAt,
    clientNonce: options.clientNonce ?? nostrIdentityDeviceApprovalClientNonce(),
    parents: nostrIdentityRosterParentIds(options.rosterOps),
    capabilities: options.capabilities ?? APP_KEY_WRITER_CAPABILITIES,
  });
}

export function buildNostrIdentityDeviceApprovalReceiptEvent(options: {
  signerSecretKey: Uint8Array;
  request: NostrIdentityDeviceApprovalRequest;
  profileId: NostrIdentityId;
  approvedAt: number;
  subjectPubkey?: string;
  rosterOpId?: string;
  rosterOpEvent?: Event | SignedNostrIdentityRosterOp;
}): Event {
  requireValidDeviceApprovalProof(options.request);
  const approvedByPubkey = getPublicKey(options.signerSecretKey);
  const profileId = requireProfileId(options.profileId);
  if (options.request.profileId !== undefined && requireProfileId(options.request.profileId) !== profileId) {
    throw new Error('device approval request profile mismatch');
  }
  if (
    options.request.adminAppKeyPubkey !== undefined
    && requirePubkey(options.request.adminAppKeyPubkey, 'request admin AppKey') !== approvedByPubkey
  ) {
    throw new Error('device approval request admin mismatch');
  }
  const signedRosterEvent = options.rosterOpEvent !== undefined
    ? signedRosterOpEventJson(options.rosterOpEvent)
    : undefined;
  const rosterOpId = options.rosterOpId ?? (
    signedRosterEvent !== undefined
      ? parseNostrIdentityRosterOpEvent(JSON.parse(signedRosterEvent) as Event).op_id
      : undefined
  );
  const receipt: NostrIdentityDeviceApprovalReceipt = {
    schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
    profileId,
    requestPubkey: requirePubkey(options.request.requestPubkey, 'request'),
    deviceAppKeyPubkey: requirePubkey(options.request.deviceAppKeyPubkey, 'device AppKey'),
    approvedByPubkey,
    approvedAt: requireInteger(options.approvedAt, 'approvedAt'),
    requestSecret: requireRequestSecret(options.request.requestSecret),
    ...(options.subjectPubkey !== undefined ? { subjectPubkey: requirePubkey(options.subjectPubkey, 'subject') } : {}),
    ...(rosterOpId !== undefined ? { rosterOpId: requireEventId(rosterOpId) } : {}),
    ...(signedRosterEvent !== undefined ? { signedRosterEvent } : {}),
  };
  if (receipt.signedRosterEvent !== undefined) {
    assertReceiptRosterOpMatches(receipt, parseNostrIdentityRosterOpEvent(
      JSON.parse(receipt.signedRosterEvent) as Event,
    ));
  }
  const conversationKey = nip44.v2.utils.getConversationKey(
    options.signerSecretKey,
    receipt.requestPubkey,
  );
  const encrypted = nip44.v2.encrypt(JSON.stringify(receipt), conversationKey);
  return finalizeEvent({
    kind: FACT_OP_KIND,
    content: encrypted,
    created_at: receipt.approvedAt,
    tags: [
      ['type', NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE],
      ['p', receipt.requestPubkey],
      ['i', receipt.profileId, 'subject'],
    ],
  }, options.signerSecretKey);
}

export function parseNostrIdentityDeviceApprovalReceiptEvent(
  event: Event,
  options: {
    requestSecretKey: Uint8Array;
    request?: NostrIdentityDeviceApprovalRequest;
    profileId?: NostrIdentityId;
    approvedByPubkey?: string;
  },
): NostrIdentityDeviceApprovalReceipt {
  requireFreshValidSignature(event);
  if (event.kind !== FACT_OP_KIND) throw new Error('device approval receipt has invalid kind');
  requireExactReceiptTags(event);
  requireExactEventTag(event, ['type', NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE], 'device approval receipt');
  const requestPubkey = getPublicKey(options.requestSecretKey);
  requireExactEventTag(event, ['p', requestPubkey], 'device approval receipt');
  const conversationKey = nip44.v2.utils.getConversationKey(
    options.requestSecretKey,
    requirePubkey(event.pubkey, 'approval receipt signer'),
  );
  const receipt = normalizeDeviceApprovalReceipt(
    JSON.parse(nip44.v2.decrypt(event.content, conversationKey)) as NostrIdentityDeviceApprovalReceipt,
  );
  if (receipt.requestPubkey !== requestPubkey) throw new Error('device approval receipt request mismatch');
  if (receipt.approvedAt !== event.created_at) throw new Error('device approval receipt approved_at mismatch');
  if (receipt.approvedByPubkey !== requirePubkey(event.pubkey, 'approval receipt signer')) {
    throw new Error('device approval receipt signer mismatch');
  }
  requireExactEventTag(event, ['i', receipt.profileId, 'subject'], 'device approval receipt');
  if (options.request !== undefined) {
    requireValidDeviceApprovalProof(options.request);
    if (requirePubkey(options.request.requestPubkey, 'request') !== requestPubkey) {
      throw new Error('device approval receipt request mismatch');
    }
    if (receipt.requestSecret !== requireRequestSecret(options.request.requestSecret)) {
      throw new Error('device approval receipt secret mismatch');
    }
    if (receipt.deviceAppKeyPubkey !== requirePubkey(options.request.deviceAppKeyPubkey, 'device AppKey')) {
      throw new Error('device approval receipt device mismatch');
    }
    if (
      options.request.profileId !== undefined
      && receipt.profileId !== requireProfileId(options.request.profileId)
    ) {
      throw new Error('device approval receipt profile mismatch');
    }
    if (
      options.request.adminAppKeyPubkey !== undefined
      && receipt.approvedByPubkey !== requirePubkey(options.request.adminAppKeyPubkey, 'request admin AppKey')
    ) {
      throw new Error('device approval receipt signer mismatch');
    }
  }
  if (options.profileId !== undefined && receipt.profileId !== requireProfileId(options.profileId)) {
    throw new Error('device approval receipt profile mismatch');
  }
  if (
    options.approvedByPubkey !== undefined
    && receipt.approvedByPubkey !== requirePubkey(options.approvedByPubkey, 'approval receipt signer')
  ) {
    throw new Error('device approval receipt signer mismatch');
  }
  return receipt;
}

export function parseNostrIdentityDeviceApprovalReceiptRosterOp(
  receipt: NostrIdentityDeviceApprovalReceipt,
): SignedNostrIdentityRosterOp {
  if (receipt.signedRosterEvent === undefined) {
    throw new Error('device approval receipt missing signed roster event');
  }
  const signed = parseNostrIdentityRosterOpEvent(JSON.parse(receipt.signedRosterEvent) as Event);
  assertReceiptRosterOpMatches(receipt, signed);
  return signed;
}

export function createNostrIdentityManualDeviceAddRosterOp(options: {
  profileId: NostrIdentityId;
  rosterOps: SignedNostrIdentityRosterOp[];
  approvedByPubkey: string;
  devicePubkey: string;
  addedAt: number;
  clientNonce?: string;
  capabilities?: NostrIdentityCapabilities;
}): NostrIdentityRosterOpContent {
  return createAddAppKeyRosterOp({
    profileId: options.profileId,
    actorPubkey: requirePubkey(options.approvedByPubkey, 'approving AppKey'),
    devicePubkey: requirePubkey(options.devicePubkey, 'device AppKey'),
    createdAt: options.addedAt,
    clientNonce: options.clientNonce ?? `${NOSTR_IDENTITY_MANUAL_DEVICE_ADD_CLIENT_NONCE_PREFIX}${randomDeviceApprovalSecret()}`,
    parents: nostrIdentityRosterParentIds(options.rosterOps),
    capabilities: options.capabilities ?? APP_KEY_WRITER_CAPABILITIES,
  });
}

export function nostrIdentityAppKeyApprovalCandidateFilters(appKeyPubkey: string): Filter[] {
  return [{
    kinds: [FACT_OP_KIND],
    '#p': [requirePubkey(appKeyPubkey, 'app key')],
  }];
}

export function nostrIdentityAppKeyApprovalCandidatesFromEvents(
  appKeyPubkey: string,
  events: Event[],
): NostrIdentityAppKeyApprovalCandidate[] {
  const appKey = requirePubkey(appKeyPubkey, 'app key');
  const candidateIds = new Set<NostrIdentityId>();
  const parsedOps: SignedNostrIdentityRosterOp[] = [];
  for (const event of events) {
    try {
      const op = parseNostrIdentityRosterOpEvent(event);
      parsedOps.push(op);
      if (op.signer_pubkey === appKey || rosterOpMentionedPubkeys(op).has(appKey)) {
        candidateIds.add(op.content.profile_id);
      }
    } catch {
      // Non-roster events are normal when projecting relay result batches.
    }
  }

  const candidates: NostrIdentityAppKeyApprovalCandidate[] = [];
  for (const profileId of candidateIds) {
    const profileRosterOps = parsedOps.filter((op) => op.content.profile_id === profileId);
    const projection = projectNostrIdentityRoster(profileId, profileRosterOps);
    const joiningFacet = projection.active_facets[appKey];
    if (!joiningFacet || !facetIsAppKey(joiningFacet) || !joiningFacet.capabilities?.can_write_roots) {
      continue;
    }
    const adminAppKeyPubkey = projectionAdminAppKeyPubkey(projection);
    if (!adminAppKeyPubkey) continue;
    const accepted = new Set(projection.accepted_op_ids);
    const latestRosterOpCreatedAt = profileRosterOps
      .filter((op) => accepted.has(op.op_id))
      .reduce<number | undefined>(
        (latest, op) => latest === undefined ? op.content.created_at : Math.max(latest, op.content.created_at),
        undefined,
      );
    candidates.push({
      profileId,
      appKeyPubkey: appKey,
      adminAppKeyPubkey,
      acceptedRosterOpCount: projection.accepted_op_ids.length,
      activeAppKeyCount: Object.values(projection.active_facets).filter(facetIsAppKey).length,
      ...(latestRosterOpCreatedAt !== undefined ? { latestRosterOpCreatedAt } : {}),
      profileRosterOps,
    });
  }
  return candidates.sort((left, right) => (
    (right.latestRosterOpCreatedAt ?? -1) - (left.latestRosterOpCreatedAt ?? -1)
      || right.acceptedRosterOpCount - left.acceptedRosterOpCount
      || right.activeAppKeyCount - left.activeAppKeyCount
      || left.profileId.localeCompare(right.profileId)
  ));
}

export function nostrIdentityDeviceApprovalClientNonce(randomValue: string = randomDeviceApprovalSecret()): string {
  return `${NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX}${requireRequestSecret(randomValue)}`;
}

export function nostrIdentityRosterOpMatchesDeviceApprovalReceipt(
  op: NostrIdentityRosterOpContent | SignedNostrIdentityRosterOp,
  receipt: NostrIdentityDeviceApprovalReceipt,
): boolean {
  const content = 'content' in op ? op.content : op;
  const rosterOp = content.op;
  return rosterOp.op === 'add_facet'
    && rosterOp.facet.pubkey === requirePubkey(receipt.deviceAppKeyPubkey, 'device AppKey');
}

export function pubkeyToNpub(pubkey: string): string {
  return nip19.npubEncode(requirePubkey(pubkey, 'pubkey'));
}

export function npubToPubkey(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const normalized = normalizeHexPubkey(trimmed);
  if (normalized) return normalized;
  return decodeNpubToPubkey(trimmed);
}

function normalizeDeviceLinkInvitePayload(
  value: unknown,
): NostrIdentityDeviceLinkInvite | null {
  const payload = requireExactObject(value, DEVICE_LINK_INVITE_PAYLOAD_FIELDS, 'device link invite');
  if (payload.v !== NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION) return null;
  const adminAppKeyPubkey = npubToPubkey(requireString(payload.adminAppKeyNpub, 'adminAppKeyNpub'));
  const invitePubkey = npubToPubkey(requireString(payload.inviteNpub, 'inviteNpub'));
  if (!adminAppKeyPubkey || !invitePubkey) return null;
  return {
    profileId: requireProfileId(requireString(payload.profileId, 'profileId')),
    adminAppKeyPubkey,
    invitePubkey,
  };
}

function normalizeDeviceApprovalBootstrap(
  value: unknown,
): NostrIdentityDeviceApprovalBootstrap {
  const bootstrap = requireExactObject(
    value,
    DEVICE_APPROVAL_BOOTSTRAP_FIELDS,
    'compact device approval request',
  );
  const deviceAppKey = requireCanonicalNpub(
    requireString(bootstrap.deviceAppKeyNpub, 'deviceAppKeyNpub'),
    'device AppKey',
  );
  const request = requireCanonicalNpub(
    requireString(bootstrap.requestNpub, 'requestNpub'),
    'request',
  );
  if (deviceAppKey.pubkey === request.pubkey) {
    throw new Error('device approval stable and ephemeral keys must be distinct');
  }
  return {
    deviceAppKeyNpub: deviceAppKey.npub,
    requestNpub: request.npub,
    requestSecret: requireRequestSecret(bootstrap.requestSecret),
  };
}

function deviceApprovalRequestEventContent(
  request: NostrIdentityDeviceApprovalRequest,
  requestSecret: string,
): NostrIdentityDeviceApprovalRequestEventContent {
  return {
    requestSecretCommitment: requestSecretCommitment(requestSecret),
    deviceAppKeyProof: requireValidDeviceApprovalProof(request),
    requestedAt: requireInteger(request.requestedAt, 'requestedAt'),
    ...(request.requestType !== undefined
      ? { requestType: normalizeOptionalDeviceApprovalString(request.requestType, 'requestType') }
      : {}),
    ...(request.resources !== undefined ? { resources: normalizeDeviceApprovalResources(request.resources) } : {}),
    ...(request.expiresAt !== undefined ? { expiresAt: requireInteger(request.expiresAt, 'expiresAt') } : {}),
    ...(request.profileId !== undefined ? { profileId: requireProfileId(request.profileId) } : {}),
    ...(request.adminAppKeyPubkey !== undefined
      ? { adminAppKeyNpub: pubkeyToNpub(request.adminAppKeyPubkey) }
      : {}),
    ...(request.label?.trim() ? { label: request.label.trim() } : {}),
  };
}

function normalizeDeviceApprovalRequestEventContent(
  value: unknown,
): NostrIdentityDeviceApprovalRequestEventContent {
  const content = requireExactObject(
    value,
    DEVICE_APPROVAL_REQUEST_EVENT_CONTENT_FIELDS,
    'device approval request event content',
  );
  let adminAppKeyNpub: string | undefined;
  if (content.adminAppKeyNpub !== undefined) {
    adminAppKeyNpub = requireCanonicalNpub(
      requireString(content.adminAppKeyNpub, 'adminAppKeyNpub'),
      'admin AppKey',
    ).npub;
  }
  const requestSecretCommitment = requireString(
    content.requestSecretCommitment,
    'requestSecretCommitment',
  );
  if (!/^[0-9a-f]{64}$/u.test(requestSecretCommitment)) {
    throw new Error('requestSecretCommitment must be lowercase SHA-256 hex');
  }
  const label = normalizeOptionalDeviceApprovalLabel(content.label);
  return {
    requestSecretCommitment,
    deviceAppKeyProof: requireNonEmpty(content.deviceAppKeyProof, 'deviceAppKeyProof'),
    requestedAt: requireInteger(content.requestedAt, 'requestedAt'),
    ...(content.requestType !== undefined
      ? { requestType: normalizeOptionalDeviceApprovalString(content.requestType, 'requestType') }
      : {}),
    ...(content.resources !== undefined ? { resources: normalizeDeviceApprovalResources(content.resources) } : {}),
    ...(content.expiresAt !== undefined ? { expiresAt: requireInteger(content.expiresAt, 'expiresAt') } : {}),
    ...(content.profileId !== undefined
      ? { profileId: requireProfileId(requireString(content.profileId, 'profileId')) }
      : {}),
    ...(adminAppKeyNpub !== undefined ? { adminAppKeyNpub } : {}),
    ...(label !== undefined ? { label } : {}),
  };
}

function normalizeDeviceApprovalReceipt(
  value: unknown,
): NostrIdentityDeviceApprovalReceipt {
  const receipt = requireExactObject(value, DEVICE_APPROVAL_RECEIPT_FIELDS, 'device approval receipt');
  if (receipt.schema !== NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA) {
    throw new Error(`unsupported device approval receipt schema ${receipt.schema}`);
  }
  return {
    schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
    profileId: requireProfileId(requireString(receipt.profileId, 'profileId')),
    requestPubkey: requirePubkey(requireString(receipt.requestPubkey, 'requestPubkey'), 'request'),
    deviceAppKeyPubkey: requirePubkey(
      requireString(receipt.deviceAppKeyPubkey, 'deviceAppKeyPubkey'),
      'device AppKey',
    ),
    approvedByPubkey: requirePubkey(
      requireString(receipt.approvedByPubkey, 'approvedByPubkey'),
      'approving AppKey',
    ),
    approvedAt: requireInteger(receipt.approvedAt, 'approvedAt'),
    requestSecret: requireRequestSecret(requireString(receipt.requestSecret, 'requestSecret')),
    ...(receipt.subjectPubkey != null
      ? { subjectPubkey: requirePubkey(requireString(receipt.subjectPubkey, 'subjectPubkey'), 'subject') }
      : {}),
    ...(receipt.rosterOpId != null
      ? { rosterOpId: requireEventId(requireString(receipt.rosterOpId, 'rosterOpId')) }
      : {}),
    ...(receipt.signedRosterEvent != null
      ? { signedRosterEvent: requireNonEmpty(receipt.signedRosterEvent, 'signedRosterEvent') }
      : {}),
  };
}

function signedRosterOpEventJson(rosterOpEvent: Event | SignedNostrIdentityRosterOp): string {
  return 'event_json' in rosterOpEvent
    ? rosterOpEvent.event_json
    : JSON.stringify(rosterOpEvent);
}

function assertReceiptRosterOpMatches(
  receipt: NostrIdentityDeviceApprovalReceipt,
  signed: SignedNostrIdentityRosterOp,
): void {
  if (receipt.rosterOpId !== undefined && signed.op_id !== receipt.rosterOpId) {
    throw new Error('device approval receipt roster op id mismatch');
  }
  if (signed.content.profile_id !== receipt.profileId) {
    throw new Error('device approval receipt roster profile mismatch');
  }
  if (signed.content.actor_pubkey !== receipt.approvedByPubkey) {
    throw new Error('device approval receipt roster signer mismatch');
  }
  if (!nostrIdentityRosterOpMatchesDeviceApprovalReceipt(signed, receipt)) {
    throw new Error('device approval receipt roster device mismatch');
  }
}

function buildNostrIdentityDeviceApprovalProofEvent(options: {
  deviceAppKeySecretKey: Uint8Array;
  requestPubkey: string;
  requestedAt: number;
  requestType?: string;
  resources?: NostrIdentityDeviceApprovalRequestedResource[];
  expiresAt?: number;
  profileId?: NostrIdentityId;
  adminAppKeyPubkey?: string;
  label?: string;
}): Event {
  const requestedAt = requireInteger(options.requestedAt, 'requestedAt');
  const requestType = normalizeOptionalDeviceApprovalString(options.requestType, 'requestType');
  const resources = normalizeDeviceApprovalResources(options.resources);
  const expiresAt = options.expiresAt !== undefined ? requireInteger(options.expiresAt, 'expiresAt') : undefined;
  return finalizeEvent({
    kind: FACT_OP_KIND,
    content: '',
    created_at: requestedAt,
    tags: [
      ['type', NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE],
      ['request_pubkey', requirePubkey(options.requestPubkey, 'request')],
      ['requested_at', String(requestedAt)],
      ...(requestType !== undefined ? [['request_type', requestType]] : []),
      ...(resources !== undefined ? [['requested_resources', JSON.stringify(resources)]] : []),
      ...(expiresAt !== undefined ? [['expires_at', String(expiresAt)]] : []),
      ...(options.profileId !== undefined ? [['profile_id', requireProfileId(options.profileId)]] : []),
      ...(options.adminAppKeyPubkey !== undefined ? [['admin_pubkey', requirePubkey(options.adminAppKeyPubkey, 'admin AppKey')]] : []),
      ...(options.label?.trim() ? [['label', options.label.trim()]] : []),
    ],
  }, options.deviceAppKeySecretKey);
}

function requireValidDeviceApprovalProof(request: NostrIdentityDeviceApprovalRequest): string {
  const raw = request.deviceAppKeyProof.trim();
  if (!raw) throw new Error('device approval proof is required');
  const event = JSON.parse(raw) as Event;
  requireValidSignature(event);
  if (event.kind !== FACT_OP_KIND) throw new Error('device approval proof has invalid kind');
  if (event.content !== '') throw new Error('device approval proof content must be empty');
  requireExactProofTags(event);
  if (event.pubkey !== requirePubkey(request.deviceAppKeyPubkey, 'device AppKey')) {
    throw new Error('device approval proof signer mismatch');
  }
  if (event.created_at !== requireInteger(request.requestedAt, 'requestedAt')) {
    throw new Error('device approval proof requested_at mismatch');
  }
  requireProofTag(event, 'type', NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE);
  requireProofTag(event, 'request_pubkey', requirePubkey(request.requestPubkey, 'request'));
  requireProofTag(event, 'requested_at', String(request.requestedAt));
  requireOptionalProofTag(event, 'request_type', request.requestType);
  requireOptionalProofTag(
    event,
    'requested_resources',
    request.resources !== undefined ? JSON.stringify(normalizeDeviceApprovalResources(request.resources)) : undefined,
  );
  requireOptionalProofTag(event, 'expires_at', request.expiresAt !== undefined ? String(request.expiresAt) : undefined);
  requireOptionalProofTag(event, 'profile_id', request.profileId);
  requireOptionalProofTag(event, 'admin_pubkey', request.adminAppKeyPubkey);
  requireOptionalProofTag(event, 'label', request.label);
  return raw;
}

function normalizeOptionalDeviceApprovalString(value: unknown, label: string): string | undefined {
  if (value === undefined) return undefined;
  const normalized = requireString(value, label).trim();
  if (!normalized) return undefined;
  if (normalized.length > 128) throw new Error(`${label} is too long`);
  return normalized;
}

function normalizeOptionalDeviceApprovalLabel(value: unknown): string | undefined {
  if (value == null) return undefined;
  const normalized = requireString(value, 'label').trim();
  return normalized || undefined;
}

function normalizeDeviceApprovalResources(
  resources: unknown,
): NostrIdentityDeviceApprovalRequestedResource[] | undefined {
  if (resources === undefined) return undefined;
  if (!Array.isArray(resources)) throw new Error('resources must be an array');
  const normalized = resources.map((resource, index) => {
    if (resource === null || typeof resource !== 'object' || Array.isArray(resource)) {
      throw new Error(`resources[${index}] must be an object`);
    }
    const record = requireExactObject(
      resource,
      DEVICE_APPROVAL_RESOURCE_FIELDS,
      `resources[${index}]`,
    );
    const type = normalizeRequiredDeviceApprovalString(record.type, `resources[${index}].type`);
    const id = normalizeRequiredDeviceApprovalString(record.id, `resources[${index}].id`);
    const scopes = normalizeDeviceApprovalScopes(record.scopes, `resources[${index}].scopes`);
    return {
      type,
      id,
      ...(scopes !== undefined ? { scopes } : {}),
    };
  });
  return normalized.length ? normalized : undefined;
}

function normalizeRequiredDeviceApprovalString(value: unknown, label: string): string {
  const normalized = requireString(value, label).trim();
  if (!normalized) throw new Error(`${label} is required`);
  if (normalized.length > 256) throw new Error(`${label} is too long`);
  return normalized;
}

function normalizeDeviceApprovalScopes(value: unknown, label: string): string[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) throw new Error(`${label} must be an array`);
  const scopes = value
    .map((scope) => normalizeRequiredDeviceApprovalString(scope, label))
    .filter((scope, index, array) => array.indexOf(scope) === index);
  return scopes.length ? scopes : undefined;
}

function normalizeDeviceApprovalRelayUrl(value: unknown): string {
  const relayUrl = requireString(value, 'device approval relay URL').trim();
  const schemeSeparator = relayUrl.indexOf('://');
  const scheme = schemeSeparator >= 0 ? relayUrl.slice(0, schemeSeparator).toLowerCase() : '';
  const authority = schemeSeparator >= 0 ? relayUrl.slice(schemeSeparator + 3) : '';
  if ((scheme !== 'ws' && scheme !== 'wss') || !authority || authority.startsWith('/')) {
    throw new Error('device approval relay URL must use ws or wss');
  }
  const authorityEnd = authority.search(/[/?#]/u);
  const authorityValue = authorityEnd >= 0 ? authority.slice(0, authorityEnd) : authority;
  if (authorityValue.includes('@')) {
    throw new Error('device approval relay URL must not contain credentials');
  }

  let parsed: URL;
  try {
    parsed = new URL(relayUrl);
  } catch {
    throw new Error('device approval relay URL is invalid');
  }
  if (!parsed.hostname) throw new Error('device approval relay URL is invalid');
  if (parsed.username || parsed.password) {
    throw new Error('device approval relay URL must not contain credentials');
  }

  const path = parsed.pathname.replace(/\/{2,}/gu, '/').replace(/\/+$/u, '');
  parsed.searchParams.sort();
  return `${parsed.protocol}//${parsed.host}${path}${parsed.search}`;
}

function requireProofTag(event: Event, name: string, expected: string): void {
  if (eventTagValue(event, name) !== expected) {
    throw new Error(`device approval proof ${name} mismatch`);
  }
}

function requireOptionalProofTag(event: Event, name: string, expected: string | undefined): void {
  const actual = eventTagValue(event, name);
  if (expected === undefined) {
    if (actual !== undefined) throw new Error(`device approval proof unexpected ${name}`);
    return;
  }
  if (actual !== expected) throw new Error(`device approval proof ${name} mismatch`);
}

function requireExactProofTags(event: Event): void {
  const seen = new Set<string>();
  for (const tag of event.tags) {
    if (tag.length !== 2 || typeof tag[0] !== 'string' || typeof tag[1] !== 'string') {
      throw new Error('device approval proof tag is malformed');
    }
    const name = tag[0];
    if (!DEVICE_APPROVAL_PROOF_TAGS.has(name)) {
      throw new Error(`device approval proof has unknown tag ${name}`);
    }
    if (seen.has(name)) throw new Error(`device approval proof has duplicate tag ${name}`);
    seen.add(name);
  }
}

function requireFreshValidSignature(event: Event): void {
  requireValidSignature({
    id: event.id,
    pubkey: event.pubkey,
    created_at: event.created_at,
    kind: event.kind,
    tags: event.tags.map((tag) => [...tag]),
    content: event.content,
    sig: event.sig,
  });
}

function requireExactReceiptTags(event: Event): void {
  const expected = new Set(['type', 'p', 'i']);
  const seen = new Set<string>();
  for (const tag of event.tags) {
    const name = tag[0];
    if (!name || !expected.has(name)) {
      throw new Error(`device approval receipt has unknown tag ${name ?? ''}`);
    }
    if (seen.has(name)) throw new Error(`device approval receipt has duplicate tag ${name}`);
    seen.add(name);
  }
  if (seen.size !== expected.size) throw new Error('device approval receipt is missing required tags');
}

function requireExactEventTag(event: Event, expected: string[], label: string): void {
  if (!event.tags.some((tag) => tag.length === expected.length && tag.every((value, index) => value === expected[index]))) {
    throw new Error(`${label} ${expected[0]} mismatch`);
  }
}

function eventTagValue(event: Event, name: string): string | undefined {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

function requireExactObject(
  value: unknown,
  allowedFields: ReadonlySet<string>,
  label: string,
): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const record = value as Record<string, unknown>;
  const unknownField = Object.keys(record).find((field) => !allowedFields.has(field));
  if (unknownField !== undefined) throw new Error(`${label} has unknown field ${unknownField}`);
  return record;
}

function requireString(value: unknown, label: string): string {
  if (typeof value !== 'string') throw new Error(`${label} must be a string`);
  return value;
}

function payloadFromPrefixedUrl(input: string, prefixes: string[]): string | null {
  const value = input.trim().replace(/^nostr:/i, '');
  if (!value) return null;
  const lower = value.toLowerCase();
  const prefix = prefixes
    .filter((candidate) => candidate.trim())
    .find((candidate) => lower.startsWith(candidate.toLowerCase()));
  if (!prefix) return null;
  const payload = value.slice(prefix.length).split(/[?#]/, 1)[0].trim();
  return payload || null;
}

function strictPayloadFromBootstrapUri(input: string, prefixes: string[]): string | null {
  if (!input || input !== input.trim() || /^nostr:/iu.test(input) || /[?#]/u.test(input)) {
    return null;
  }
  for (const rawPrefix of prefixes) {
    const prefix = rawPrefix.trim();
    if (!prefix || !input.startsWith(prefix)) continue;
    const payload = input.slice(prefix.length);
    return payload && /^[A-Za-z0-9_-]+$/u.test(payload) ? payload : null;
  }
  return null;
}

function rosterOpMentionedPubkeys(op: SignedNostrIdentityRosterOp): Set<string> {
  switch (op.content.op.op) {
    case 'add_facet':
      return new Set([op.content.op.facet.pubkey]);
    case 'tombstone_facet':
    case 'set_capabilities':
      return new Set([op.content.op.pubkey]);
    case 'rotate_secret_epoch':
    case 'repair_secret_wraps':
      return new Set(Object.keys(op.content.op.wrapped_secrets ?? {}));
  }
}

function projectionAdminAppKeyPubkey(projection: NostrIdentityRosterProjection): string | undefined {
  return Object.values(projection.active_facets)
    .find((facet) => facetIsAppKey(facet) && Boolean(facet.capabilities?.can_admin_profile))
    ?.pubkey;
}

function facetIsAppKey(facet: NostrIdentityFacet): boolean {
  return (facet.purposes ?? []).includes('app_key');
}

function randomDeviceApprovalSecret(): string {
  return base64UrlEncodeBytes(generateSecretKey());
}

function base64UrlEncode(value: string): string {
  return base64UrlEncodeBytes(new TextEncoder().encode(value));
}

function base64UrlEncodeBytes(bytes: Uint8Array): string {
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(bytes).toString('base64url');
  }
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/u, '');
}

function base64UrlDecode(value: string): string {
  return new TextDecoder('utf-8', { fatal: true }).decode(base64UrlDecodeBytes(value));
}

function base64UrlDecodeBytes(value: string): Uint8Array {
  const normalized = value.trim();
  if (looksLikePlaceholder(normalized)) throw new Error('device link payload is a placeholder');
  if (normalized.length % 4 === 1 || !/^[A-Za-z0-9_-]+$/u.test(normalized)) {
    throw new Error('device link payload is not base64url');
  }
  if (typeof Buffer !== 'undefined') {
    const bytes = Buffer.from(normalized, 'base64url');
    if (bytes.toString('base64url') !== normalized) {
      throw new Error('device link payload has invalid base64url padding bits');
    }
    return bytes;
  }
  let base64 = normalized.replace(/-/g, '+').replace(/_/g, '/');
  base64 += '='.repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(base64);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  if (base64UrlEncodeBytes(bytes) !== normalized) {
    throw new Error('device link payload has invalid base64url padding bits');
  }
  return bytes;
}

function requireProfileId(profileId: NostrIdentityId): NostrIdentityId {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(profileId.trim())) {
    throw new Error('NostrIdentity id must be a UUID');
  }
  return profileId.trim().toLowerCase();
}

function requirePubkey(value: string, label: string): string {
  const normalized = normalizeHexPubkey(value);
  if (normalized) return normalized;
  const decoded = decodeNpubToPubkey(value);
  if (decoded) return decoded;
  throw new Error(`${label} pubkey must be npub or 64-char hex`);
}

function requireCanonicalNpub(value: string, label: string): { npub: string; pubkey: string } {
  const trimmed = value.trim();
  const pubkey = decodeNpubToPubkey(trimmed);
  if (!pubkey) throw new Error(`${label} must be a valid npub`);
  const npub = pubkeyToNpub(pubkey);
  if (trimmed !== npub) throw new Error(`${label} must be a canonical lowercase npub`);
  return { npub, pubkey };
}

function decodeNpubToPubkey(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed.toLowerCase().startsWith('npub1')) return null;
  try {
    const decoded = nip19.decode(trimmed);
    return decoded.type === 'npub' && typeof decoded.data === 'string'
      ? normalizeHexPubkey(decoded.data)
      : null;
  } catch {
    return null;
  }
}

function requireRequestSecret(value: unknown): string {
  const trimmed = requireString(value, 'requestSecret').trim();
  let bytes: Uint8Array;
  try {
    bytes = base64UrlDecodeBytes(trimmed);
  } catch {
    throw new Error('device approval request secret must be canonical unpadded base64url');
  }
  if (bytes.length !== NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_BYTE_LENGTH) {
    throw new Error('device approval request secret must encode exactly 32 bytes');
  }
  return trimmed;
}

function requestSecretCommitment(requestSecret: string): string {
  const domain = new TextEncoder().encode('nostr_identity_device_approval_request_secret_v1\0');
  const secret = base64UrlDecodeBytes(requireRequestSecret(requestSecret));
  const input = new Uint8Array(domain.length + secret.length);
  input.set(domain);
  input.set(secret, domain.length);
  return Array.from(sha256(input))
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
}

function requireEventId(value: string): string {
  const trimmed = value.trim();
  if (!/^[0-9a-f]{64}$/i.test(trimmed)) throw new Error('roster op id must be a 64-char hex event id');
  return trimmed.toLowerCase();
}

function requireInteger(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value)) {
    throw new Error(`${label} must be an integer`);
  }
  return value;
}

function requireNonEmpty(value: unknown, label: string): string {
  const trimmed = requireString(value, label).trim();
  if (!trimmed) throw new Error(`${label} is required`);
  return trimmed;
}

function looksLikePlaceholder(value: string): boolean {
  const trimmed = value.trim();
  return trimmed.includes('...') || matchesPlaceholder(trimmed);
}

function matchesPlaceholder(value: string): boolean {
  return value === '<code>' || value === '<payload>' || value === '<invite>';
}
