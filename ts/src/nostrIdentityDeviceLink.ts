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
export const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX = 'nostr-identity://device-approval/';
export const NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX = 'nostr-identity://device-approval';
export const NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION = 1;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION = 1;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE = 'nostr_identity_device_approval_proof';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE = 'nostr_identity_device_approval_receipt';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA = 1;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_MIN_LENGTH = 32;
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

export interface NostrIdentityCompactDeviceApprovalRequest {
  deviceAppKeyPubkey: string;
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

interface NostrIdentityDeviceApprovalRequestPayload {
  v: number;
  requestNpub: string;
  deviceAppKeyNpub: string;
  requestSecret: string;
  deviceAppKeyProof: string;
  requestedAt: number;
  requestType?: string;
  resources?: NostrIdentityDeviceApprovalRequestedResource[];
  expiresAt?: number;
  profileId?: string;
  adminAppKeyNpub?: string;
  label?: string;
}

const DEVICE_LINK_INVITE_PAYLOAD_FIELDS = new Set([
  'v',
  'profileId',
  'adminAppKeyNpub',
  'inviteNpub',
]);
const DEVICE_APPROVAL_REQUEST_PAYLOAD_FIELDS = new Set([
  'v',
  'requestNpub',
  'deviceAppKeyNpub',
  'requestSecret',
  'deviceAppKeyProof',
  'requestedAt',
  'requestType',
  'resources',
  'expiresAt',
  'profileId',
  'adminAppKeyNpub',
  'label',
]);
const DEVICE_APPROVAL_RESOURCE_FIELDS = new Set(['type', 'id', 'scopes']);
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
    requestSecret,
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

export function encodeNostrIdentityDeviceApprovalRequest(
  request: NostrIdentityDeviceApprovalRequest,
  options: EncodeNostrIdentityDeviceLinkOptions = {},
): string {
  const payload: NostrIdentityDeviceApprovalRequestPayload = {
    v: NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION,
    requestNpub: pubkeyToNpub(request.requestPubkey),
    deviceAppKeyNpub: pubkeyToNpub(request.deviceAppKeyPubkey),
    requestSecret: requireRequestSecret(request.requestSecret),
    deviceAppKeyProof: requireValidDeviceApprovalProof(request),
    requestedAt: requireInteger(request.requestedAt, 'requestedAt'),
    ...(request.requestType !== undefined
      ? { requestType: normalizeOptionalDeviceApprovalString(request.requestType, 'requestType') }
      : {}),
    ...(request.resources !== undefined ? { resources: normalizeDeviceApprovalResources(request.resources) } : {}),
    ...(request.expiresAt !== undefined ? { expiresAt: requireInteger(request.expiresAt, 'expiresAt') } : {}),
    ...(request.profileId !== undefined ? { profileId: requireProfileId(request.profileId) } : {}),
    ...(request.adminAppKeyPubkey !== undefined ? { adminAppKeyNpub: pubkeyToNpub(request.adminAppKeyPubkey) } : {}),
    ...(request.label?.trim() ? { label: request.label.trim() } : {}),
  };
  return `${options.prefix ?? NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX}${base64UrlEncode(JSON.stringify(payload))}`;
}

export function encodeCompactNostrIdentityDeviceApprovalRequest(
  deviceAppKeyPubkey: string,
  options: EncodeNostrIdentityDeviceLinkOptions = {},
): string {
  const prefix = (options.prefix ?? NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX).trim().replace(/\?+$/u, '');
  if (!prefix) throw new Error('compact device approval prefix is empty');
  return `${prefix}?app_key=${requirePubkey(deviceAppKeyPubkey, 'device AppKey')}`;
}

export function parseCompactNostrIdentityDeviceApprovalRequest(
  input: string,
  options: ParseNostrIdentityDeviceLinkOptions = {},
): NostrIdentityCompactDeviceApprovalRequest | null {
  const query = queryFromPrefixedUrl(input, [
    ...(options.prefixes ?? []),
    NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX,
  ]);
  if (query === null) return null;
  const appKey = queryValue(query, 'app_key') ?? queryValue(query, 'device');
  if (!appKey) throw new Error('device request is missing app_key');
  return { deviceAppKeyPubkey: requirePubkey(appKey, 'device AppKey') };
}

export function compactNostrIdentityDeviceApprovalRequestHasPrefix(
  input: string,
  options: ParseNostrIdentityDeviceLinkOptions = {},
): boolean {
  return queryFromPrefixedUrl(input, [
    ...(options.prefixes ?? []),
    NOSTR_IDENTITY_COMPACT_DEVICE_APPROVAL_REQUEST_PREFIX,
  ]) !== null;
}

export function parseNostrIdentityDeviceApprovalRequest(
  input: string,
  options: ParseNostrIdentityDeviceLinkOptions = {},
): NostrIdentityDeviceApprovalRequest | null {
  const payload = payloadFromPrefixedUrl(input, [
    ...(options.prefixes ?? []),
    NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX,
  ]);
  if (payload === null) return null;
  try {
    return normalizeDeviceApprovalRequestPayload(
      JSON.parse(base64UrlDecode(payload)) as unknown,
    );
  } catch {
    return null;
  }
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

function normalizeDeviceApprovalRequestPayload(
  value: unknown,
): NostrIdentityDeviceApprovalRequest | null {
  const payload = requireExactObject(value, DEVICE_APPROVAL_REQUEST_PAYLOAD_FIELDS, 'device approval request');
  if (payload.v !== NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION) return null;
  const requestPubkey = npubToPubkey(requireString(payload.requestNpub, 'requestNpub'));
  const deviceAppKeyPubkey = npubToPubkey(requireString(payload.deviceAppKeyNpub, 'deviceAppKeyNpub'));
  if (!requestPubkey || !deviceAppKeyPubkey) return null;
  let adminAppKeyPubkey: string | undefined;
  if (payload.adminAppKeyNpub != null) {
    const parsedAdmin = npubToPubkey(requireString(payload.adminAppKeyNpub, 'adminAppKeyNpub'));
    if (!parsedAdmin) return null;
    adminAppKeyPubkey = parsedAdmin;
  }
  const label = normalizeOptionalDeviceApprovalLabel(payload.label);
  const request = {
    requestPubkey,
    deviceAppKeyPubkey,
    requestSecret: requireRequestSecret(requireString(payload.requestSecret, 'requestSecret')),
    deviceAppKeyProof: requireString(payload.deviceAppKeyProof, 'deviceAppKeyProof'),
    requestedAt: requireInteger(payload.requestedAt, 'requestedAt'),
    ...(payload.requestType != null
      ? { requestType: normalizeOptionalDeviceApprovalString(payload.requestType, 'requestType') }
      : {}),
    ...(payload.resources !== undefined ? { resources: normalizeDeviceApprovalResources(payload.resources) } : {}),
    ...(payload.expiresAt != null ? { expiresAt: requireInteger(payload.expiresAt, 'expiresAt') } : {}),
    ...(payload.profileId != null
      ? { profileId: requireProfileId(requireString(payload.profileId, 'profileId')) }
      : {}),
    ...(adminAppKeyPubkey !== undefined ? { adminAppKeyPubkey } : {}),
    ...(label !== undefined ? { label } : {}),
  };
  requireValidDeviceApprovalProof(request);
  return request;
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
  requestSecret: string;
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

function queryFromPrefixedUrl(input: string, prefixes: string[]): string | null {
  const value = input.trim().replace(/^nostr:/i, '');
  if (!value) return null;
  const lower = value.toLowerCase();
  for (const rawPrefix of prefixes) {
    const prefix = rawPrefix.trim();
    if (!prefix || !lower.startsWith(prefix.toLowerCase())) continue;
    const rest = value.slice(prefix.length);
    const query = prefix.endsWith('?') ? rest : rest.startsWith('?') ? rest.slice(1) : null;
    const normalized = query?.split('#', 1)[0].trim() ?? '';
    if (normalized) return normalized;
  }
  return null;
}

function queryValue(query: string, name: string): string | null {
  for (const part of query.split('&')) {
    const [key, value = ''] = part.split('=', 2);
    if (key.toLowerCase() === name.toLowerCase()) return percentDecode(value);
  }
  return null;
}

function percentDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
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
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  }
  let base64 = normalized.replace(/-/g, '+').replace(/_/g, '/');
  base64 += '='.repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(base64);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  if (base64UrlEncodeBytes(bytes) !== normalized) {
    throw new Error('device link payload has invalid base64url padding bits');
  }
  return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
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
  if (
    trimmed.length < NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_MIN_LENGTH
    || !/^[A-Za-z0-9_-]+$/u.test(trimmed)
  ) {
    throw new Error('device approval request secret must be at least 32 base64url characters');
  }
  return trimmed;
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
