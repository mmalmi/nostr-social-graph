import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPrivateContactSync, editPrivateContact, privateContactDocuments, privateContactValues, type PrivateContactSyncState } from '../src/privateContactSyncV2';
import { createPrivateContactSyncController } from '../src/privateContactSyncV2Controller';
const owner = 'a'.repeat(64), contact = 'b'.repeat(64), writer = '1'.repeat(32);
const fresh = () => createPrivateContactSync(owner, writer);
const deferred = <T>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; };
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('V2 durable sibling controller', () => {
  it('saves before projection/handoff and retains failed enqueue across restart', async () => {
    const calls: string[] = []; let saved = fresh();
    const controller = createPrivateContactSyncController({ state: saved, save: value => { saved = structuredClone(value); calls.push('save'); }, onChange: () => { calls.push('project'); }, send: async () => { calls.push('send'); return false; } });
    await controller.edit(contact, { note: 'offline' });
    await controller.flush();
    expect(calls.slice(0, 3)).toEqual(['save', 'project', 'send']);
    expect(controller.getStatus()).toBe('error');
    controller.stop();
    const send = vi.fn(async () => true);
    const restarted = createPrivateContactSyncController({ state: saved, save: value => { saved = value; }, send });
    await restarted.flush();
    expect(send).toHaveBeenCalledOnce();
    expect(saved.pending).toEqual({});
    expect(restarted.getStatus()).toBe('ready');
    restarted.stop();
  });
  it('keeps newer local edits while the durable handoff awaits acceptance', async () => {
    const handoff = deferred<boolean>();
    const controller = createPrivateContactSyncController({ state: fresh(), save: () => {}, send: () => handoff.promise });
    await controller.edit(contact, { favorite: true });
    const flush = controller.flush();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    await controller.edit(contact, { favorite: false });
    handoff.resolve(true); await flush;
    expect(privateContactValues(controller.getState(), contact).favorite).toBe(false);
    expect(controller.getState().pending[contact]?.fields.favorite?.value).toBe(false);
    controller.stop();
  });
  it('does not ACK or project an inbound document until durable storage succeeds and never echoes', async () => {
    const document = privateContactDocuments(editPrivateContact(fresh(), contact, { muted: true }))[0];
    const send = vi.fn(async () => true), change = vi.fn();
    const controller = createPrivateContactSyncController({ state: fresh(), save: () => { throw new Error('disk full'); }, send, onChange: change });
    await expect(controller.mergeTrusted(document)).rejects.toThrow('disk full');
    expect(change).not.toHaveBeenCalled(); expect(send).not.toHaveBeenCalled();
    controller.stop();
    const good = createPrivateContactSyncController({ state: fresh(), save: () => {}, send });
    await good.mergeTrusted(document); await good.flush();
    expect(send).not.toHaveBeenCalled(); expect(good.getState().pending).toEqual({});
    await good.queueSnapshot(); await good.flush(); expect(send).toHaveBeenCalledOnce();
    good.stop();
  });
  it('rejects stopped inbound handlers and fences a late handoff acknowledgement', async () => {
    const handoff = deferred<boolean>();
    const controller = createPrivateContactSyncController({ state: fresh(), save: () => {}, send: () => handoff.promise });
    await controller.edit(contact, { note: 'retain' });
    const doc = privateContactDocuments(controller.getState())[0];
    const flushing = controller.flush();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    controller.stop(); handoff.resolve(true); await flushing;
    expect(controller.getState().pending[contact]).toBeDefined();
    await expect(controller.mergeTrusted(doc)).rejects.toThrow('stopped');
  });
  it('reloads under the same lock so concurrent tabs preserve independent fields', async () => {
    let saved = fresh(); let chain = Promise.resolve();
    const options = { state: saved, save: (value: PrivateContactSyncState) => { saved = structuredClone(value); }, load: () => structuredClone(saved), withLock: (run: () => Promise<void>) => { const next = chain.then(run); chain = next.catch(() => {}); return next; } };
    const a = createPrivateContactSyncController(options), b = createPrivateContactSyncController(options);
    await Promise.all([a.edit(contact, { nickname: 'one' }), b.edit(contact, { note: 'two' })]);
    await a.refresh(); expect(privateContactValues(a.getState(), contact)).toMatchObject({ nickname: 'one', note: 'two' });
    expect(a.getStatus()).toBe('local-only'); a.stop(); b.stop();
  });
});
