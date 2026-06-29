import { verifyEvent, type Event } from 'nostr-tools';

export function requireKind(event: Event, expected: number): void {
  if (event.kind !== expected) {
    throw new Error(`invalid event kind: expected ${expected}, got ${event.kind}`);
  }
}

export function requireIdentifier(event: Event): string {
  const tag = event.tags.find(([name]) => name === 'd');
  const identifier = tag?.[1];
  if (!identifier) throw new Error('missing d tag');
  return identifier;
}

export function requireValidSignature(event: Event): void {
  if (!verifyEvent(event)) throw new Error('event signature verification failed');
}

export function parseObject(json: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(json);
  if (!isRecord(parsed)) throw new Error('event content must be a JSON object');
  return parsed;
}

export function randomClientNonce(): string {
  return globalThis.crypto?.randomUUID?.() ?? `nonce-${Math.random().toString(36).slice(2)}`;
}

export function currentUnixSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

export function stableStringify(value: unknown): string {
  return JSON.stringify(sortJson(value));
}

export function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortJson);
  if (isRecord(value)) {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sortJson(value[key])]),
    );
  }
  return value;
}

export function sortRecord<T>(record: Record<string, T>): Record<string, T> {
  return Object.fromEntries(
    Object.entries(record).sort(([a], [b]) => a.localeCompare(b)),
  ) as Record<string, T>;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function isHex32(value: string): boolean {
  return /^[0-9a-f]{64}$/i.test(value);
}

export function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value);
}
