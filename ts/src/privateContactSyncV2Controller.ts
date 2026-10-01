import {
  acknowledgePrivateContactDocument, editPrivateContact, mergePrivateContactDocument,
  pendingPrivateContacts, queuePrivateContactSnapshot, seedPrivateContact,
  type PrivateContactDocument, type PrivateContactPatch, type PrivateContactSyncState,
} from './privateContactSyncV2';

/** Ready means handed to a durable sibling outbox, never confirmed on every device. */
export type PrivateContactSyncStatus = 'local-only' | 'pending' | 'queueing' | 'ready' | 'error';
export interface PrivateContactSyncChange { source: 'local' | 'sibling' | 'snapshot' | 'ack' | 'refresh'; contact: string }
export interface PrivateContactSyncControllerOptions {
  state: PrivateContactSyncState;
  save(state: PrivateContactSyncState): void | Promise<void>;
  load?(): PrivateContactSyncState | null | Promise<PrivateContactSyncState | null>;
  withLock?(run: () => Promise<void>): Promise<void>;
  /** True ONLY after durable authenticated sibling-outbox acceptance. False/throw retains the document. */
  send?(document: PrivateContactDocument): Promise<boolean>;
  onChange?(state: PrivateContactSyncState, change: PrivateContactSyncChange): void;
  onStatus?(status: PrivateContactSyncStatus): void;
  onError?(error: unknown): void;
  debounceMs?: number;
}
export function createPrivateContactSyncController(options: PrivateContactSyncControllerOptions) {
  if (!!options.load !== !!options.withLock) throw new Error('Shared contact persistence needs both a lock and a fresh load');
  let state = options.state;
  let stopped = false;
  let writes: Promise<void> = Promise.resolve();
  let sending: Promise<void> | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let failures = 0;
  let status: PrivateContactSyncStatus = !options.send ? 'local-only' : pendingPrivateContacts(state).length ? 'pending' : 'ready';
  function setStatus(next: PrivateContactSyncStatus) {
    if (stopped || status === next) return;
    status = next; options.onStatus?.(next);
  }
  function report(error: unknown) {
    if (stopped) return;
    setStatus('error'); options.onError?.(error);
  }
  function update(updater: (current: PrivateContactSyncState) => PrivateContactSyncState, change: PrivateContactSyncChange): Promise<void> {
    const run = async () => {
      if (stopped) throw new Error('Private contact controller stopped before durable completion');
      const latest = await options.load?.() ?? state;
      if (latest.owner !== state.owner || latest.writer !== state.writer) throw new Error('Private contact replica changed');
      const next = updater(latest);
      if (stopped) throw new Error('Private contact controller stopped before durable completion');
      if (JSON.stringify(next) !== JSON.stringify(latest)) await options.save(next);
      if (stopped) throw new Error('Private contact controller stopped before durable completion');
      if (JSON.stringify(next) === JSON.stringify(state)) return;
      state = next; options.onChange?.(state, change);
    };
    const operation = writes.then(() => options.withLock ? options.withLock(run) : run());
    writes = operation.catch(() => {});
    return operation;
  }
  function schedule(delay = options.debounceMs ?? 300) {
    if (stopped || !options.send || !pendingPrivateContacts(state).length) return;
    clearTimeout(timer);
    timer = setTimeout(() => { timer = undefined; void flush(); }, Math.max(0, delay));
  }
  function pending() {
    setStatus(!options.send ? 'local-only' : pendingPrivateContacts(state).length ? 'pending' : 'ready');
    schedule();
  }
  async function mutate(updater: (current: PrivateContactSyncState) => PrivateContactSyncState, change: PrivateContactSyncChange) {
    try { await update(updater, change); pending(); }
    catch (error) { report(error); throw error; }
  }
  async function drain() {
    await writes;
    if (stopped || !options.send) return;
    setStatus('queueing');
    try {
      // A bounded pass. Handoff waits never hold the local-write queue or cross-tab lock.
      for (const document of pendingPrivateContacts(state)) {
        if (stopped) return;
        await update(current => current, { source: 'refresh', contact: document.contact });
        if (stopped) return;
        const current = state.pending[document.contact];
        if (!current) continue;
        const captured = structuredClone(current);
        const accepted = await options.send(captured);
        if (stopped) return;
        if (!accepted) throw new Error('Private contact update is waiting for a durable sibling queue');
        await update(latest => acknowledgePrivateContactDocument(latest, captured), { source: 'ack', contact: captured.contact });
      }
      failures = 0; pending();
    } catch (error) {
      report(error);
      schedule(Math.min(30_000, 1000 * 2 ** Math.min(failures++, 5)));
    }
  }
  function flush(): Promise<void> {
    if (stopped || !options.send) return Promise.resolve();
    if (sending) return sending;
    clearTimeout(timer); timer = undefined;
    sending = drain().finally(() => { sending = undefined; });
    return sending;
  }
  schedule();
  return {
    getState: () => state,
    getStatus: () => status,
    edit: (contact: string, patch: PrivateContactPatch) => mutate(current => editPrivateContact(current, contact, patch), { source: 'local', contact }),
    seed: (contact: string, patch: PrivateContactPatch) => mutate(current => seedPrivateContact(current, contact, patch), { source: 'local', contact }),
    /** Resolve only after durable merge. Runtime must retain/replay an inbound control until this resolves. */
    mergeTrusted: (document: PrivateContactDocument) => mutate(current => mergePrivateContactDocument(current, document), { source: 'sibling', contact: document.contact }),
    queueSnapshot: () => mutate(queuePrivateContactSnapshot, { source: 'snapshot', contact: '' }),
    refresh: () => mutate(current => current, { source: 'refresh', contact: '' }),
    flush,
    retry() { failures = 0; schedule(0); },
    stop() { stopped = true; clearTimeout(timer); timer = undefined; },
  };
}
export type PrivateContactSyncController = ReturnType<typeof createPrivateContactSyncController>;
