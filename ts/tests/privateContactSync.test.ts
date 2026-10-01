import { describe, expect, it } from 'vitest';
import { finalizeEvent, getPublicKey, nip44 } from 'nostr-tools';
import fixture from '../../fixtures/private-contact-sync.json';
import {
  createPrivateContactSync, editPrivateContact, seedPrivateContact,
  mergePrivateContactDocument, privateContactValues, pendingPrivateContacts,
  preparePrivateContactEvent, acknowledgePrivateContactEvent, openPrivateContactEvent,
  privateContactSyncFilter, restorePrivateContactSync, stagePrivateContactDocument,
} from '../src/privateContactSync';

const key = new Uint8Array(32).fill(1);
const owner = getPublicKey(key);
const contact = 'b'.repeat(64);
const alice = '1'.repeat(32);
const bob = '2'.repeat(32);
const rid = 'a'.repeat(32);
const signer = {
  getPublicKey: () => owner,
  signEvent: (draft: Parameters<typeof finalizeEvent>[0]) => finalizeEvent(draft, key),
  nip44Encrypt: (peer: string, value: string) => nip44.v2.encrypt(value, nip44.v2.utils.getConversationKey(key, peer)),
  nip44Decrypt: (peer: string, value: string) => nip44.v2.decrypt(value, nip44.v2.utils.getConversationKey(key, peer)),
};

describe('private contact sync', () => {
  it('accepts the shared Rust/TypeScript fixture and portable saved state', async () => {
    let state = createPrivateContactSync(fixture.owner, '3'.repeat(32));
    for (const document of fixture.documents) state = mergePrivateContactDocument(state, document as never);
    expect(privateContactValues(state, fixture.contact)).toEqual(fixture.expected);
    expect(restorePrivateContactSync(fixture.state, fixture.owner)).toEqual(fixture.state);
    expect(await openPrivateContactEvent(fixture.event!, fixture.owner, signer)).toEqual(fixture.documents[2]);
  });
  it('merges unrelated offline edits and converges regardless of delivery order', () => {
    const a = editPrivateContact(createPrivateContactSync(owner, alice), contact, { favorite: true }, rid);
    const b = editPrivateContact(createPrivateContactSync(owner, bob), contact, { note: 'Met at the park', nickname: 'Sam' }, rid);
    const ab = mergePrivateContactDocument(a, pendingPrivateContacts(b)[0].document);
    const ba = mergePrivateContactDocument(b, pendingPrivateContacts(a)[0].document);
    expect(privateContactValues(ab, contact)).toEqual({ favorite: true, nickname: 'Sam', note: 'Met at the park' });
    expect(privateContactValues(ab, contact)).toEqual(privateContactValues(ba, contact));
    expect(mergePrivateContactDocument(ab, pendingPrivateContacts(b)[0].document)).toEqual(ab);
    expect(pendingPrivateContacts(ab)).toHaveLength(1);
  });

  it('retains clears through stale heads, concurrent edits, and one-time legacy imports', () => {
    const old = seedPrivateContact(createPrivateContactSync(owner, alice), contact, { favorite: true, nickname: 'Old' }, rid);
    const remote = mergePrivateContactDocument(createPrivateContactSync(owner, bob), pendingPrivateContacts(old)[0].document);
    const cleared = editPrivateContact(remote, contact, { favorite: false, nickname: null }, rid);
    let next = mergePrivateContactDocument(old, pendingPrivateContacts(cleared)[0].document);
    next = seedPrivateContact(next, contact, { favorite: true, nickname: 'Old' }, rid);
    expect(privateContactValues(next, contact)).toEqual({ favorite: false, nickname: null, note: null });
    expect(privateContactValues(mergePrivateContactDocument(cleared, pendingPrivateContacts(old)[0].document), contact).favorite).toBe(false);
    const again = editPrivateContact(next, contact, { favorite: false });
    expect(again).toBe(next);
  });

  it('uses a deterministic writer tie-break for concurrent edits to the same field', () => {
    const a = editPrivateContact(createPrivateContactSync(owner, alice), contact, { nickname: 'A' }, rid);
    const b = editPrivateContact(createPrivateContactSync(owner, bob), contact, { nickname: 'B' }, rid);
    const ab = mergePrivateContactDocument(a, pendingPrivateContacts(b)[0].document);
    const ba = mergePrivateContactDocument(b, pendingPrivateContacts(a)[0].document);
    expect(privateContactValues(ab, contact).nickname).toBe('B');
    expect(privateContactValues(ab, contact)).toEqual(privateContactValues(ba, contact));
    expect(editPrivateContact(ab, contact, { nickname: 'Chosen' }).clock).toBe(2);
  });

  it('keeps retries identical and only acknowledges the exact still-current edit', async () => {
    const initial = editPrivateContact(createPrivateContactSync(owner, alice), contact, { favorite: true }, rid);
    const prepared = await preparePrivateContactEvent(initial, contact, signer, 100);
    expect((await preparePrivateContactEvent(prepared.state, contact, signer, 100)).event).toEqual(prepared.event);
    const changed = editPrivateContact(prepared.state, contact, { nickname: 'New' });
    expect(pendingPrivateContacts(acknowledgePrivateContactEvent(changed, contact, prepared.event.id))).toHaveLength(1);
    expect(await preparePrivateContactEvent(changed, contact, signer, 100)).toMatchObject({ event: null, retryAt: 101 });
    const next = await preparePrivateContactEvent(changed, contact, signer, 101);
    expect(next.event!.created_at).toBe(101);
    expect(pendingPrivateContacts(acknowledgePrivateContactEvent(next.state, contact, next.event!.id))).toHaveLength(0);
    expect(privateContactValues(JSON.parse(JSON.stringify(next.state)), contact).nickname).toBe('New');
  });

  it('encrypts only to self, exposes no contact/value tags, validates before decrypting', async () => {
    const state = editPrivateContact(createPrivateContactSync(owner, alice), contact, { note: 'Private note' }, rid);
    const sealed = await preparePrivateContactEvent(state, contact, signer, 100);
    const event = sealed.event!;
    expect(event.tags).toEqual([['d', `nostr-social-memory/v1:${alice}:${rid}`], ['t', 'nostr-social-memory/v1']]);
    expect(JSON.stringify(event)).not.toContain(contact);
    expect(JSON.stringify(event)).not.toContain('Private note');
    expect(await openPrivateContactEvent(event, owner, signer)).toEqual(pendingPrivateContacts(state)[0].document);
    let decrypts = 0;
    const spy = { ...signer, nip44Decrypt: (...args: [string, string]) => { decrypts++; return signer.nip44Decrypt(...args); } };
    await expect(openPrivateContactEvent({ ...event, content: 'tampered' }, owner, spy)).rejects.toThrow();
    await expect(openPrivateContactEvent(event, contact, spy)).rejects.toThrow();
    expect(decrypts).toBe(0);
    expect(privateContactSyncFilter(owner)).toEqual({ kinds: [30078], authors: [owner], '#t': ['nostr-social-memory/v1'] });
  });

  it('rejects invalid fields, oversized notes, wrong owners and forged signer outputs', async () => {
    const state = createPrivateContactSync(owner, alice);
    expect(() => editPrivateContact(state, contact, { accepted_name: 'Bypass' } as never, rid)).toThrow();
    expect(() => editPrivateContact(state, contact, { note: 'x'.repeat(16_385) }, rid)).toThrow();
    const next = editPrivateContact(state, contact, { note: 'okay' }, rid);
    const doc = pendingPrivateContacts(next)[0].document;
    expect(() => mergePrivateContactDocument(state, { ...doc, owner: contact })).toThrow();
    expect(() => mergePrivateContactDocument(state, { ...doc, fields: { note: { value: 'bad', counter: 1, writer: 'invalid' } } })).toThrow();
    const evil = { ...signer, signEvent: (draft: Parameters<typeof finalizeEvent>[0]) => finalizeEvent({ ...draft, tags: [...draft.tags, ['p', contact]] }, key) };
    await expect(preparePrivateContactEvent(next, contact, evil, 100)).rejects.toThrow();
  });

  it('validates durable state and forwards authenticated sibling stamps without new edits', async () => {
    const sibling = editPrivateContact(createPrivateContactSync(owner, bob), contact, { nickname: 'Shared' }, rid);
    const staged = stagePrivateContactDocument(createPrivateContactSync(owner, alice), pendingPrivateContacts(sibling)[0].document, rid);
    expect(staged.clock).toBe(1);
    expect(staged.records[contact].document.fields.nickname?.writer).toBe(bob);
    const prepared = await preparePrivateContactEvent(staged, contact, signer, 100);
    expect(restorePrivateContactSync(JSON.parse(JSON.stringify(prepared.state)), owner)).toEqual(prepared.state);
    expect(() => restorePrivateContactSync(prepared.state, contact)).toThrow();
    expect(() => restorePrivateContactSync({ ...prepared.state, clock: 0 }, owner)).toThrow();
    expect(await openPrivateContactEvent(prepared.event!, owner, signer)).toEqual(staged.records[contact].document);
  });
});
