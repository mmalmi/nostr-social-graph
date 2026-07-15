import {
  FACT_OP_KIND,
  buildFactOpTags,
  buildFactOpDraft,
  fact,
  parseFactOpEvent,
  type Fact,
  type FactEventDraft,
  type FactOp,
} from './factEvents';
import type { NostrEvent } from './utils';
import { finalizeEvent, getPublicKey, nip44, verifyEvent, type Event } from 'nostr-tools';

export const IDENTITY_GRAPH_ROSTER_SCHEMA = 1;
export const IDENTITY_GRAPH_KEY_ACCEPTANCE_SCHEMA = 1;
export const IDENTITY_GRAPH_ROSTER_TYPE = 'nostr_identity_roster_op';
export const IDENTITY_GRAPH_KEY_ACCEPTANCE_TYPE = 'nostr_identity_key_acceptance';
export const IDENTITY_GRAPH_LINK_REQUEST_TYPE = 'nostr_identity_link_request';

export const IDENTITY_CAPABILITY_ADMIN = 'admin';
export const IDENTITY_CAPABILITY_WRITE = 'write';
export const IDENTITY_CAPABILITY_RECOVER = 'recover';
export const IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS = 'receive_secret_wraps';
export const IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS = 'decrypt_secret_epochs';

export const IDENTITY_PURPOSE_APP = 'app';
export const IDENTITY_PURPOSE_RECOVERY = 'recovery';
export const IDENTITY_PURPOSE_REMOTE_SIGNER = 'remote_signer';
export const IDENTITY_PURPOSE_PROFILE = 'profile';
export const IDENTITY_PURPOSE_FIPS_TRANSPORT = 'fips_transport';

export type IdentityGraphId = string;
export type IdentityKeyPurpose = string;
export type IdentityKeyCapability = string;

export type IdentityEventDraft = FactEventDraft & {
  created_at: number;
};

export interface IdentityKey {
  pubkey: string;
  subject?: IdentityGraphId;
  purposes?: IdentityKeyPurpose[];
  capabilities?: IdentityKeyCapability[];
  addedAt: number;
  label?: string;
}

export type IdentityRosterOp =
  | { op: 'add_key'; key: IdentityKey }
  | { op: 'tombstone_key'; pubkey: string; reason?: string }
  | { op: 'set_key_capabilities'; pubkey: string; capabilities: IdentityKeyCapability[] }
  | { op: 'rotate_secret_epoch'; epoch: number; wrappedSecrets?: Record<string, string> }
  | { op: 'repair_secret_wraps'; epoch: number; wrappedSecrets?: Record<string, string> };

export interface IdentityRosterOpContent {
  schema: number;
  identity: IdentityGraphId;
  actorPubkey: string;
  actorSeq?: number;
  parents?: string[];
  clientNonce: string;
  createdAt: number;
  op: IdentityRosterOp;
}

export interface SignedIdentityRosterOp {
  opId: string;
  signerPubkey: string;
  content: IdentityRosterOpContent;
}

export interface IdentityKeyAcceptanceContent {
  schema: number;
  identity: IdentityGraphId;
  keyPubkey: string;
  purposes: IdentityKeyPurpose[];
  rosterOpId?: string;
  clientNonce: string;
  acceptedAt: number;
}

export interface IdentityLinkRequestContent {
  identity: IdentityGraphId;
  adminPubkey: string;
  invitePubkey: string;
  joiningPubkey: string;
  clientNonce: string;
  requestedAt: number;
  label?: string;
}

export interface SignedIdentityKeyAcceptance {
  acceptanceId: string;
  signerPubkey: string;
  content: IdentityKeyAcceptanceContent;
}

export interface SignedIdentityLinkRequest {
  requestId: string;
  signerPubkey: string;
  content: IdentityLinkRequestContent;
}

export interface IdentitySecretEpoch {
  epoch: number;
  createdAt: number;
  signedByPubkey: string;
  wrappedSecrets: Record<string, string>;
}

export interface IdentityKeyTombstone {
  pubkey: string;
  subject?: IdentityGraphId;
  removedByPubkey: string;
  removedAt: number;
  reason?: string;
}

export interface IdentityRosterProjection {
  identity: IdentityGraphId;
  activeKeys: Record<string, IdentityKey>;
  tombstones: Record<string, IdentityKeyTombstone>;
  secretEpochs: Record<string, IdentitySecretEpoch>;
  acceptedOpIds: string[];
  rejectedOpIds: string[];
}

export interface IdentityKeyAcceptanceProjection {
  identity: IdentityGraphId;
  acceptedKeys: Record<string, IdentityKeyAcceptanceContent>;
  acceptedAcceptanceIds: string[];
  rejectedAcceptanceIds: string[];
}

export interface FipsTransportIdentityBinding {
  transportPubkey: string;
  rosterOpId: string;
  acceptanceId: string;
}

export interface BuildIdentityRosterOpDraftOptions {
  signerPubkey: string;
  identity: IdentityGraphId;
  op: IdentityRosterOp;
  parents?: string[];
  actorSeq?: number;
  createdAt?: number;
  clientNonce?: string;
  extensionFacts?: Fact[];
}

export interface BuildIdentityKeyAcceptanceDraftOptions {
  signerPubkey: string;
  identity: IdentityGraphId;
  purposes: IdentityKeyPurpose[];
  rosterOpId?: string;
  acceptedAt?: number;
  clientNonce?: string;
}

export interface BuildIdentityLinkRequestEventOptions {
  signerSecretKey: Uint8Array;
  identity: IdentityGraphId;
  adminPubkey: string;
  invitePubkey: string;
  requestedAt?: number;
  clientNonce?: string;
  label?: string;
}

export interface ParseIdentityLinkRequestEventOptions {
  inviteSecretKey: Uint8Array;
  identity?: IdentityGraphId;
  adminPubkey?: string;
  invitePubkey?: string;
}

export const IDENTITY_ADMIN_CAPABILITIES: IdentityKeyCapability[] = [
  IDENTITY_CAPABILITY_ADMIN,
  IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
  IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS,
];

export const IDENTITY_APP_KEY_CAPABILITIES: IdentityKeyCapability[] = [
  IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
  IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS,
  IDENTITY_CAPABILITY_WRITE,
];

export function buildIdentityRosterOpDraft(options: BuildIdentityRosterOpDraftOptions): IdentityEventDraft {
  const actorPubkey = requireHexPubkey(options.signerPubkey, 'identity roster signer');
  const createdAt = options.createdAt ?? currentUnixSeconds();
  const clientNonce = options.clientNonce ?? randomIdentityNonce();
  const parents = normalizeEventIds(options.parents ?? [], 'parent');
  const content: IdentityRosterOpContent = {
    schema: IDENTITY_GRAPH_ROSTER_SCHEMA,
    identity: requireIdentityId(options.identity),
    actorPubkey,
    ...(options.actorSeq !== undefined ? { actorSeq: requireInteger(options.actorSeq, 'actorSeq') } : {}),
    ...(parents.length ? { parents } : {}),
    clientNonce: requireNonEmpty(clientNonce, 'clientNonce'),
    createdAt,
    op: normalizeIdentityRosterOp(options.op),
  };
  const draft = buildFactOpDraft(
    content.identity,
    [...rosterOpContentFacts(content), ...(options.extensionFacts ?? [])],
    { prev: parents },
  );
  return {
    kind: draft.kind,
    content: draft.content,
    created_at: createdAt,
    tags: draft.tags,
  };
}

export function buildIdentityKeyAcceptanceDraft(
  options: BuildIdentityKeyAcceptanceDraftOptions,
): IdentityEventDraft {
  const keyPubkey = requireHexPubkey(options.signerPubkey, 'identity key acceptance signer');
  const acceptedAt = options.acceptedAt ?? currentUnixSeconds();
  const clientNonce = options.clientNonce ?? randomIdentityNonce();
  const content: IdentityKeyAcceptanceContent = {
    schema: IDENTITY_GRAPH_KEY_ACCEPTANCE_SCHEMA,
    identity: requireIdentityId(options.identity),
    keyPubkey,
    purposes: normalizeTokens(options.purposes, 'purpose'),
    ...(options.rosterOpId !== undefined ? { rosterOpId: requireEventId(options.rosterOpId, 'rosterOpId') } : {}),
    clientNonce: requireNonEmpty(clientNonce, 'clientNonce'),
    acceptedAt,
  };
  if (content.purposes.length === 0) {
    throw new Error('identity key acceptance purposes must not be empty');
  }
  const draft = buildFactOpDraft(content.identity, keyAcceptanceContentFacts(content));
  return {
    kind: draft.kind,
    content: draft.content,
    created_at: acceptedAt,
    tags: draft.tags,
  };
}

export function buildIdentityLinkRequestEvent(
  options: BuildIdentityLinkRequestEventOptions,
): Event {
  const joiningPubkey = requireHexPubkey(getPublicKey(options.signerSecretKey), 'identity link request signer');
  const requestedAt = options.requestedAt ?? currentUnixSeconds();
  const clientNonce = options.clientNonce ?? randomIdentityNonce();
  const content: IdentityLinkRequestContent = {
    identity: requireIdentityId(options.identity),
    adminPubkey: requireHexPubkey(options.adminPubkey, 'identity link request admin'),
    invitePubkey: requireHexPubkey(options.invitePubkey, 'identity link request invite'),
    joiningPubkey,
    clientNonce: requireNonEmpty(clientNonce, 'clientNonce'),
    requestedAt: requireInteger(requestedAt, 'requestedAt'),
    ...(options.label?.trim() ? { label: options.label.trim() } : {}),
  };
  const conversationKey = nip44.v2.utils.getConversationKey(options.signerSecretKey, content.invitePubkey);
  const encrypted = nip44.v2.encrypt(JSON.stringify(linkRequestWireContent(content)), conversationKey);
  return finalizeEvent({
    kind: FACT_OP_KIND,
    content: encrypted,
    created_at: requestedAt,
    tags: linkRequestEventTags(content),
  }, options.signerSecretKey);
}

/** Parse a roster event whose event ID and signature are trusted or verified separately. */
export function parseIdentityRosterOpEvent(event: NostrEvent): SignedIdentityRosterOp {
  if (event.content !== '') throw new Error('identity roster op event must have empty content');
  const op = parseFactOpEvent(event);
  const content = rosterOpContentFromFacts(op);
  if (content.actorPubkey !== normalizeHexPubkey(event.pubkey)) {
    throw new Error('identity roster actor signer mismatch');
  }
  if (content.createdAt !== event.created_at) {
    throw new Error('identity roster created_at mismatch');
  }
  return {
    opId: requireEventId(event.id, 'identity roster op id'),
    signerPubkey: requireHexPubkey(event.pubkey, 'identity roster signer'),
    content,
  };
}

/** Parse an acceptance event whose event ID and signature are trusted or verified separately. */
export function parseIdentityKeyAcceptanceEvent(event: NostrEvent): SignedIdentityKeyAcceptance {
  if (event.content !== '') throw new Error('identity key acceptance event must have empty content');
  const op = parseFactOpEvent(event);
  const content = keyAcceptanceContentFromFacts(op);
  if (content.keyPubkey !== normalizeHexPubkey(event.pubkey)) {
    throw new Error('identity key acceptance signer mismatch');
  }
  if (content.acceptedAt !== event.created_at) {
    throw new Error('identity key acceptance accepted_at mismatch');
  }
  return {
    acceptanceId: requireEventId(event.id, 'identity key acceptance id'),
    signerPubkey: requireHexPubkey(event.pubkey, 'identity key acceptance signer'),
    content,
  };
}

export function parseIdentityLinkRequestEvent(
  event: NostrEvent,
  options: ParseIdentityLinkRequestEventOptions,
): SignedIdentityLinkRequest {
  const op = parseFactOpEvent({ ...event, content: '' });
  requireType(op, IDENTITY_GRAPH_LINK_REQUEST_TYPE);
  const expectedInvitePubkey = options.invitePubkey !== undefined
    ? requireHexPubkey(options.invitePubkey, 'identity link request invite')
    : getPublicKey(options.inviteSecretKey);
  if (!op.pubkeys.has(expectedInvitePubkey)) {
    throw new Error('identity link request invite pubkey mismatch');
  }
  const conversationKey = nip44.v2.utils.getConversationKey(
    options.inviteSecretKey,
    requireHexPubkey(event.pubkey, 'identity link request signer'),
  );
  const plaintext = nip44.v2.decrypt(event.content, conversationKey);
  const content = linkRequestContentFromWire(JSON.parse(plaintext) as IdentityLinkRequestWireContent);
  if (content.identity !== op.subject) {
    throw new Error('identity link request subject mismatch');
  }
  if (content.joiningPubkey !== normalizeHexPubkey(event.pubkey)) {
    throw new Error('identity link request signer mismatch');
  }
  if (content.invitePubkey !== expectedInvitePubkey) {
    throw new Error('identity link request invite pubkey mismatch');
  }
  if (options.identity !== undefined && content.identity !== requireIdentityId(options.identity)) {
    throw new Error('identity link request identity mismatch');
  }
  if (options.adminPubkey !== undefined && content.adminPubkey !== requireHexPubkey(options.adminPubkey, 'identity link request admin')) {
    throw new Error('identity link request admin mismatch');
  }
  if (content.requestedAt !== event.created_at) {
    throw new Error('identity link request requested_at mismatch');
  }
  return {
    requestId: requireEventId(event.id, 'identity link request id'),
    signerPubkey: requireHexPubkey(event.pubkey, 'identity link request signer'),
    content,
  };
}

export function identityRosterParentIds(ops: SignedIdentityRosterOp[]): string[] {
  const identity = ops[0]?.content.identity;
  return identity ? projectIdentityRoster(identity, ops).acceptedOpIds : [];
}

export function projectIdentityRoster(
  identity: IdentityGraphId,
  ops: SignedIdentityRosterOp[],
): IdentityRosterProjection {
  return projectIdentityRosterWithProvenance(identity, ops).projection;
}

function projectIdentityRosterWithProvenance(
  identity: IdentityGraphId,
  ops: SignedIdentityRosterOp[],
): { projection: IdentityRosterProjection; activeAddOpIds: Map<string, string> } {
  const normalizedIdentity = requireIdentityId(identity);
  const projection: IdentityRosterProjection = {
    identity: normalizedIdentity,
    activeKeys: {},
    tombstones: {},
    secretEpochs: {},
    acceptedOpIds: [],
    rejectedOpIds: [],
  };
  const activeAddOpIds = new Map<string, string>();
  const sorted = ops
    .filter((op) => op.content.identity === normalizedIdentity)
    .slice()
    .sort((left, right) => (left.content.createdAt - right.content.createdAt) || left.opId.localeCompare(right.opId));

  for (const signed of sorted) {
    if (!applyIdentityRosterOp(projection, signed)) {
      projection.rejectedOpIds.push(signed.opId);
      continue;
    }
    projection.acceptedOpIds.push(signed.opId);
    const op = signed.content.op;
    if (op.op === 'add_key' && !activeAddOpIds.has(op.key.pubkey)) {
      activeAddOpIds.set(op.key.pubkey, signed.opId);
    } else if (op.op === 'tombstone_key') {
      activeAddOpIds.delete(op.pubkey);
    }
  }
  return { projection, activeAddOpIds };
}

export function applyIdentityRosterOp(
  projection: IdentityRosterProjection,
  signed: SignedIdentityRosterOp,
): boolean {
  const op = signed.content.op;
  const signer = signed.signerPubkey;
  const hasAcceptedOps = projection.acceptedOpIds.length > 0;
  const isBootstrap = !hasAcceptedOps
    && op.op === 'add_key'
    && op.key.pubkey === signer
    && keyHasCapability(op.key, IDENTITY_CAPABILITY_ADMIN);
  const canAdmin = isBootstrap || identityKeyCanAdmin(projection, signer);
  const canRecover = identityKeyCanRecover(projection, signer);
  const canDecryptSecretEpochs = keyHasCapability(
    projection.activeKeys[signer],
    IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
  );
  const canRecoverRoster = canRecover && (
    (op.op === 'add_key' && keyHasPurpose(op.key, IDENTITY_PURPOSE_APP))
    || op.op === 'tombstone_key'
    || ((op.op === 'rotate_secret_epoch' || op.op === 'repair_secret_wraps') && canDecryptSecretEpochs)
  );
  const canRepairEpoch = op.op === 'repair_secret_wraps'
    && projection.secretEpochs[String(op.epoch)]?.signedByPubkey === signer;

  if (!canAdmin && !canRecoverRoster && !canRepairEpoch) return false;

  if (op.op === 'add_key') {
    delete projection.tombstones[op.key.pubkey];
    projection.activeKeys[op.key.pubkey] ??= {
      ...op.key,
      purposes: op.key.purposes ?? [],
      capabilities: op.key.capabilities ?? [],
    };
    return true;
  }
  if (op.op === 'tombstone_key') {
    const subject = projection.activeKeys[op.pubkey]?.subject;
    delete projection.activeKeys[op.pubkey];
    projection.tombstones[op.pubkey] = {
      pubkey: op.pubkey,
      ...(subject !== undefined ? { subject } : {}),
      removedByPubkey: signer,
      removedAt: signed.content.createdAt,
      ...(op.reason !== undefined ? { reason: op.reason } : {}),
    };
    return true;
  }
  if (op.op === 'set_key_capabilities') {
    const key = projection.activeKeys[op.pubkey];
    if (!key || projection.tombstones[op.pubkey]) return false;
    key.capabilities = op.capabilities;
    return true;
  }
  if (op.op === 'rotate_secret_epoch') {
    projection.secretEpochs[String(op.epoch)] = {
      epoch: op.epoch,
      createdAt: signed.content.createdAt,
      signedByPubkey: signer,
      wrappedSecrets: op.wrappedSecrets ?? {},
    };
    return true;
  }
  if (op.op === 'repair_secret_wraps') {
    const epoch = projection.secretEpochs[String(op.epoch)];
    if (!epoch || epoch.signedByPubkey !== signer) return false;
    epoch.wrappedSecrets = { ...epoch.wrappedSecrets, ...(op.wrappedSecrets ?? {}) };
    return true;
  }
  return false;
}

export function projectIdentityKeyAcceptances(
  identity: IdentityGraphId,
  acceptances: SignedIdentityKeyAcceptance[],
): IdentityKeyAcceptanceProjection {
  return projectIdentityKeyAcceptancesWithProvenance(identity, acceptances).projection;
}

function projectIdentityKeyAcceptancesWithProvenance(
  identity: IdentityGraphId,
  acceptances: SignedIdentityKeyAcceptance[],
): { projection: IdentityKeyAcceptanceProjection; acceptanceIds: Map<string, string> } {
  const normalizedIdentity = requireIdentityId(identity);
  const projection: IdentityKeyAcceptanceProjection = {
    identity: normalizedIdentity,
    acceptedKeys: {},
    acceptedAcceptanceIds: [],
    rejectedAcceptanceIds: [],
  };
  const acceptanceIds = new Map<string, string>();
  const sorted = acceptances
    .filter((acceptance) => acceptance.content.identity === normalizedIdentity)
    .slice()
    .sort((left, right) => (
      left.content.acceptedAt - right.content.acceptedAt
      || left.acceptanceId.localeCompare(right.acceptanceId)
    ));
  for (const signed of sorted) {
    if (signed.signerPubkey !== signed.content.keyPubkey || signed.content.purposes.length === 0) {
      projection.rejectedAcceptanceIds.push(signed.acceptanceId);
      continue;
    }
    projection.acceptedKeys[signed.content.keyPubkey] = signed.content;
    acceptanceIds.set(signed.content.keyPubkey, signed.acceptanceId);
    projection.acceptedAcceptanceIds.push(signed.acceptanceId);
  }
  return { projection, acceptanceIds };
}

export function resolveFipsTransportIdentityBindings(
  identity: IdentityGraphId,
  rosterEvents: readonly NostrEvent[],
  acceptanceEvents: readonly NostrEvent[],
): FipsTransportIdentityBinding[] {
  return resolveTrustedFipsTransportIdentityBindings(
    identity,
    parseVerifiedIdentityEvents(rosterEvents, parseIdentityRosterOpEvent),
    parseVerifiedIdentityEvents(acceptanceEvents, parseIdentityKeyAcceptanceEvent),
  );
}

/** Resolve records whose event IDs and signatures were verified before parsing. */
export function resolveTrustedFipsTransportIdentityBindings(
  identity: IdentityGraphId,
  rosterOps: SignedIdentityRosterOp[],
  acceptances: SignedIdentityKeyAcceptance[],
): FipsTransportIdentityBinding[] {
  const normalizedIdentity = requireIdentityId(identity);
  const { projection: roster, activeAddOpIds } = projectIdentityRosterWithProvenance(
    normalizedIdentity,
    rosterOps,
  );

  const { projection: acceptanceProjection, acceptanceIds } = (
    projectIdentityKeyAcceptancesWithProvenance(normalizedIdentity, acceptances)
  );
  return Object.entries(roster.activeKeys)
    .sort(([left], [right]) => left.localeCompare(right))
    .flatMap(([transportPubkey, key]) => {
      if ((key.capabilities ?? []).length !== 0
        || key.purposes?.length !== 1
        || !(key.purposes ?? []).includes(IDENTITY_PURPOSE_FIPS_TRANSPORT)) return [];
      const rosterOpId = activeAddOpIds.get(transportPubkey);
      const acceptance = acceptanceProjection.acceptedKeys[transportPubkey];
      if (!rosterOpId
        || acceptance?.rosterOpId !== rosterOpId
        || acceptance.purposes.length !== 1
        || !acceptance.purposes.includes(IDENTITY_PURPOSE_FIPS_TRANSPORT)) return [];
      const acceptanceId = acceptanceIds.get(transportPubkey);
      return acceptanceId ? [{ transportPubkey, rosterOpId, acceptanceId }] : [];
    });
}

function parseVerifiedIdentityEvents<T>(
  events: readonly NostrEvent[],
  parse: (event: NostrEvent) => T,
): T[] {
  const parsed: T[] = [];
  for (const event of events) {
    try {
      const wireEvent: NostrEvent = {
        id: event.id,
        pubkey: event.pubkey,
        created_at: event.created_at,
        kind: event.kind,
        tags: event.tags.map((tag) => tag.slice()),
        content: event.content,
        sig: event.sig,
      };
      if (!verifyEvent(wireEvent)) continue;
      parsed.push(parse(wireEvent));
    } catch {
      // Relay result sets may contain unrelated or malformed kind-7368 events.
    }
  }
  return parsed;
}

export function identityKeyCanAdmin(projection: IdentityRosterProjection, pubkey: string): boolean {
  const normalized = normalizeHexPubkey(pubkey);
  return Boolean(normalized && keyHasCapability(projection.activeKeys[normalized], IDENTITY_CAPABILITY_ADMIN));
}

export function identityKeyCanRecover(projection: IdentityRosterProjection, pubkey: string): boolean {
  const normalized = normalizeHexPubkey(pubkey);
  return Boolean(normalized && keyHasCapability(projection.activeKeys[normalized], IDENTITY_CAPABILITY_RECOVER));
}

export function identityKey(
  pubkey: string,
  options: {
    addedAt: number;
    subject?: IdentityGraphId;
    purposes?: IdentityKeyPurpose[];
    capabilities?: IdentityKeyCapability[];
    label?: string;
  },
): IdentityKey {
  return normalizeIdentityKey({
    pubkey,
    ...(options.subject !== undefined ? { subject: options.subject } : {}),
    purposes: options.purposes ?? [IDENTITY_PURPOSE_APP],
    capabilities: options.capabilities ?? IDENTITY_APP_KEY_CAPABILITIES,
    addedAt: options.addedAt,
    ...(options.label !== undefined ? { label: options.label } : {}),
  });
}

export function normalizeIdentityCapabilities(
  capabilities: Iterable<IdentityKeyCapability>,
): IdentityKeyCapability[] {
  return normalizeTokens([...capabilities], 'capability');
}

export function normalizeIdentityPurposes(purposes: Iterable<IdentityKeyPurpose>): IdentityKeyPurpose[] {
  return normalizeTokens([...purposes], 'purpose');
}

export function normalizeHexPubkey(value: string): string | null {
  const lower = value.trim().toLowerCase();
  return /^[0-9a-f]{64}$/.test(lower) ? lower : null;
}

export function isIdentityGraphId(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value.trim());
}

export function randomIdentityNonce(): string {
  const bytes = new Uint8Array(16);
  const cryptoApi = globalThis.crypto;
  if (cryptoApi?.getRandomValues) {
    cryptoApi.getRandomValues(bytes);
  } else {
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Math.floor(Math.random() * 256);
    }
  }
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

export function currentUnixSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

function rosterOpContentFacts(content: IdentityRosterOpContent): Fact[] {
  return [
    fact('type', [IDENTITY_GRAPH_ROSTER_TYPE]),
    fact('schema', [String(content.schema)]),
    fact('actor_pubkey', [content.actorPubkey]),
    ...(content.actorSeq !== undefined ? [fact('actor_seq', [String(content.actorSeq)])] : []),
    fact('client_nonce', [content.clientNonce]),
    fact('created_at', [String(content.createdAt)]),
    fact('op', [content.op.op]),
    ...rosterOpFacts(content.op),
  ];
}

function rosterOpFacts(op: IdentityRosterOp): Fact[] {
  if (op.op === 'add_key') {
    return [
      fact('key_pubkey', [op.key.pubkey]),
      ...(op.key.subject !== undefined ? [fact('key_subject', [op.key.subject])] : []),
      ...(op.key.purposes ?? []).map((purpose) => fact('key_purpose', [purpose])),
      ...(op.key.capabilities ?? []).map((capability) => fact('key_capability', [capability])),
      fact('key_added_at', [String(op.key.addedAt)]),
      ...(op.key.label !== undefined ? [fact('key_label', [op.key.label])] : []),
    ];
  }
  if (op.op === 'tombstone_key') {
    return [
      fact('target_pubkey', [op.pubkey]),
      ...(op.reason !== undefined ? [fact('reason', [op.reason])] : []),
    ];
  }
  if (op.op === 'set_key_capabilities') {
    return [
      fact('target_pubkey', [op.pubkey]),
      ...op.capabilities.map((capability) => fact('capability', [capability])),
    ];
  }
  return [
    fact('secret_epoch', [String(op.epoch)]),
    ...Object.entries(op.wrappedSecrets ?? {})
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([pubkey, wrapped]) => fact('wrapped_secret', [pubkey, wrapped])),
  ];
}

function keyAcceptanceContentFacts(content: IdentityKeyAcceptanceContent): Fact[] {
  return [
    fact('type', [IDENTITY_GRAPH_KEY_ACCEPTANCE_TYPE]),
    fact('schema', [String(content.schema)]),
    fact('key_pubkey', [content.keyPubkey]),
    ...content.purposes.map((purpose) => fact('purpose', [purpose])),
    ...(content.rosterOpId !== undefined ? [fact('roster_op_id', [content.rosterOpId])] : []),
    fact('client_nonce', [content.clientNonce]),
    fact('accepted_at', [String(content.acceptedAt)]),
  ];
}

function rosterOpContentFromFacts(op: FactOp): IdentityRosterOpContent {
  requireType(op, IDENTITY_GRAPH_ROSTER_TYPE);
  const schema = requiredInteger(op, 'schema');
  if (schema !== IDENTITY_GRAPH_ROSTER_SCHEMA) {
    throw new Error(`unsupported Nostr identity roster schema ${schema}`);
  }
  const actorSeq = optionalInteger(op, 'actor_seq');
  return {
    schema,
    identity: op.subject,
    actorPubkey: requiredPubkey(op, 'actor_pubkey'),
    ...(actorSeq !== undefined ? { actorSeq } : {}),
    ...(op.prev.length ? { parents: op.prev } : {}),
    clientNonce: requiredNonEmptyScalar(op, 'client_nonce'),
    createdAt: requiredInteger(op, 'created_at'),
    op: rosterOpFromFacts(op),
  };
}

function rosterOpFromFacts(op: FactOp): IdentityRosterOp {
  const kind = requiredScalar(op, 'op');
  if (kind === 'add_key') {
    return {
      op: 'add_key',
      key: normalizeIdentityKey({
        pubkey: requiredPubkey(op, 'key_pubkey'),
        ...(optionalScalar(op, 'key_subject') !== undefined ? { subject: optionalScalar(op, 'key_subject') } : {}),
        purposes: scalarValues(op, 'key_purpose'),
        capabilities: scalarValues(op, 'key_capability'),
        addedAt: requiredInteger(op, 'key_added_at'),
        ...(optionalScalar(op, 'key_label') !== undefined ? { label: optionalScalar(op, 'key_label') } : {}),
      }),
    };
  }
  if (kind === 'tombstone_key') {
    return {
      op: 'tombstone_key',
      pubkey: requiredPubkey(op, 'target_pubkey'),
      ...(optionalScalar(op, 'reason') !== undefined ? { reason: optionalScalar(op, 'reason') } : {}),
    };
  }
  if (kind === 'set_key_capabilities') {
    return {
      op: 'set_key_capabilities',
      pubkey: requiredPubkey(op, 'target_pubkey'),
      capabilities: normalizeTokens(scalarValues(op, 'capability'), 'capability'),
    };
  }
  if (kind === 'rotate_secret_epoch' || kind === 'repair_secret_wraps') {
    return {
      op: kind,
      epoch: requiredInteger(op, 'secret_epoch'),
      wrappedSecrets: wrappedSecretsFromFacts(op),
    };
  }
  throw new Error(`unsupported Nostr identity roster op ${kind}`);
}

function keyAcceptanceContentFromFacts(op: FactOp): IdentityKeyAcceptanceContent {
  requireType(op, IDENTITY_GRAPH_KEY_ACCEPTANCE_TYPE);
  const schema = requiredInteger(op, 'schema');
  if (schema !== IDENTITY_GRAPH_KEY_ACCEPTANCE_SCHEMA) {
    throw new Error(`unsupported Nostr identity key acceptance schema ${schema}`);
  }
  const content: IdentityKeyAcceptanceContent = {
    schema,
    identity: op.subject,
    keyPubkey: requiredPubkey(op, 'key_pubkey'),
    purposes: normalizeTokens(scalarValues(op, 'purpose'), 'purpose'),
    ...(optionalScalar(op, 'roster_op_id') !== undefined ? { rosterOpId: optionalScalar(op, 'roster_op_id') } : {}),
    clientNonce: requiredNonEmptyScalar(op, 'client_nonce'),
    acceptedAt: requiredInteger(op, 'accepted_at'),
  };
  if (content.purposes.length === 0) {
    throw new Error('identity key acceptance purposes must not be empty');
  }
  if (content.rosterOpId !== undefined) {
    content.rosterOpId = requireEventId(content.rosterOpId, 'roster_op_id');
  }
  return content;
}

function normalizeIdentityRosterOp(op: IdentityRosterOp): IdentityRosterOp {
  if (op.op === 'add_key') {
    return { op: 'add_key', key: normalizeIdentityKey(op.key) };
  }
  if (op.op === 'tombstone_key') {
    return {
      op: 'tombstone_key',
      pubkey: requireHexPubkey(op.pubkey, 'target'),
      ...(op.reason !== undefined ? { reason: op.reason.trim() } : {}),
    };
  }
  if (op.op === 'set_key_capabilities') {
    return {
      op: 'set_key_capabilities',
      pubkey: requireHexPubkey(op.pubkey, 'target'),
      capabilities: normalizeTokens(op.capabilities, 'capability'),
    };
  }
  return {
    op: op.op,
    epoch: requireInteger(op.epoch, 'secret epoch'),
    wrappedSecrets: normalizeWrappedSecrets(op.wrappedSecrets ?? {}),
  };
}

type IdentityLinkRequestWireContent = {
  identity?: unknown;
  admin_pubkey?: unknown;
  invite_pubkey?: unknown;
  joining_pubkey?: unknown;
  client_nonce?: unknown;
  requested_at?: unknown;
  label?: unknown;
};

function linkRequestEventTags(content: IdentityLinkRequestContent): string[][] {
  const tags = buildFactOpTags(content.identity, [
    fact('type', [IDENTITY_GRAPH_LINK_REQUEST_TYPE]),
  ]);
  tags.push(['p', content.invitePubkey]);
  return tags;
}

function linkRequestWireContent(content: IdentityLinkRequestContent): Record<string, string | number> {
  return {
    identity: content.identity,
    admin_pubkey: content.adminPubkey,
    invite_pubkey: content.invitePubkey,
    joining_pubkey: content.joiningPubkey,
    client_nonce: content.clientNonce,
    requested_at: content.requestedAt,
    ...(content.label !== undefined ? { label: content.label } : {}),
  };
}

function linkRequestContentFromWire(wire: IdentityLinkRequestWireContent): IdentityLinkRequestContent {
  return {
    identity: requireIdentityId(requiredWireString(wire.identity, 'identity')),
    adminPubkey: requireHexPubkey(requiredWireString(wire.admin_pubkey, 'admin_pubkey'), 'identity link request admin'),
    invitePubkey: requireHexPubkey(requiredWireString(wire.invite_pubkey, 'invite_pubkey'), 'identity link request invite'),
    joiningPubkey: requireHexPubkey(requiredWireString(wire.joining_pubkey, 'joining_pubkey'), 'identity link request signer'),
    clientNonce: requireNonEmpty(requiredWireString(wire.client_nonce, 'client_nonce'), 'clientNonce'),
    requestedAt: requireInteger(requiredWireNumber(wire.requested_at, 'requested_at'), 'requestedAt'),
    ...(typeof wire.label === 'string' && wire.label.trim() ? { label: wire.label.trim() } : {}),
  };
}

function requiredWireString(value: unknown, label: string): string {
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error(`identity link request ${label} is required`);
  }
  return value;
}

function requiredWireNumber(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isInteger(value)) {
    throw new Error(`identity link request ${label} must be an integer`);
  }
  return value;
}

function normalizeIdentityKey(key: IdentityKey): IdentityKey {
  return {
    pubkey: requireHexPubkey(key.pubkey, 'identity key'),
    ...(key.subject !== undefined ? { subject: requireIdentityId(key.subject) } : {}),
    purposes: normalizeTokens(key.purposes ?? [], 'purpose'),
    capabilities: normalizeTokens(key.capabilities ?? [], 'capability'),
    addedAt: requireInteger(key.addedAt, 'key addedAt'),
    ...(key.label?.trim() ? { label: key.label.trim() } : {}),
  };
}

function normalizeWrappedSecrets(wrappedSecrets: Record<string, string>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(wrappedSecrets)
      .map(([pubkey, wrapped]) => [
        requireHexPubkey(pubkey, 'wrapped secret recipient'),
        requireNonEmpty(wrapped, 'wrapped secret'),
      ] as const)
      .sort(([left], [right]) => left.localeCompare(right)),
  );
}

function wrappedSecretsFromFacts(op: FactOp): Record<string, string> {
  return Object.fromEntries(
    valueTuples(op, 'wrapped_secret').map((values) => {
      if (values.length !== 2) throw new Error('wrapped_secret fact must have pubkey and wrapped value');
      return [requireHexPubkey(values[0], 'wrapped secret recipient'), requireNonEmpty(values[1], 'wrapped secret')] as const;
    }).sort(([left], [right]) => left.localeCompare(right)),
  );
}

function keyHasCapability(key: IdentityKey | undefined, capability: string): boolean {
  return Boolean(key?.capabilities?.includes(capability));
}

function keyHasPurpose(key: IdentityKey | undefined, purpose: string): boolean {
  return Boolean(key?.purposes?.includes(purpose));
}

function requireType(op: FactOp, expected: string): void {
  const value = requiredScalar(op, 'type');
  if (value !== expected) throw new Error(`unexpected Nostr identity fact event type ${value}`);
}

function requiredPubkey(op: FactOp, predicate: string): string {
  return requireHexPubkey(requiredScalar(op, predicate), predicate);
}

function requiredScalar(op: FactOp, predicate: string): string {
  const value = optionalScalar(op, predicate);
  if (value === undefined) throw new Error(`missing Nostr identity fact ${predicate}`);
  return value;
}

function requiredNonEmptyScalar(op: FactOp, predicate: string): string {
  return requireNonEmpty(requiredScalar(op, predicate), predicate);
}

function optionalScalar(op: FactOp, predicate: string): string | undefined {
  const values = valueTuples(op, predicate);
  if (values.length === 0) return undefined;
  if (values.length !== 1 || values[0].length !== 1) {
    throw new Error(`Nostr identity fact ${predicate} must be a single scalar`);
  }
  return values[0][0];
}

function scalarValues(op: FactOp, predicate: string): string[] {
  return valueTuples(op, predicate).map((values) => {
    if (values.length !== 1) throw new Error(`Nostr identity fact ${predicate} must be scalar`);
    return values[0];
  });
}

function valueTuples(op: FactOp, predicate: string): string[][] {
  return op.facts
    .filter((item) => item.predicate === predicate)
    .map((item) => item.values);
}

function requiredInteger(op: FactOp, predicate: string): number {
  return requireInteger(requiredScalar(op, predicate), predicate);
}

function optionalInteger(op: FactOp, predicate: string): number | undefined {
  const value = optionalScalar(op, predicate);
  return value === undefined ? undefined : requireInteger(value, predicate);
}

function requireInteger(value: string | number, label: string): number {
  const parsed = typeof value === 'number' ? value : Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0 || String(parsed) !== String(value)) {
    throw new Error(`Nostr identity ${label} must be a non-negative integer`);
  }
  return parsed;
}

function requireIdentityId(value: string): string {
  const trimmed = value.trim().toLowerCase();
  if (!isIdentityGraphId(trimmed) || trimmed !== value.trim()) {
    throw new Error(`Nostr identity id must be a canonical UUID: ${value}`);
  }
  return trimmed;
}

function requireHexPubkey(value: string, label: string): string {
  const normalized = normalizeHexPubkey(value);
  if (!normalized) throw new Error(`${label} pubkey must be 64-char hex`);
  return normalized;
}

function requireEventId(value: string, label: string): string {
  const normalized = normalizeHexPubkey(value);
  if (!normalized) throw new Error(`${label} must be 64-char hex`);
  return normalized;
}

function requireNonEmpty(value: string, label: string): string {
  const trimmed = value.trim();
  if (!trimmed) throw new Error(`Nostr identity ${label} must not be empty`);
  return trimmed;
}

function normalizeTokens(values: Iterable<string>, label: string): string[] {
  return [...new Set([...values].map((value) => normalizeToken(value, label)))].sort();
}

function normalizeToken(value: string, label: string): string {
  const normalized = value.trim().toLowerCase();
  if (!normalized) throw new Error(`Nostr identity ${label} must not be empty`);
  if (/\s/.test(normalized)) throw new Error(`Nostr identity ${label} must not contain whitespace: ${value}`);
  return normalized;
}

function normalizeEventIds(values: string[], label: string): string[] {
  return [...new Set(values.map((value) => requireEventId(value, label)))].sort();
}
