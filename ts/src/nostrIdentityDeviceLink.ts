import { finalizeEvent, generateSecretKey, getPublicKey, nip19, nip44, type Event } from 'nostr-tools';
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
  type NostrIdentityId,
  type NostrIdentityRosterOpContent,
  type SignedNostrIdentityRosterOp,
} from './nostrIdentity';
import { parseNostrIdentityRosterOpEvent } from './nostrIdentityEvents';
import { requireValidSignature } from './nostrIdentityJson';
import { nostrIdentityRosterParentIds } from './nostrIdentityProjection';

export const NOSTR_IDENTITY_DEVICE_LINK_INVITE_PREFIX = 'nostr-identity://device-link/';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX = 'nostr-identity://device-approval/';
export const NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION = 1;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION = 1;
export const NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE = 'nostr_identity_device_approval_proof';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE = 'nostr_identity_device_approval_receipt';
export const NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA = 1;
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
  profileId?: NostrIdentityId;
  adminAppKeyPubkey?: string;
  label?: string;
}

export interface LocalNostrIdentityDeviceApprovalRequest extends NostrIdentityDeviceApprovalRequest {
  requestSecretKey: Uint8Array;
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
  profileId?: string;
  adminAppKeyNpub?: string;
  label?: string;
}

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
    return normalizeDeviceLinkInvitePayload(JSON.parse(base64UrlDecode(payload)) as NostrIdentityDeviceLinkInvitePayload);
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
  profileId?: NostrIdentityId;
  adminAppKeyPubkey?: string;
  label?: string;
}): LocalNostrIdentityDeviceApprovalRequest {
  const deviceAppKeyPubkey = getPublicKey(options.deviceAppKeySecretKey);
  const requestSecretKey = options.requestSecretKey ?? generateSecretKey();
  const requestPubkey = getPublicKey(requestSecretKey);
  const requestSecret = requireRequestSecret(options.requestSecret ?? randomDeviceApprovalSecret());
  const requestedAt = requireInteger(options.requestedAt, 'requestedAt');
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
    ...(request.profileId !== undefined ? { profileId: requireProfileId(request.profileId) } : {}),
    ...(request.adminAppKeyPubkey !== undefined ? { adminAppKeyNpub: pubkeyToNpub(request.adminAppKeyPubkey) } : {}),
    ...(request.label?.trim() ? { label: request.label.trim() } : {}),
  };
  return `${options.prefix ?? NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX}${base64UrlEncode(JSON.stringify(payload))}`;
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
      JSON.parse(base64UrlDecode(payload)) as NostrIdentityDeviceApprovalRequestPayload,
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
    profileId: requireProfileId(options.profileId),
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
  requireValidSignature(event);
  if (event.kind !== FACT_OP_KIND) throw new Error('device approval receipt has invalid kind');
  requireProofTag(event, 'type', NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE);
  const requestPubkey = getPublicKey(options.requestSecretKey);
  requireProofTag(event, 'p', requestPubkey);
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
  if (options.request !== undefined) {
    if (receipt.requestSecret !== requireRequestSecret(options.request.requestSecret)) {
      throw new Error('device approval receipt secret mismatch');
    }
    if (receipt.deviceAppKeyPubkey !== requirePubkey(options.request.deviceAppKeyPubkey, 'device AppKey')) {
      throw new Error('device approval receipt device mismatch');
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
  payload: NostrIdentityDeviceLinkInvitePayload,
): NostrIdentityDeviceLinkInvite | null {
  if (payload.v !== NOSTR_IDENTITY_DEVICE_LINK_INVITE_VERSION) return null;
  const adminAppKeyPubkey = npubToPubkey(String(payload.adminAppKeyNpub ?? ''));
  const invitePubkey = npubToPubkey(String(payload.inviteNpub ?? ''));
  if (!adminAppKeyPubkey || !invitePubkey) return null;
  return {
    profileId: requireProfileId(String(payload.profileId ?? '')),
    adminAppKeyPubkey,
    invitePubkey,
  };
}

function normalizeDeviceApprovalRequestPayload(
  payload: NostrIdentityDeviceApprovalRequestPayload,
): NostrIdentityDeviceApprovalRequest | null {
  if (payload.v !== NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_VERSION) return null;
  const requestPubkey = npubToPubkey(String(payload.requestNpub ?? ''));
  const deviceAppKeyPubkey = npubToPubkey(String(payload.deviceAppKeyNpub ?? ''));
  if (!requestPubkey || !deviceAppKeyPubkey) return null;
  let adminAppKeyPubkey: string | undefined;
  if (payload.adminAppKeyNpub !== undefined) {
    const parsedAdmin = npubToPubkey(String(payload.adminAppKeyNpub));
    if (!parsedAdmin) return null;
    adminAppKeyPubkey = parsedAdmin;
  }
  const request = {
    requestPubkey,
    deviceAppKeyPubkey,
    requestSecret: requireRequestSecret(String(payload.requestSecret ?? '')),
    deviceAppKeyProof: String(payload.deviceAppKeyProof ?? ''),
    requestedAt: requireInteger(payload.requestedAt, 'requestedAt'),
    ...(payload.profileId !== undefined ? { profileId: requireProfileId(String(payload.profileId)) } : {}),
    ...(adminAppKeyPubkey !== undefined ? { adminAppKeyPubkey } : {}),
    ...(typeof payload.label === 'string' && payload.label.trim() ? { label: payload.label.trim() } : {}),
  };
  requireValidDeviceApprovalProof(request);
  return request;
}

function normalizeDeviceApprovalReceipt(
  receipt: NostrIdentityDeviceApprovalReceipt,
): NostrIdentityDeviceApprovalReceipt {
  if (receipt.schema !== NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA) {
    throw new Error(`unsupported device approval receipt schema ${receipt.schema}`);
  }
  return {
    schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
    profileId: requireProfileId(String(receipt.profileId ?? '')),
    requestPubkey: requirePubkey(String(receipt.requestPubkey ?? ''), 'request'),
    deviceAppKeyPubkey: requirePubkey(String(receipt.deviceAppKeyPubkey ?? ''), 'device AppKey'),
    approvedByPubkey: requirePubkey(String(receipt.approvedByPubkey ?? ''), 'approving AppKey'),
    approvedAt: requireInteger(receipt.approvedAt, 'approvedAt'),
    requestSecret: requireRequestSecret(String(receipt.requestSecret ?? '')),
    ...(receipt.subjectPubkey !== undefined ? { subjectPubkey: requirePubkey(String(receipt.subjectPubkey), 'subject') } : {}),
    ...(receipt.rosterOpId !== undefined ? { rosterOpId: requireEventId(String(receipt.rosterOpId)) } : {}),
    ...(typeof receipt.signedRosterEvent === 'string' && receipt.signedRosterEvent.trim()
      ? { signedRosterEvent: receipt.signedRosterEvent.trim() }
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
  profileId?: NostrIdentityId;
  adminAppKeyPubkey?: string;
  label?: string;
}): Event {
  const requestedAt = requireInteger(options.requestedAt, 'requestedAt');
  return finalizeEvent({
    kind: FACT_OP_KIND,
    content: '',
    created_at: requestedAt,
    tags: [
      ['type', NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE],
      ['request_pubkey', requirePubkey(options.requestPubkey, 'request')],
      ['requested_at', String(requestedAt)],
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
  if (event.pubkey !== requirePubkey(request.deviceAppKeyPubkey, 'device AppKey')) {
    throw new Error('device approval proof signer mismatch');
  }
  if (event.created_at !== requireInteger(request.requestedAt, 'requestedAt')) {
    throw new Error('device approval proof requested_at mismatch');
  }
  requireProofTag(event, 'type', NOSTR_IDENTITY_DEVICE_APPROVAL_PROOF_TYPE);
  requireProofTag(event, 'request_pubkey', requirePubkey(request.requestPubkey, 'request'));
  requireProofTag(event, 'requested_at', String(request.requestedAt));
  requireOptionalProofTag(event, 'profile_id', request.profileId);
  requireOptionalProofTag(event, 'admin_pubkey', request.adminAppKeyPubkey);
  requireOptionalProofTag(event, 'label', request.label);
  return raw;
}

function requireProofTag(event: Event, name: string, expected: string): void {
  if (event.tags.find((tag) => tag[0] === name)?.[1] !== expected) {
    throw new Error(`device approval proof ${name} mismatch`);
  }
}

function requireOptionalProofTag(event: Event, name: string, expected: string | undefined): void {
  const actual = event.tags.find((tag) => tag[0] === name)?.[1];
  if (expected === undefined) {
    if (actual !== undefined) throw new Error(`device approval proof unexpected ${name}`);
    return;
  }
  if (actual !== expected) throw new Error(`device approval proof ${name} mismatch`);
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
  if (looksLikePlaceholder(value)) throw new Error('device link payload is a placeholder');
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(value, 'base64url').toString('utf8');
  }
  let base64 = value.replace(/-/g, '+').replace(/_/g, '/');
  base64 += '='.repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(base64);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  return new TextDecoder().decode(bytes);
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

function requireRequestSecret(value: string): string {
  const trimmed = value.trim();
  if (!/^[A-Za-z0-9_-]{32,}$/u.test(trimmed)) {
    throw new Error('device approval request secret must be at least 32 base64url characters');
  }
  return trimmed;
}

function requireEventId(value: string): string {
  const trimmed = value.trim();
  if (!/^[0-9a-f]{64}$/i.test(trimmed)) throw new Error('roster op id must be a 64-char hex event id');
  return trimmed.toLowerCase();
}

function requireInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value)) throw new Error(`${label} must be an integer`);
  return value;
}

function requireNonEmpty(value: string, label: string): string {
  const trimmed = value.trim();
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
