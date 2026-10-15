/** Standard Nostr events without signatures. Admit only through authenticated private transport. */
import { sha256 } from '@noble/hashes/sha2.js';
import { bytesToHex } from '@noble/hashes/utils.js';
import { getEventHash, type UnsignedEvent } from 'nostr-tools';
import { buildFactOpDraft, type FactEventDraft } from './factEvents';
import { validatePrivateContactDocument, mergePrivateContactFields,
  restorePrivateContactSync, migratePrivateContactSync, privateContactDocuments, type PrivateContactSyncState, type PrivateContactDocument, type PrivateContactField, type PrivateContactRegister } from './privateContactSyncV2';

export const PRIVATE_CONTACT_FACTS_VERSION = 2;
export const PRIVATE_CONTACT_FACT_CONTROL_KIND = 10456;
export type PrivateContactFact = UnsignedEvent & { id: string };
const fields = ['favorite', 'muted', 'nickname', 'note'] as const;
const object = (x: unknown): x is Record<string, any> => !!x && typeof x === 'object' && !Array.isArray(x);
const exact = (x: Record<string, unknown>, keys: string[]) => Object.keys(x).length === keys.length && keys.every(key => Object.prototype.hasOwnProperty.call(x, key));
const hash = (owner: string, draft: FactEventDraft) => bytesToHex(sha256(new TextEncoder().encode(
  JSON.stringify(['iris-private-contact-fact', 1, owner, draft.kind, draft.tags, draft.content]))));

export function buildPrivateContactFact(owner: string, contact: string, field: PrivateContactField, register: PrivateContactRegister): PrivateContactFact {
  validatePrivateContactDocument({ version: 2, owner, contact, fields: { [field]: register } }, owner);
  const draft = buildFactOpDraft(contact, [
    { predicate: 'type', values: ['private_contact'] }, { predicate: 'schema', values: ['1'] },
    { predicate: 'counter', values: [String(register.counter)] }, { predicate: 'writer', values: [register.writer] },
    { predicate: field, values: [JSON.stringify(register.value)] },
  ]);
  // Field counters determine order. A fixed timestamp makes repair IDs stable
  // when a legacy register is independently migrated on several devices.
  const event = { ...draft, pubkey: owner, created_at: 0 };
  return { ...event, id: getEventHash(event) };
}

/** Call only after authenticating the account and active sibling. A claimed writer is not a signature. */
export function parsePrivateContactFact(value: unknown, owner: string): { contact: string; field: PrivateContactField; register: PrivateContactRegister } {
  if (!object(value) || !exact(value, ['id', 'pubkey', 'created_at', 'kind', 'content', 'tags']) || value.pubkey !== owner ||
    value.created_at !== 0 || value.kind !== 7368 || value.content !== '' ||
    !Array.isArray(value.tags) || value.tags.length !== 6 || !value.tags.every((tag: unknown) => Array.isArray(tag) && tag.every(x => typeof x === 'string'))) throw new Error('Invalid private contact fact');
  const tags = value.tags as string[][];
  const subject = tags.find(tag => tag[0] === 'i' && tag[2] === 'subject')?.[1];
  const fieldTag = tags.find(tag => fields.includes(tag[0] as PrivateContactField));
  if (!subject || !fieldTag || tags.find(tag => tag[0] === 'type')?.[1] !== 'private_contact' || tags.find(tag => tag[0] === 'schema')?.[1] !== '1') throw new Error('Invalid private contact schema');
  const field = fieldTag[0] as PrivateContactField;
  const register = { counter: Number(tags.find(tag => tag[0] === 'counter')?.[1]), writer: tags.find(tag => tag[0] === 'writer')?.[1] ?? '', value: JSON.parse(fieldTag[1]) };
  const canonical = buildPrivateContactFact(owner, subject, field, register);
  if (value.id !== canonical.id || JSON.stringify(tags) !== JSON.stringify(canonical.tags)) throw new Error('Noncanonical private contact fact');
  return { contact: subject, field, register };
}

export function privateContactFacts(documents: PrivateContactDocument[]): PrivateContactFact[] {
  return documents.flatMap(document => {
    validatePrivateContactDocument(document, document.owner);
    return fields.flatMap(field => document.fields[field] ? [buildPrivateContactFact(document.owner, document.contact, field, document.fields[field]!)] : []);
  }).sort((a, b) => a.id.localeCompare(b.id));
}
export function privateContactDocumentsFromFacts(facts: PrivateContactFact[], owner: string): PrivateContactDocument[] {
  const documents = new Map<string, PrivateContactDocument>();
  for (const fact of facts) {
    const { contact, field, register } = parsePrivateContactFact(fact, owner);
    const previous = documents.get(contact);
    documents.set(contact, { version: 2, owner, contact, fields: mergePrivateContactFields(previous?.fields ?? {}, { [field]: register }) });
  }
  return [...documents.values()].sort((a, b) => a.contact.localeCompare(b.contact));
}

/** Facts are the durable authority; V2 remains an in-memory/old-peer compatibility view. */
export interface PrivateContactFactState {
  version: 4; owner: string; writer: string; clock: number;
  facts: PrivateContactFact[]; pending: PrivateContactFact[];
}
export function storePrivateContactFacts(state: PrivateContactSyncState): PrivateContactFactState {
  restorePrivateContactSync(state, state.owner);
  return { version: 4, owner: state.owner, writer: state.writer, clock: state.clock,
    facts: privateContactFacts(privateContactDocuments(state)), pending: privateContactFacts(Object.values(state.pending)) };
}
export function restorePrivateContactFacts(value: unknown, owner: string): PrivateContactSyncState {
  if (!object(value) || ![3, 4].includes(value.version)) return migratePrivateContactSync(value, owner);
  if (!exact(value, ['version', 'owner', 'writer', 'clock', 'facts', 'pending']) || value.owner !== owner ||
    !Array.isArray(value.facts) || !Array.isArray(value.pending)) throw new Error('Invalid private fact state');
  const documents = (values: unknown[]) => {
    const facts = values.map(value => {
      if (!object(value)) throw new Error('Invalid private contact fact');
      if ('draft' in value) {
        if (!exact(value, ['v', 'owner', 'id', 'draft']) || value.v !== 1 || value.owner !== owner ||
          !object(value.draft) || !exact(value.draft, ['kind', 'content', 'tags']) || value.id !== hash(owner, value.draft as FactEventDraft)) throw new Error('Invalid legacy private contact fact');
        const event = { ...value.draft, pubkey: owner, created_at: 0 } as UnsignedEvent;
        return { ...event, id: getEventHash(event) };
      }
      return value as PrivateContactFact;
    });
    const seen = new Set<string>();
    for (const fact of facts) {
      const parsed = parsePrivateContactFact(fact, owner), key = parsed.contact + ':' + parsed.field;
      if (seen.has(key)) throw new Error('Duplicate private fact field');
      seen.add(key);
    }
    return privateContactDocumentsFromFacts(facts, owner);
  };
  return restorePrivateContactSync({ version: 2, owner, writer: value.writer, clock: value.clock,
    contacts: Object.fromEntries(documents(value.facts).map(d => [d.contact, d.fields])),
    pending: Object.fromEntries(documents(value.pending).map(d => [d.contact, d])) }, owner);
}
