/** Private preferences carried only by caller-authenticated encrypted sibling channels. */
export const PRIVATE_CONTACT_CONTROL_KIND = 10452;
export const PRIVATE_CONTACT_SYNC_VERSION = 2;
export const PRIVATE_CONTACT_MAX_NOTE_BYTES = 16_384;
export const PRIVATE_CONTACT_MAX_NICKNAME_BYTES = 320;
export const PRIVATE_CONTACT_MAX_DOCUMENT_BYTES = 24_576;
const FIELDS = ['favorite', 'muted', 'nickname', 'note'] as const;
const HEX_KEY = /^[0-9a-f]{64}$/;
const HEX_ID = /^[0-9a-f]{32}$/;
export type PrivateContactField = typeof FIELDS[number];
export type PrivateContactValue = boolean | string | null;
export type PrivateContactPatch = { favorite?: boolean; muted?: boolean; nickname?: string | null; note?: string | null };
export interface PrivateContactRegister { counter: number; writer: string; value: PrivateContactValue }
export type PrivateContactFields = Partial<Record<PrivateContactField, PrivateContactRegister>>;
export interface PrivateContactDocument { version: 2; owner: string; contact: string; fields: PrivateContactFields }
/** Persist atomically before projection or handoff to the sibling outbox. */
export interface PrivateContactSyncState {
  version: 2;
  owner: string;
  writer: string;
  clock: number;
  contacts: Record<string, PrivateContactFields>;
  pending: Record<string, PrivateContactDocument>;
}
export type PrivateContactControl =
  | { type: 'private-contact-sync'; v: 2; document: PrivateContactDocument }
  | { type: 'private-contact-sync-request'; v: 2; owner: string };

function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}
function key(value: unknown): asserts value is string {
  if (typeof value !== 'string' || !HEX_KEY.test(value)) throw new Error('Invalid contact account or public key');
}
function writer(value: unknown): asserts value is string {
  if (typeof value !== 'string' || !HEX_ID.test(value)) throw new Error('Invalid contact writer');
}
function counter(value: unknown): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) throw new Error('Invalid contact counter');
}
function bytes(value: string): number { return new TextEncoder().encode(value).length; }
function validateValue(field: PrivateContactField, value: unknown): void {
  if (field === 'favorite' || field === 'muted') {
    if (typeof value !== 'boolean') throw new Error('Contact flag must be boolean');
  } else if (value !== null && (typeof value !== 'string'
    || bytes(value) > (field === 'note' ? PRIVATE_CONTACT_MAX_NOTE_BYTES : PRIVATE_CONTACT_MAX_NICKNAME_BYTES))) {
    throw new Error('Invalid or oversized contact text');
  }
}
function validateFields(value: unknown, legacy = false): asserts value is PrivateContactFields {
  if (!object(value) || Object.keys(value).some(field => !FIELDS.includes(field as PrivateContactField) || (legacy && field === 'muted'))) {
    throw new Error('Invalid contact fields');
  }
  for (const field of FIELDS) {
    const register = value[field];
    if (register === undefined) continue;
    if (!object(register)) throw new Error('Invalid contact register');
    counter(register.counter); writer(register.writer); validateValue(field, register.value);
  }
}
function cloneFields(fields: PrivateContactFields): PrivateContactFields {
  return Object.fromEntries(FIELDS.filter(field => fields[field]).map(field => {
    const register = fields[field]!;
    return [field, { counter: register.counter, writer: register.writer, value: register.value }];
  }));
}
export function validatePrivateContactDocument(value: unknown, expectedOwner: string): asserts value is PrivateContactDocument {
  key(expectedOwner);
  if (!object(value) || value.version !== 2 || value.owner !== expectedOwner) throw new Error('Contact account or version mismatch');
  key(value.contact); validateFields(value.fields);
  if (!Object.keys(value.fields).length || bytes(JSON.stringify(value)) > PRIVATE_CONTACT_MAX_DOCUMENT_BYTES) throw new Error('Invalid contact document size');
}
function cloneDocument(document: PrivateContactDocument): PrivateContactDocument {
  return { version: 2, owner: document.owner, contact: document.contact, fields: cloneFields(document.fields) };
}
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
export function createPrivateContactSync(owner: string, replica: string): PrivateContactSyncState {
  key(owner); writer(replica);
  return { version: 2, owner, writer: replica, clock: 0, contacts: {}, pending: {} };
}
export function restorePrivateContactSync(value: unknown, expectedOwner: string): PrivateContactSyncState {
  if (!object(value) || value.version !== 2 || value.owner !== expectedOwner || !object(value.contacts) || !object(value.pending)) throw new Error('Invalid saved contact state');
  writer(value.writer); counter(value.clock);
  let state = createPrivateContactSync(expectedOwner, value.writer);
  state.clock = value.clock;
  for (const [contact, fields] of Object.entries(value.contacts)) {
    key(contact); validateFields(fields);
    state.contacts[contact] = cloneFields(fields);
  }
  for (const [contact, document] of Object.entries(value.pending)) {
    validatePrivateContactDocument(document, expectedOwner);
    if (document.contact !== contact) throw new Error('Pending contact mismatch');
    state = mergePrivateContactDocument(state, document);
    state.pending[contact] = cloneDocument(document);
  }
  if (Object.values(state.contacts).some(fields => Object.values(fields).some(register => register.counter > (value.clock as number)))) throw new Error('Saved contact clock is behind its data');
  return state;
}
/** Import local V1 data once. Old ciphertext/relay ACKs are deliberately never restored. */
export function migratePrivateContactSync(value: unknown, expectedOwner: string): PrivateContactSyncState {
  if (object(value) && value.version === 2) return restorePrivateContactSync(value, expectedOwner);
  if (!object(value) || value.version !== 1 || value.owner !== expectedOwner || !object(value.contacts) || !object(value.records)) throw new Error('Invalid legacy contact state');
  writer(value.writer); counter(value.clock);
  let state = createPrivateContactSync(expectedOwner, value.writer);
  state.clock = value.clock;
  for (const [contact, fields] of Object.entries(value.contacts)) {
    key(contact); validateFields(fields, true);
    state.contacts[contact] = cloneFields(fields);
  }
  for (const [contact, record] of Object.entries(value.records)) {
    if (!object(record) || !object(record.document) || record.document.version !== 1 || record.document.owner !== expectedOwner
      || record.document.contact !== contact || record.document.writer !== state.writer) throw new Error('Invalid legacy contact record');
    writer(record.document.record_id); validateFields(record.document.fields, true);
    state = mergePrivateContactDocument(state, { version: 2, owner: expectedOwner, contact, fields: record.document.fields });
  }
  if (Object.values(state.contacts).some(fields => Object.values(fields).some(register => register.counter > (value.clock as number)))) throw new Error('Legacy contact clock is behind its data');
  return queuePrivateContactSnapshot(state);
}
export function privateContactValues(state: PrivateContactSyncState, contact: string): Required<PrivateContactPatch> {
  const fields = state.contacts[contact];
  return { favorite: fields?.favorite?.value === true, muted: fields?.muted?.value === true,
    nickname: typeof fields?.nickname?.value === 'string' ? fields.nickname.value : null,
    note: typeof fields?.note?.value === 'string' ? fields.note.value : null };
}
export function privateContactDocuments(state: PrivateContactSyncState): PrivateContactDocument[] {
  return Object.entries(state.contacts).filter(([, fields]) => Object.keys(fields).length).map(([contact, fields]) => ({ version: 2, owner: state.owner, contact, fields: cloneFields(fields) }));
}
/** Caller must authenticate the account AND active sibling device before calling. This does not authorize controls. */
export function mergePrivateContactDocument(state: PrivateContactSyncState, document: PrivateContactDocument): PrivateContactSyncState {
  validatePrivateContactDocument(document, state.owner);
  const fields = mergePrivateContactFields(state.contacts[document.contact] ?? {}, document.fields);
  const clock = Math.max(state.clock, ...Object.values(document.fields).map(register => register.counter));
  return { ...state, clock, contacts: { ...state.contacts, [document.contact]: fields } };
}
function applyPatch(state: PrivateContactSyncState, contact: string, patch: PrivateContactPatch, seed: boolean): PrivateContactSyncState {
  key(contact);
  if (!object(patch) || Object.keys(patch).some(field => !FIELDS.includes(field as PrivateContactField))) throw new Error('Invalid contact patch');
  const fields: PrivateContactFields = {};
  for (const field of FIELDS) {
    const value = patch[field];
    if (value === undefined) continue;
    validateValue(field, value);
    const known = state.contacts[contact]?.[field];
    if (seed && (known || value === false || value === null || value === '')) continue;
    if (!seed && known?.value === value) continue;
    const next = seed ? 0 : state.clock + 1;
    counter(next);
    fields[field] = { counter: next, writer: state.writer, value };
  }
  if (!Object.keys(fields).length) return state;
  const document: PrivateContactDocument = { version: 2, owner: state.owner, contact, fields: mergePrivateContactFields(state.contacts[contact] ?? {}, fields) };
  return { ...mergePrivateContactDocument(state, document), pending: { ...state.pending, [contact]: document } };
}
export function editPrivateContact(state: PrivateContactSyncState, contact: string, patch: PrivateContactPatch): PrivateContactSyncState {
  return applyPatch(state, contact, patch, false);
}
export function seedPrivateContact(state: PrivateContactSyncState, contact: string, patch: PrivateContactPatch): PrivateContactSyncState {
  return applyPatch(state, contact, patch, true);
}
/** Queue current merged registers for pairing/recovery without changing any field clocks. */
export function queuePrivateContactSnapshot(state: PrivateContactSyncState): PrivateContactSyncState {
  return { ...state, pending: { ...state.pending, ...Object.fromEntries(privateContactDocuments(state).map(document => [document.contact, document])) } };
}
export function pendingPrivateContacts(state: PrivateContactSyncState): PrivateContactDocument[] {
  return Object.values(state.pending).map(cloneDocument);
}
/** True handoff means the caller's durable sibling outbox owns every intended recipient's retry. */
export function acknowledgePrivateContactDocument(state: PrivateContactSyncState, document: PrivateContactDocument): PrivateContactSyncState {
  validatePrivateContactDocument(document, state.owner);
  const pending = state.pending[document.contact];
  if (!pending || JSON.stringify(cloneDocument(pending)) !== JSON.stringify(cloneDocument(document))) return state;
  const next = { ...state.pending };
  delete next[document.contact];
  return { ...state, pending: next };
}
export function buildPrivateContactControl(document: PrivateContactDocument): PrivateContactControl {
  validatePrivateContactDocument(document, document.owner);
  return { type: 'private-contact-sync', v: 2, document: cloneDocument(document) };
}
export function buildPrivateContactRequest(owner: string): PrivateContactControl {
  key(owner);
  return { type: 'private-contact-sync-request', v: 2, owner };
}
/** Parse decrypted payload only AFTER caller authentication; no signatures or decryption happen here. */
export function parsePrivateContactControl(value: unknown, expectedOwner: string): PrivateContactControl {
  key(expectedOwner);
  if (!object(value) || value.v !== 2) throw new Error('Unsupported private contact control');
  const allowed = value.type === 'private-contact-sync' ? ['type', 'v', 'document'] : ['type', 'v', 'owner'];
  if (Object.keys(value).length !== 3 || Object.keys(value).some(field => !allowed.includes(field))) throw new Error('Ambiguous private contact control');
  if (value.type === 'private-contact-sync') {
    validatePrivateContactDocument(value.document, expectedOwner);
    return buildPrivateContactControl(value.document);
  }
  if (value.type === 'private-contact-sync-request' && value.owner === expectedOwner) return buildPrivateContactRequest(expectedOwner);
  throw new Error('Private contact control account or type mismatch');
}
