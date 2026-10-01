import type { Event } from 'nostr-tools';
import {
  acknowledgePrivateContactEvent, editPrivateContact, mergePrivateContactDocument,
  openPrivateContactEvent, pendingPrivateContacts, preparePrivateContactEvent,
  seedPrivateContact, stagePrivateContactDocument, rememberPrivateContactEvent,
  type PrivateContactDocument, type PrivateContactPatch,
  type PrivateContactSigner, type PrivateContactSyncState,
} from './privateContactSync';

export type PrivateContactSyncStatus = 'local-only' | 'loading' | 'pending' | 'syncing' | 'synced' | 'error';
export interface PrivateContactSyncChange {
  source: 'local' | 'remote' | 'sibling' | 'prepared' | 'ack' | 'refresh';
  contact: string;
}
export interface PrivateContactSyncControllerOptions {
  /** Already restored and validated by restorePrivateContactSync. */
  state: PrivateContactSyncState;
  /** Must atomically persist the entire state; reject if persistence fails. */
  save(state: PrivateContactSyncState): void | Promise<void>;
  /** Pair these to share one replica safely across tabs/processes. Reload occurs inside the lock. */
  load?(): PrivateContactSyncState | null | Promise<PrivateContactSyncState | null>;
  withLock?(run: () => Promise<void>): Promise<void>;
  signer?: PrivateContactSigner;
  /** True means explicit remote acceptance, never a queued or optimistic echo. */
  publish?(event: Event): Promise<boolean>;
  onChange?(state: PrivateContactSyncState, change: PrivateContactSyncChange): void;
  onStatus?(status: PrivateContactSyncStatus): void;
  onError?(error: unknown): void;
  newId?(): string;
  now?(): number;
  debounceMs?: number;
  initialReadComplete?: boolean;
}

/** Shared durable queue; app adapters own account lifecycle and transport subscriptions. */
export function createPrivateContactSyncController(options: PrivateContactSyncControllerOptions) {
  if (!!options.load !== !!options.withLock) throw new Error('Shared contact persistence needs both a lock and a fresh load');
  let state = options.state;
  let stopped = false;
  let writes: Promise<void> = Promise.resolve();
  let sending: Promise<void> | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let failures = 0;
  let readComplete = options.initialReadComplete ?? false;
  let readFailed = false;
  let reads: Promise<void> = Promise.resolve();
  const receiving = new Map<string, Promise<void>>();
  const canPublish = !!(options.signer && options.publish);
  let status: PrivateContactSyncStatus = !canPublish ? 'local-only'
    : pendingPrivateContacts(state).length ? 'pending' : readComplete ? 'synced' : 'loading';
  const now = options.now ?? (() => Math.floor(Date.now() / 1000));
  const newId = options.newId ?? (() => {
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  });

  function setStatus(next: PrivateContactSyncStatus) {
    if (stopped || status === next) return;
    status = next;
    options.onStatus?.(next);
  }
  function report(error: unknown) {
    if (stopped) return;
    setStatus('error');
    options.onError?.(error);
  }
  function update(updater: (current: PrivateContactSyncState) => PrivateContactSyncState | Promise<PrivateContactSyncState>, change: PrivateContactSyncChange): Promise<void> {
    const run = async () => {
      if (stopped) return;
      const latest = await options.load?.() ?? state;
      if (latest.owner !== state.owner) throw new Error('Private contact account mismatch');
      const next = await updater(latest);
      if (stopped) return;
      if (next !== latest && JSON.stringify(next) !== JSON.stringify(latest)) await options.save(next);
      if (stopped) return;
      if (JSON.stringify(next) === JSON.stringify(state)) return;
      state = next;
      options.onChange?.(state, change);
    };
    const operation = writes.then(() => options.withLock ? options.withLock(run) : run());
    writes = operation.catch(() => {});
    return operation;
  }
  function schedule(delay = options.debounceMs ?? 300) {
    if (stopped || !canPublish || !pendingPrivateContacts(state).length) return;
    clearTimeout(timer);
    timer = setTimeout(() => { timer = undefined; void flush(); }, Math.max(0, delay));
  }
  function pending() {
    setStatus(canPublish ? pendingPrivateContacts(state).length ? 'pending' : readFailed ? 'error' : readComplete ? 'synced' : 'loading' : 'local-only');
    schedule();
  }
  async function patch(contact: string, values: PrivateContactPatch, seed: boolean) {
    try {
      await update(current => (seed ? seedPrivateContact : editPrivateContact)(current, contact, values, newId()), { source: 'local', contact });
      pending();
    } catch (error) { report(error); throw error; }
  }
  function knownEvent(id: string) {
    return state.received_event_ids.includes(id) || Object.values(state.records).some(record => record.event?.id === id);
  }
  function receive(event: Event): Promise<void> {
    if (!options.signer || stopped) return Promise.resolve();
    if (event.pubkey !== state.owner) return Promise.reject(new Error('Private contact account mismatch'));
    if (knownEvent(event.id)) return Promise.resolve();
    const inflight = receiving.get(event.id);
    if (inflight) return inflight;
    // Decode separately from writes; duplicate relays cannot trigger parallel signer prompts.
    const operation = reads.then(async () => {
      if (stopped || knownEvent(event.id)) return;
      try {
        const document = await openPrivateContactEvent(event, state.owner, options.signer!);
        if (stopped) return;
        await update(current => rememberPrivateContactEvent(mergePrivateContactDocument(current, document), event.id), { source: 'remote', contact: document.contact });
      } catch (error) {
        if (!stopped) { readFailed = true; report(error); throw error; }
      }
    });
    const result = operation.finally(() => { receiving.delete(event.id); });
    receiving.set(event.id, result);
    reads = result.catch(() => {});
    return result;
  }
  async function mergeTrusted(document: PrivateContactDocument) {
    await update(current => stagePrivateContactDocument(current, document, newId()), { source: 'sibling', contact: document.contact });
    pending();
  }
  async function drain() {
    await writes;
    if (stopped || !options.signer || !options.publish) return;
    let retryDelay: number | undefined;
    setStatus('syncing');
    try {
      // Capture one bounded batch. Edits arriving during a send wait for the next pass.
      for (const record of pendingPrivateContacts(state)) {
        if (stopped) return;
        const contact = record.document.contact;
        await update(current => current, { source: 'refresh', contact });
        if (stopped) return;
        const captured = state;
        // Never hold the mutation queue or cross-tab lock while a signer awaits approval.
        const prepared = await preparePrivateContactEvent(captured, contact, options.signer!, now());
        if (stopped) return;
        if (!prepared.event) {
          if (prepared.retryAt) retryDelay = Math.max(1, (prepared.retryAt - now()) * 1000);
          continue;
        }
        let event: Event | undefined;
        await update(current => {
          // A local edit or another tab may have changed this record while the signer was open.
          if (JSON.stringify(current.records[contact]) !== JSON.stringify(captured.records[contact])) return current;
          event = prepared.event!;
          return { ...current, records: { ...current.records, [contact]: prepared.state.records[contact] } };
        }, { source: 'prepared', contact });
        if (stopped) return;
        if (!event) continue;
        // This exact ciphertext is durable before transport can observe it.
        const publishedEvent = event;
        const accepted = await options.publish(publishedEvent);
        if (stopped) return;
        if (!accepted) throw new Error('Private contact sync is waiting for server confirmation');
        await update(current => acknowledgePrivateContactEvent(current, contact, publishedEvent.id), { source: 'ack', contact });
      }
      failures = 0;
      pending();
      if (retryDelay !== undefined) schedule(retryDelay);
    } catch (error) {
      report(error);
      schedule(Math.min(30_000, 1000 * 2 ** Math.min(failures++, 5)));
    }
  }
  function flush(): Promise<void> {
    if (stopped || !canPublish) return Promise.resolve();
    if (sending) return sending;
    clearTimeout(timer); timer = undefined;
    const operation = drain();
    sending = operation.finally(() => { sending = undefined; });
    return sending;
  }
  // Resume an already durable outbox after an application restart.
  schedule();
  return {
    getState: () => state,
    getStatus: () => status,
    edit: (contact: string, values: PrivateContactPatch) => patch(contact, values, false),
    seed: (contact: string, values: PrivateContactPatch) => patch(contact, values, true),
    receive,
    mergeTrusted,
    async refresh() { await update(current => current, { source: 'refresh', contact: '' }); pending(); },
    beginRead() { readComplete = false; readFailed = false; pending(); },
    async markReady(complete = true) {
      await reads;
      if (stopped) return;
      readComplete = complete;
      if (!complete) readFailed = true;
      pending();
    },
    flush,
    retry() { failures = 0; schedule(0); },
    stop() { stopped = true; clearTimeout(timer); timer = undefined; },
  };
}
export type PrivateContactSyncController = ReturnType<typeof createPrivateContactSyncController>;
