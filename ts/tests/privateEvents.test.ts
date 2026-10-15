import { describe, expect, it } from 'vitest';
import { finalizeEvent, getEventHash, getPublicKey } from 'nostr-tools';
import { buildPrivateSetting, parsePrivateSetting, parsePrivateEvent } from '../src/privateEvents';
const key = new Uint8Array(32).fill(7), owner = getPublicKey(key), target = 'a'.repeat(64);
describe('ordinary unsigned Nostr events', () => {
  it('hashes only the standard Nostr event fields and rejects extra metadata', () => {
    const event = buildPrivateSetting(owner, 'chatDeletion', target, { id: target, deletedAt: 100 });
    expect(event.id).toBe(getEventHash(event));
    expect(Object.keys(event).sort()).toEqual(['content','created_at','id','kind','pubkey','tags']);
    expect(parsePrivateSetting(event, owner)).toEqual({ setting: 'chatDeletion', target, value: { id: target, deletedAt: 100 } });
    expect(() => parsePrivateSetting(event, target)).toThrow();
    expect(() => parsePrivateEvent({ ...event, sig: '0'.repeat(128) })).toThrow();
    expect(() => parsePrivateEvent({ ...event, content: 'changed' })).toThrow();
  });
  it('normalizes key order across languages and migration', () => {
    expect(buildPrivateSetting(owner, 'chatPin', target, { pinned: false, chatId: target, updatedAtMs: 1000 }))
      .toEqual(buildPrivateSetting(owner, 'chatPin', target, { updatedAtMs: 1000, chatId: target, pinned: false }));
  });
  it('verifies old signed events before dropping their signatures', () => {
    const signed = finalizeEvent({ kind: 10454, created_at: 100, tags: [], content: 'legacy' }, key);
    const migrated = parsePrivateEvent(signed, true);
    expect(migrated.id).toBe(signed.id);
    expect(migrated).not.toHaveProperty('sig');
    expect(() => parsePrivateEvent(signed)).toThrow();
    expect(() => parsePrivateEvent({ ...signed, sig: '0'.repeat(128) }, true)).toThrow();
  });
});
