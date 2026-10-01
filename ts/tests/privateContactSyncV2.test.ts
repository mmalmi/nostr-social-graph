import { describe, it, expect } from 'vitest';
import fixture from '../../fixtures/private-contact-sync-v2.json';
import {
  createPrivateContactSync, editPrivateContact, seedPrivateContact, mergePrivateContactDocument,
  privateContactDocuments, privateContactValues, migratePrivateContactSync, restorePrivateContactSync,
  pendingPrivateContacts, acknowledgePrivateContactDocument, queuePrivateContactSnapshot,
  parsePrivateContactControl, buildPrivateContactControl, buildPrivateContactRequest,
} from '../src/privateContactSyncV2';
import { validatePrivateContactDocument as validateV1 } from '../src/privateContactSync';
const owner = fixture.owner, contact = fixture.contact, writer = '1'.repeat(32);

describe('private contact V2', () => {
  it('converges shared Rust/TS fixtures in either order and retains tombstones in snapshots', () => {
    for (const docs of [fixture.documents, [...fixture.documents].reverse()]) {
      let state = createPrivateContactSync(owner, writer);
      for (const raw of docs) {
        const control = parsePrivateContactControl({ type: 'private-contact-sync', v: 2, document: raw }, owner);
        if (control.type !== 'private-contact-sync') throw new Error('wrong fixture');
        state = mergePrivateContactDocument(state, control.document);
      }
      expect(privateContactValues(state, contact)).toEqual(fixture.expected);
      expect(pendingPrivateContacts(state)).toEqual([]);
      expect(privateContactDocuments(state)[0].fields.muted?.value).toBe(false);
      expect(restorePrivateContactSync(queuePrivateContactSnapshot(state), owner)).toEqual(queuePrivateContactSnapshot(state));
    }
  });
  it('migrates full local V1 state with original stamps and retires sealed transport permanently', () => {
    const state = migratePrivateContactSync(fixture.legacy_state, owner);
    expect(state.clock).toBe(fixture.legacy_state.clock);
    expect(state.contacts[contact]).toEqual(fixture.legacy_state.contacts[contact]);
    expect(pendingPrivateContacts(state)).toHaveLength(1);
    expect(JSON.stringify(state)).not.toContain('30078');
    expect(JSON.stringify(state)).not.toContain('received_event_ids');
    expect(migratePrivateContactSync(state, owner)).toEqual(state);
  });
  it('keeps independent concurrent fields and deterministically resolves same-field conflicts', () => {
    const a = editPrivateContact(createPrivateContactSync(owner, writer), contact, { nickname: 'A', muted: true });
    const b = editPrivateContact(createPrivateContactSync(owner, '2'.repeat(32)), contact, { note: 'B', nickname: 'B' });
    const left = mergePrivateContactDocument(a, privateContactDocuments(b)[0]);
    const right = mergePrivateContactDocument(b, privateContactDocuments(a)[0]);
    expect(left.contacts).toEqual(right.contacts);
    expect(privateContactValues(left, contact)).toEqual({ favorite: false, muted: true, nickname: 'B', note: 'B' });
  });
  it('old snapshots and legacy mute seeds cannot resurrect an explicit clear', () => {
    const old = seedPrivateContact(createPrivateContactSync(owner, writer), contact, { muted: true, note: 'remember' });
    let state = editPrivateContact(old, contact, { muted: false, note: null });
    state = mergePrivateContactDocument(state, privateContactDocuments(old)[0]);
    state = seedPrivateContact(state, contact, { muted: true, note: 'remember' });
    expect(privateContactValues(state, contact).muted).toBe(false);
    expect(privateContactValues(state, contact).note).toBe(null);
  });
  it('only dequeues the exact handoff revision', () => {
    const a = editPrivateContact(createPrivateContactSync(owner, writer), contact, { favorite: true });
    const pending = pendingPrivateContacts(a)[0];
    const b = editPrivateContact(a, contact, { favorite: false });
    expect(acknowledgePrivateContactDocument(b, pending)).toBe(b);
    expect(pendingPrivateContacts(acknowledgePrivateContactDocument(a, pending))).toEqual([]);
  });
  it('has a strict account/version boundary that the old publisher cannot accept', () => {
    const state = editPrivateContact(createPrivateContactSync(owner, writer), contact, { favorite: true });
    const document = privateContactDocuments(state)[0];
    expect(() => validateV1(document, owner)).toThrow();
    expect(() => parsePrivateContactControl({ ...buildPrivateContactControl(document), request: true }, owner)).toThrow();
    expect(() => parsePrivateContactControl({ ...buildPrivateContactRequest(owner), document }, owner)).toThrow();
    expect(() => parsePrivateContactControl({ type: 'private-contact-sync', v: 1, document }, owner)).toThrow();
    expect(() => parsePrivateContactControl(buildPrivateContactControl(document), 'f'.repeat(64))).toThrow();
    expect(parsePrivateContactControl(buildPrivateContactRequest(owner), owner)).toEqual(buildPrivateContactRequest(owner));
    expect(() => parsePrivateContactControl(buildPrivateContactRequest(owner), 'f'.repeat(64))).toThrow();
  });
  it('rejects invalid field types, clocks, excessive UTF8 text and corrupt persistence', () => {
    const empty = createPrivateContactSync(owner, writer);
    expect(() => editPrivateContact(empty, contact, { muted: 'yes' as never })).toThrow();
    expect(() => editPrivateContact(empty, contact, { nickname: '😀'.repeat(81) })).toThrow();
    expect(() => editPrivateContact(empty, contact, { note: 'x'.repeat(16_385) })).toThrow();
    expect(() => editPrivateContact({ ...empty, clock: Number.MAX_SAFE_INTEGER }, contact, { muted: true })).toThrow();
    const state = editPrivateContact(empty, contact, { muted: true });
    expect(() => restorePrivateContactSync({ ...state, clock: 0 }, owner)).toThrow();
  });
});
