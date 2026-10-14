import { createPrivateContactSync, editPrivateContact } from '../src/privateContactSyncV2';
import { describe, expect, it } from 'vitest';
import fixture from '../../fixtures/private-contact-facts.json';
import { buildPrivateContactFact, parsePrivateContactFact, privateContactFacts, privateContactDocumentsFromFacts, storePrivateContactFacts, restorePrivateContactFacts } from '../src/privateContactFacts';

describe('private contact facts shared with Rust', () => {
  it('uses the exact canonical unsigned records and hashes', () => {
    for (const entry of fixture.cases) {
      expect(buildPrivateContactFact(fixture.owner, fixture.contact, entry.field as any, entry.register)).toEqual(entry.fact);
      expect(parsePrivateContactFact(entry.fact, fixture.owner)).toEqual({ contact: fixture.contact, field: entry.field, register: entry.register });
    }
  });
  it('round-trips clears and original field stamps without signing', () => {
    const docs = [{ version: 2 as const, owner: fixture.owner, contact: fixture.contact,
      fields: Object.fromEntries(fixture.cases.map(entry => [entry.field, entry.register])) }];
    const facts = privateContactFacts(docs);
    expect(privateContactDocumentsFromFacts(facts, fixture.owner)).toEqual(docs);
    for (const fact of facts) { expect(fact).not.toHaveProperty('sig'); expect(fact).not.toHaveProperty('pubkey'); }
  });
  it('rejects foreign accounts, changed content, signed envelopes, extra routing tags and invalid stamps', () => {
    const original = fixture.cases[0].fact;
    expect(() => parsePrivateContactFact(original, 'f'.repeat(64))).toThrow();
    for (const mutate of [
      (x: any) => { x.sig = '0'.repeat(128); },
      (x: any) => { x.draft.tags.push(['p', fixture.contact]); },
      (x: any) => { x.draft.content = 'leak'; },
      (x: any) => { x.id = '0'.repeat(64); },
      (x: any) => { x.draft.kind = 37368; },
    ]) { const changed = structuredClone(original); mutate(changed); expect(() => parsePrivateContactFact(changed, fixture.owner)).toThrow(); }
    expect(() => buildPrivateContactFact(fixture.owner, fixture.contact, 'note', { counter: Number.MAX_SAFE_INTEGER + 1, writer: '1'.repeat(32), value: 'x' })).toThrow();
  });
});

it('migrates offline pending edits and clears without changing clocks or writer across restart', () => {
  let state = createPrivateContactSync(fixture.owner, '1'.repeat(32));
  state = editPrivateContact(state, fixture.contact, { nickname: 'Tea friend', note: 'secret', favorite: true });
  state = editPrivateContact(state, fixture.contact, { nickname: null, favorite: false });
  const saved = storePrivateContactFacts(restorePrivateContactFacts(state, fixture.owner));
  expect(saved.version).toBe(3);
  expect(restorePrivateContactFacts(JSON.parse(JSON.stringify(saved)), fixture.owner)).toEqual(state);
  expect(() => restorePrivateContactFacts({ ...saved, clock: 0 }, fixture.owner)).toThrow();
  expect(() => restorePrivateContactFacts({ ...saved, facts: [...saved.facts, saved.facts[0]] }, fixture.owner)).toThrow();
  expect(() => restorePrivateContactFacts(saved, 'c'.repeat(64))).toThrow();
});
