import { afterEach, describe, expect, it, vi } from 'vitest';
import { finalizeEvent, getPublicKey, nip44 } from 'nostr-tools';
import { createPrivateContactSync, editPrivateContact, pendingPrivateContacts, privateContactValues } from '../src/privateContactSync';
import { createPrivateContactSyncController } from '../src/privateContactSyncController';

const secret = new Uint8Array(32).fill(3), owner = getPublicKey(secret), contact = 'b'.repeat(64);
const writer = '1'.repeat(32), recordId = 'a'.repeat(32);
const signer = {
  getPublicKey: () => owner,
  signEvent: (draft: Parameters<typeof finalizeEvent>[0]) => finalizeEvent(draft, secret),
  nip44Encrypt: (peer: string, value: string) => nip44.v2.encrypt(value, nip44.v2.utils.getConversationKey(secret, peer)),
  nip44Decrypt: (peer: string, value: string) => nip44.v2.decrypt(value, nip44.v2.utils.getConversationKey(secret, peer)),
};
const deferred = <T>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; };
afterEach(() => vi.useRealTimers());

describe('private contact sync controller', () => {
  it('persists edits and prepared ciphertext before sending, then retains failed sends across restart', async () => {
    let durable = createPrivateContactSync(owner, writer);
    const publish = vi.fn(async event => {
      expect(durable.records[contact].event).toEqual(event);
      return false;
    });
    const first = createPrivateContactSyncController({ state: durable, save: state => { durable = structuredClone(state); }, signer, publish, newId: () => recordId, now: () => 100, debounceMs: 60_000 });
    await first.edit(contact, { nickname: 'Private nickname' });
    expect(privateContactValues(durable, contact).nickname).toBe('Private nickname');
    await first.flush();
    expect(pendingPrivateContacts(durable)).toHaveLength(1);
    const event = durable.records[contact].event;
    first.stop();
    const secondPublish = vi.fn(async value => { expect(value).toEqual(event); return true; });
    const second = createPrivateContactSyncController({ state: durable, save: state => { durable = structuredClone(state); }, signer, publish: secondPublish, now: () => 105, debounceMs: 60_000 });
    await second.flush();
    expect(secondPublish).toHaveBeenCalledOnce();
    expect(pendingPrivateContacts(durable)).toHaveLength(0);
    second.stop();
  });

  it('does not let an old acknowledgement drop an edit made while the request was in flight', async () => {
    const ack = deferred<boolean>(), sent = deferred<void>();
    let durable = editPrivateContact(createPrivateContactSync(owner, writer), contact, { favorite: true }, recordId);
    const controller = createPrivateContactSyncController({ state: durable, save: state => { durable = structuredClone(state); }, signer, publish: async () => { sent.resolve(); return ack.promise; }, now: () => 100, debounceMs: 60_000 });
    const sending = controller.flush();
    await sent.promise;
    await controller.edit(contact, { favorite: false });
    ack.resolve(true);
    await sending;
    expect(privateContactValues(durable, contact).favorite).toBe(false);
    expect(pendingPrivateContacts(durable)).toHaveLength(1);
    controller.stop();
  });

  it('does not project or publish an edit if durable storage fails', async () => {
    const change = vi.fn(), publish = vi.fn(async () => true);
    const controller = createPrivateContactSyncController({ state: createPrivateContactSync(owner, writer), save: () => { throw new Error('Full'); }, onChange: change, signer, publish, newId: () => recordId });
    await expect(controller.edit(contact, { favorite: true })).rejects.toThrow('Full');
    expect(privateContactValues(controller.getState(), contact).favorite).toBe(false);
    expect(change).not.toHaveBeenCalled();
    expect(publish).not.toHaveBeenCalled();
    controller.stop();
  });

  it('fences delayed encryption when the account stops', async () => {
    const encryption = deferred<string>(), started = deferred<void>();
    let durable = editPrivateContact(createPrivateContactSync(owner, writer), contact, { note: 'Private' }, recordId);
    const publish = vi.fn(async () => true), change = vi.fn();
    const controller = createPrivateContactSyncController({ state: durable, save: state => { durable = state; }, onChange: change,
      signer: { ...signer, nip44Encrypt: async (peer, value) => { started.resolve(); await encryption.promise; return signer.nip44Encrypt(peer, value); } }, publish, now: () => 100 });
    const pending = controller.flush();
    await started.promise;
    controller.stop();
    encryption.resolve('continue');
    await pending;
    expect(durable.records[contact].event).toBeNull();
    expect(publish).not.toHaveBeenCalled();
    expect(change).not.toHaveBeenCalled();
  });

  it('coalesces rapid edits and retries on a bounded timer without calling a missing signer', async () => {
    vi.useFakeTimers();
    const publish = vi.fn(async () => false);
    const controller = createPrivateContactSyncController({ state: createPrivateContactSync(owner, writer), save: () => {}, signer, publish, newId: () => recordId, now: () => 100, debounceMs: 250 });
    await controller.edit(contact, { nickname: 'A' });
    await controller.edit(contact, { nickname: 'B' });
    await vi.advanceTimersByTimeAsync(249);
    expect(publish).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(publish).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(999);
    expect(publish).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(1);
    expect(publish).toHaveBeenCalledTimes(2);
    controller.stop();
    const local = createPrivateContactSyncController({ state: createPrivateContactSync(owner, writer), save: () => {}, publish, newId: () => recordId });
    await local.edit(contact, { favorite: true });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(publish).toHaveBeenCalledTimes(2);
    expect(local.getStatus()).toBe('local-only');
    local.stop();
  });

  it('converges two independent devices without losing offline edits or reviving cleared values', async () => {
    let aState = createPrivateContactSync(owner, writer);
    let bState = createPrivateContactSync(owner, '2'.repeat(32));
    const events: Parameters<NonNullable<Parameters<typeof createPrivateContactSyncController>[0]['publish']>>[0][] = [];
    let time = 100;
    const a = createPrivateContactSyncController({ state: aState, save: state => { aState = structuredClone(state); }, signer, publish: async event => { events.push(event); return true; }, newId: () => recordId, now: () => time, debounceMs: 60_000 });
    const b = createPrivateContactSyncController({ state: bState, save: state => { bState = structuredClone(state); }, signer, publish: async event => { events.push(event); return true; }, newId: () => recordId, now: () => time, debounceMs: 60_000 });
    await a.edit(contact, { favorite: true });
    await b.edit(contact, { nickname: 'Sam', note: 'Met last spring' });
    await Promise.all([a.flush(), b.flush()]);
    for (const event of events.slice()) await Promise.all([a.receive(event), b.receive(event)]);
    expect(privateContactValues(aState, contact)).toEqual({ favorite: true, nickname: 'Sam', note: 'Met last spring' });
    expect(privateContactValues(aState, contact)).toEqual(privateContactValues(bState, contact));
    await b.edit(contact, { favorite: false, note: null });
    time++;
    await b.flush();
    await a.receive(events[2]);
    await a.receive(events[0]);
    expect(privateContactValues(aState, contact)).toEqual({ favorite: false, nickname: 'Sam', note: null });
    a.stop(); b.stop();
  });

  it('fences a late decrypt when its account stops and never mutates the next account', async () => {
    const encrypted = deferred<void>(), received = deferred<void>();
    let aState = editPrivateContact(createPrivateContactSync(owner, writer), contact, { note: 'Old account secret' }, recordId);
    let event: Parameters<NonNullable<Parameters<typeof createPrivateContactSyncController>[0]['publish']>>[0] | undefined;
    const a = createPrivateContactSyncController({ state: aState, save: state => { aState = state; }, signer, publish: async sent => { event = sent; return true; }, now: () => 100, debounceMs: 60_000 });
    await a.flush(); a.stop();
    const save = vi.fn(), change = vi.fn();
    const b = createPrivateContactSyncController({ state: createPrivateContactSync(owner, '2'.repeat(32)), save, onChange: change,
      signer: { ...signer, nip44Decrypt: async (peer, content) => { received.resolve(); await encrypted.promise; return signer.nip44Decrypt(peer, content); } } });
    const receiving = b.receive(event!);
    await received.promise;
    b.stop(); encrypted.resolve();
    await receiving;
    expect(save).not.toHaveBeenCalled();
    expect(change).not.toHaveBeenCalled();
  });

  it('remembers accepted event IDs across restart and waits for the initial read before claiming sync', async () => {
    const aState = editPrivateContact(createPrivateContactSync(owner, writer), contact, { note: 'Remember this privately' }, recordId);
    let event: Parameters<NonNullable<Parameters<typeof createPrivateContactSyncController>[0]['publish']>>[0] | undefined;
    const a = createPrivateContactSyncController({ state: aState, save: () => {}, signer, publish: async value => { event = value; return true; }, now: () => 100, debounceMs: 60_000 });
    await a.flush(); a.stop();
    let durable = createPrivateContactSync(owner, '2'.repeat(32));
    const decrypt = vi.fn(signer.nip44Decrypt);
    const b = createPrivateContactSyncController({ state: durable, save: state => { durable = structuredClone(state); }, signer: { ...signer, nip44Decrypt: decrypt }, publish: async () => true });
    expect(b.getStatus()).toBe('loading');
    await Promise.all([b.receive(event!), b.receive(event!)]);
    expect(decrypt).toHaveBeenCalledOnce();
    expect(b.getStatus()).toBe('loading');
    await b.markReady();
    expect(b.getStatus()).toBe('synced');
    b.stop();
    const restored = createPrivateContactSyncController({ state: durable, save: () => {}, signer: { ...signer, nip44Decrypt: decrypt }, publish: async () => true });
    await restored.receive(event!);
    expect(decrypt).toHaveBeenCalledOnce();
    await expect(restored.receive({ ...event!, pubkey: contact })).rejects.toThrow('account mismatch');
    restored.stop();
  });

  it('reloads under a shared lock so simultaneous tabs do not overwrite each other', async () => {
    let durable = createPrivateContactSync(owner, writer);
    let lock = Promise.resolve();
    const withLock = (run: () => Promise<void>) => { const next = lock.then(run); lock = next.catch(() => {}); return next; };
    const options = { state: structuredClone(durable), load: () => structuredClone(durable),
      save: (next: typeof durable) => { durable = structuredClone(next); }, withLock, newId: () => recordId };
    const a = createPrivateContactSyncController(options);
    const b = createPrivateContactSyncController(options);
    await Promise.all([a.edit(contact, { nickname: 'From tab A' }), b.edit(contact, { note: 'From tab B' })]);
    await a.refresh();
    expect(privateContactValues(durable, contact)).toEqual({ favorite: false, nickname: 'From tab A', note: 'From tab B' });
    expect(privateContactValues(a.getState(), contact)).toEqual(privateContactValues(b.getState(), contact));
    expect(durable.clock).toBe(2);
    a.stop(); b.stop();
  });

  it('saves local edits while the signer is waiting and discards the superseded ciphertext', async () => {
    const waiting = deferred<void>(), proceed = deferred<void>();
    let durable = editPrivateContact(createPrivateContactSync(owner, writer), contact, { note: 'Before prompt' }, recordId);
    const publish = vi.fn(async () => true);
    const controller = createPrivateContactSyncController({ state: durable, save: next => { durable = structuredClone(next); },
      signer: { ...signer, nip44Encrypt: async (peer, value) => { waiting.resolve(); await proceed.promise; return signer.nip44Encrypt(peer, value); } },
      publish, now: () => 100, debounceMs: 60_000 });
    const flush = controller.flush();
    await waiting.promise;
    await controller.edit(contact, { note: 'While waiting' });
    expect(privateContactValues(durable, contact).note).toBe('While waiting');
    proceed.resolve();
    await flush;
    expect(publish).not.toHaveBeenCalled();
    expect(durable.records[contact].event).toBeNull();
    expect(pendingPrivateContacts(durable)).toHaveLength(1);
    controller.stop();
  });
});
