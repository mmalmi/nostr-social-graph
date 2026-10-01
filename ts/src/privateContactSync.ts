import { verifyEvent, type Event, type EventTemplate } from 'nostr-tools';

export const PRIVATE_CONTACT_SYNC_KIND = 30078;
export const PRIVATE_CONTACT_SYNC_NAMESPACE = 'nostr-social-memory/v1';
export const PRIVATE_CONTACT_MAX_NOTE_BYTES = 16_384;
export const PRIVATE_CONTACT_MAX_NICKNAME_BYTES = 320;
const MAX_PLAINTEXT_BYTES = 24_576;
const MAX_CIPHERTEXT_BYTES = 40_000;
const HEX_KEY = /^[0-9a-f]{64}$/;
const HEX_ID = /^[0-9a-f]{32}$/;
const FIELDS = ['favorite', 'nickname', 'note'] as const;
export type PrivateContactField = typeof FIELDS[number];
export type PrivateContactValue = boolean | string | null;
export type PrivateContactPatch = { favorite?: boolean; nickname?: string | null; note?: string | null };
export interface PrivateContactRegister {
  counter: number;
  writer: string;
  value: PrivateContactValue;
}
export type PrivateContactFields = Partial<Record<PrivateContactField, PrivateContactRegister>>;
export interface PrivateContactDocument {
  version: 1;
  owner: string;
  contact: string;
  writer: string;
  record_id: string;
  fields: PrivateContactFields;
}
export interface PrivateContactLocalRecord {
  document: PrivateContactDocument;
  pending: boolean;
  event: Event | null;
  last_created_at: number;
}
/** Persist the entire returned state atomically before projecting or publishing it. */
export interface PrivateContactSyncState {
  version: 1;
  owner: string;
  writer: string;
  clock: number;
  contacts: Record<string, PrivateContactFields>;
  records: Record<string, PrivateContactLocalRecord>;
  received_event_ids: string[];
}
/** Structurally compatible with @iris/identity's existing signers. */
export interface PrivateContactSigner {
  getPublicKey(): string | Promise<string>;
  signEvent(draft: EventTemplate): Event | Promise<Event>;
  nip44Encrypt?(recipient: string, plaintext: string): string | Promise<string>;
  nip44Decrypt?(sender: string, ciphertext: string): string | Promise<string>;
}

function requireKey(value: unknown): asserts value is string {
  if (typeof value !== 'string' || !HEX_KEY.test(value)) throw new Error('Invalid contact owner or public key');
}
function requireId(value: unknown): asserts value is string {
  if (typeof value !== 'string' || !HEX_ID.test(value)) throw new Error('Invalid private contact writer or record ID');
}
function requireCounter(value: unknown): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) throw new Error('Invalid private contact counter');
}
function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}
function bytes(value: string): number { return new TextEncoder().encode(value).length; }
function validateValue(field: PrivateContactField, value: unknown): void {
  if (field === 'favorite') {
    if (typeof value !== 'boolean') throw new Error('Favorite must be a boolean');
  } else if (value !== null && (typeof value !== 'string'
    || bytes(value) > (field === 'note' ? PRIVATE_CONTACT_MAX_NOTE_BYTES : PRIVATE_CONTACT_MAX_NICKNAME_BYTES))) {
    throw new Error(`Invalid or oversized private contact ${field}`);
  }
}
function validateFields(value: unknown): asserts value is PrivateContactFields {
  if (!object(value) || Object.keys(value).some(key => !FIELDS.includes(key as PrivateContactField))) {
    throw new Error('Invalid private contact fields');
  }
  for (const field of FIELDS) {
    const register = value[field];
    if (register === undefined) continue;
    if (!object(register)) throw new Error('Invalid private contact register');
    requireCounter(register.counter);
    requireId(register.writer);
    validateValue(field, register.value);
  }
}
export function validatePrivateContactDocument(value: unknown, expectedOwner: string): asserts value is PrivateContactDocument {
  requireKey(expectedOwner);
  if (!object(value) || value.version !== 1 || value.owner !== expectedOwner) throw new Error('Private contact account or version mismatch');
  requireKey(value.contact);
  requireId(value.writer);
  requireId(value.record_id);
  validateFields(value.fields);
  if (!Object.keys(value.fields).length || bytes(JSON.stringify(value)) > MAX_PLAINTEXT_BYTES) throw new Error('Invalid private contact document size');
}
function cloneFields(fields: PrivateContactFields): PrivateContactFields {
  return Object.fromEntries(FIELDS.filter(field => fields[field]).map(field => [field, { ...fields[field]! }]));
}
function cloneDocument(document: PrivateContactDocument): PrivateContactDocument {
  return { ...document, fields: cloneFields(document.fields) };
}
/** Same counter/writer equivocation also converges by UTF-8 JSON value order. */
function compare(left: PrivateContactRegister, right: PrivateContactRegister): number {
  if (left.counter !== right.counter) return left.counter < right.counter ? -1 : 1;
  if (left.writer !== right.writer) return left.writer < right.writer ? -1 : 1;
  const a = new TextEncoder().encode(JSON.stringify(left.value));
  const b = new TextEncoder().encode(JSON.stringify(right.value));
  for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  return Math.sign(a.length - b.length);
}
export function mergePrivateContactFields(left: PrivateContactFields, right: PrivateContactFields): PrivateContactFields {
  validateFields(left); validateFields(right);
  const merged = cloneFields(left);
  for (const field of FIELDS) {
    const incoming = right[field];
    if (incoming && (!merged[field] || compare(incoming, merged[field]!) > 0)) merged[field] = { ...incoming };
  }
  return merged;
}
export function createPrivateContactSync(owner: string, writer: string): PrivateContactSyncState {
  requireKey(owner); requireId(writer);
  return { version: 1, owner, writer, clock: 0, contacts: {}, records: {}, received_event_ids: [] };
}
export function rememberPrivateContactEvent(state: PrivateContactSyncState, eventId: string): PrivateContactSyncState {
  requireKey(eventId);
  if (state.received_event_ids.includes(eventId)) return state;
  return { ...state, received_event_ids: [...state.received_event_ids, eventId].slice(-512) };
}
/** Reject corrupt/account-mismatched persistence instead of silently replacing a replica. */
export function restorePrivateContactSync(value: unknown, expectedOwner: string): PrivateContactSyncState {
  if (!object(value) || value.version !== 1 || value.owner !== expectedOwner || !object(value.contacts) || !object(value.records)) {
    throw new Error('Invalid saved private contact state');
  }
  requireKey(expectedOwner); requireId(value.writer); requireCounter(value.clock);
  const state = createPrivateContactSync(expectedOwner, value.writer);
  state.clock = value.clock;
  if (value.received_event_ids !== undefined) {
    if (!Array.isArray(value.received_event_ids) || value.received_event_ids.length > 512) throw new Error('Invalid received contact events');
    value.received_event_ids.forEach(requireKey);
    state.received_event_ids = [...new Set(value.received_event_ids)];
  }
  for (const [contact, fields] of Object.entries(value.contacts)) {
    requireKey(contact); validateFields(fields);
    state.contacts[contact] = cloneFields(fields);
  }
  for (const [contact, record] of Object.entries(value.records)) {
    requireKey(contact);
    if (!object(record) || typeof record.pending !== 'boolean') throw new Error('Invalid saved contact record');
    validatePrivateContactDocument(record.document, expectedOwner);
    if (record.document.contact !== contact || record.document.writer !== state.writer) throw new Error('Saved contact writer mismatch');
    requireCounter(record.last_created_at);
    const event = record.event === null ? null : freshEvent(record.event as Event);
    if (event && (event.pubkey !== expectedOwner || event.kind !== PRIVATE_CONTACT_SYNC_KIND
      || event.created_at !== record.last_created_at || JSON.stringify(event.tags) !== JSON.stringify(documentTags(record.document))
      || event.content.length > MAX_CIPHERTEXT_BYTES || !verifyEvent(freshEvent(event)))) throw new Error('Invalid saved contact event');
    state.records[contact] = { document: cloneDocument(record.document), pending: record.pending, event, last_created_at: record.last_created_at };
    state.contacts[contact] = mergePrivateContactFields(state.contacts[contact] ?? {}, record.document.fields);
  }
  for (const fields of Object.values(state.contacts)) {
    if (Object.values(fields).some(register => register.counter > state.clock)) throw new Error('Saved contact clock is behind its data');
  }
  return state;
}
export function privateContactValues(state: PrivateContactSyncState, contact: string): Required<PrivateContactPatch> {
  const fields = state.contacts[contact];
  return {
    favorite: fields?.favorite?.value === true,
    nickname: typeof fields?.nickname?.value === 'string' ? fields.nickname.value : null,
    note: typeof fields?.note?.value === 'string' ? fields.note.value : null,
  };
}
/** Authenticated sibling snapshot only. Receivers stage into their OWN random publication addresses. */
export function privateContactDocuments(state: PrivateContactSyncState): PrivateContactDocument[] {
  return Object.entries(state.contacts).map(([contact, fields]) => ({
    version: 1, owner: state.owner, contact, writer: state.writer,
    record_id: state.records[contact]?.document.record_id ?? state.writer,
    fields: cloneFields(fields),
  }));
}
/** Trusted local storage / authenticated sibling bridge entry. Nostr input must use openPrivateContactEvent first. */
export function mergePrivateContactDocument(state: PrivateContactSyncState, document: PrivateContactDocument): PrivateContactSyncState {
  validatePrivateContactDocument(document, state.owner);
  const fields = mergePrivateContactFields(state.contacts[document.contact] ?? {}, document.fields);
  const clock = Math.max(state.clock, ...Object.values(document.fields).map(register => register.counter));
  return { ...state, clock, contacts: { ...state.contacts, [document.contact]: fields } };
}
function saveOwnFields(state: PrivateContactSyncState, contact: string, fields: PrivateContactFields, recordId?: string): PrivateContactSyncState {
  const existing = state.records[contact];
  if (!existing) requireId(recordId);
  const document: PrivateContactDocument = {
    version: 1, owner: state.owner, contact, writer: state.writer,
    record_id: existing?.document.record_id ?? recordId!,
    fields: mergePrivateContactFields(existing?.document.fields ?? {}, fields),
  };
  validatePrivateContactDocument(document, state.owner);
  return {
    ...mergePrivateContactDocument(state, document),
    records: { ...state.records, [contact]: { document, pending: true, event: null, last_created_at: existing?.last_created_at ?? 0 } },
  };
}
function applyPatch(state: PrivateContactSyncState, contact: string, patch: PrivateContactPatch, recordId: string | undefined, seed: boolean): PrivateContactSyncState {
  requireKey(contact);
  if (!object(patch) || Object.keys(patch).some(key => !FIELDS.includes(key as PrivateContactField))) throw new Error('Invalid private contact patch');
  const fields: PrivateContactFields = {};
  for (const field of FIELDS) {
    const value = patch[field];
    if (value === undefined) continue;
    validateValue(field, value);
    const known = state.contacts[contact]?.[field];
    // Absent/default legacy fields are not edits. Counter-zero seeds lose to every real edit.
    if (seed && (known || value === false || value === null || value === '')) continue;
    if (!seed && known?.value === value) continue;
    const counter = seed ? 0 : state.clock + 1;
    requireCounter(counter);
    fields[field] = { value, counter, writer: state.writer };
  }
  return Object.keys(fields).length ? saveOwnFields(state, contact, fields, recordId) : state;
}
export function editPrivateContact(state: PrivateContactSyncState, contact: string, patch: PrivateContactPatch, recordId?: string): PrivateContactSyncState {
  return applyPatch(state, contact, patch, recordId, false);
}
export function seedPrivateContact(state: PrivateContactSyncState, contact: string, patch: PrivateContactPatch, recordId?: string): PrivateContactSyncState {
  return applyPatch(state, contact, patch, recordId, true);
}
/** After authenticating a sibling channel, a full-key device may relay its registers without inventing edits. */
export function stagePrivateContactDocument(state: PrivateContactSyncState, document: PrivateContactDocument, recordId?: string): PrivateContactSyncState {
  const merged = mergePrivateContactDocument(state, document);
  const fields = merged.contacts[document.contact];
  if (JSON.stringify(state.records[document.contact]?.document.fields) === JSON.stringify(fields)) return merged;
  return saveOwnFields(merged, document.contact, fields, recordId);
}
export function pendingPrivateContacts(state: PrivateContactSyncState): PrivateContactLocalRecord[] {
  return Object.values(state.records).filter(record => record.pending);
}
export function privateContactSyncFilter(owner: string) {
  requireKey(owner);
  return { kinds: [PRIVATE_CONTACT_SYNC_KIND], authors: [owner], '#t': [PRIVATE_CONTACT_SYNC_NAMESPACE] };
}
function documentTags(document: PrivateContactDocument): string[][] {
  return [['d', `${PRIVATE_CONTACT_SYNC_NAMESPACE}:${document.writer}:${document.record_id}`], ['t', PRIVATE_CONTACT_SYNC_NAMESPACE]];
}
function freshEvent(event: Event): Event {
  // Do not trust a mutable object's nostr-tools signature verification cache.
  return JSON.parse(JSON.stringify(event));
}
async function assertSigner(signer: PrivateContactSigner, owner: string): Promise<void> {
  if (await signer.getPublicKey() !== owner) throw new Error('Private contact signer account changed');
}
/** Return exact retry bytes; do not publish until this returned state is durable. Serialize this with edits/merges. */
export async function preparePrivateContactEvent(state: PrivateContactSyncState, contact: string, signer: PrivateContactSigner, nowSecs: number): Promise<{ state: PrivateContactSyncState; event: Event | null; retryAt?: number }> {
  requireCounter(nowSecs);
  const record = state.records[contact];
  if (!record?.pending) return { state, event: null };
  await assertSigner(signer, state.owner);
  if (record.event) return { state, event: record.event };
  // Never compete with a preceding head in NIP-01's same-second event-ID tie-break or forge a future timestamp.
  if (nowSecs <= record.last_created_at) return { state, event: null, retryAt: record.last_created_at + 1 };
  if (!signer.nip44Encrypt) throw new Error('This signer does not support private contact encryption');
  validatePrivateContactDocument(record.document, state.owner);
  const content = await signer.nip44Encrypt(state.owner, JSON.stringify(record.document));
  await assertSigner(signer, state.owner);
  if (typeof content !== 'string' || content.length > MAX_CIPHERTEXT_BYTES || !content.startsWith('A')) throw new Error('Invalid NIP44 encrypted contact content');
  const draft: EventTemplate = { kind: PRIVATE_CONTACT_SYNC_KIND, created_at: nowSecs, tags: documentTags(record.document), content };
  const event = freshEvent(await signer.signEvent(draft));
  await assertSigner(signer, state.owner);
  if (event.pubkey !== state.owner || event.kind !== draft.kind || event.created_at !== draft.created_at || event.content !== draft.content
    || JSON.stringify(event.tags) !== JSON.stringify(draft.tags) || !verifyEvent(freshEvent(event))) throw new Error('Invalid private contact signature or signed draft');
  return { state: { ...state, records: { ...state.records, [contact]: { ...record, event, last_created_at: nowSecs } } }, event };
}
/** Call only after explicit remote acceptance, with the exact published event ID. */
export function acknowledgePrivateContactEvent(state: PrivateContactSyncState, contact: string, eventId: string): PrivateContactSyncState {
  const record = state.records[contact];
  if (!record?.pending || record.event?.id !== eventId) return state;
  return { ...state, records: { ...state.records, [contact]: { ...record, pending: false } } };
}
/** Validate account, namespace and signature before asking the signer to decrypt anything. */
export async function openPrivateContactEvent(raw: Event, owner: string, signer: PrivateContactSigner): Promise<PrivateContactDocument> {
  requireKey(owner);
  const event = freshEvent(raw);
  if (event.pubkey !== owner || event.kind !== PRIVATE_CONTACT_SYNC_KIND || typeof event.content !== 'string'
    || event.content.length > MAX_CIPHERTEXT_BYTES || !Array.isArray(event.tags) || event.tags.length !== 2
    || !verifyEvent(event)) throw new Error('Invalid private contact event');
  const d = event.tags.find(tag => tag[0] === 'd');
  const t = event.tags.find(tag => tag[0] === 't');
  if (d?.length !== 2 || t?.length !== 2 || t[1] !== PRIVATE_CONTACT_SYNC_NAMESPACE
    || !new RegExp(`^${PRIVATE_CONTACT_SYNC_NAMESPACE}:[0-9a-f]{32}:[0-9a-f]{32}$`).test(d[1])) throw new Error('Invalid private contact namespace');
  await assertSigner(signer, owner);
  if (!signer.nip44Decrypt) throw new Error('This signer does not support private contact decryption');
  const plaintext = await signer.nip44Decrypt(owner, event.content);
  await assertSigner(signer, owner);
  if (bytes(plaintext) > MAX_PLAINTEXT_BYTES) throw new Error('Private contact document is too large');
  const document: unknown = JSON.parse(plaintext);
  validatePrivateContactDocument(document, owner);
  if (d[1] !== documentTags(document)[0][1]) throw new Error('Private contact address mismatch');
  return cloneDocument(document);
}
