import { buildFactOpDraft } from './factEvents';
import { getEventHash, verifyEvent, type Event, type UnsignedEvent } from 'nostr-tools';

/** An ordinary Nostr event without a signature. Transport must authenticate admission. */
export type PrivateEvent = UnsignedEvent & { id: string };

export function buildPrivateEvent(event: UnsignedEvent): PrivateEvent {
  const result = { pubkey: event.pubkey, created_at: event.created_at, kind: event.kind,
    tags: event.tags.map(tag => [...tag]), content: event.content, id: getEventHash(event) };
  return parsePrivateEvent(result);
}

/** Legacy signed records are validated before their signature is discarded. */
export function parsePrivateEvent(value: unknown, allowLegacySigned = false): PrivateEvent {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid private event');
  const v = value as Record<string, any>;
  const keys = ['id', 'pubkey', 'created_at', 'kind', 'tags', 'content'];
  if (allowLegacySigned && 'sig' in v) keys.push('sig');
  if (Object.keys(v).length !== keys.length || !keys.every(key => Object.prototype.hasOwnProperty.call(v, key)) ||
    !/^[a-f0-9]{64}$/.test(v.id) || !/^[a-f0-9]{64}$/.test(v.pubkey) ||
    !Number.isSafeInteger(v.created_at) || v.created_at < 0 || !Number.isInteger(v.kind) || v.kind < 0 || v.kind > 65535 ||
    typeof v.content !== 'string' || !Array.isArray(v.tags) || !v.tags.every((tag: unknown) => Array.isArray(tag) && tag.every(x => typeof x === 'string'))) throw new Error('Invalid private event');
  const event: PrivateEvent = { id: v.id, pubkey: v.pubkey, created_at: v.created_at, kind: v.kind, content: v.content, tags: v.tags.map((tag: string[]) => [...tag]) };
  if (getEventHash(event) !== event.id || 'sig' in v && !verifyEvent({ ...event, sig: v.sig } as Event)) throw new Error('Invalid private event ID or legacy signature');
  return event;
}

/** Account settings use the same fact vocabulary as contacts; the value carries
 * the setting's existing conflict stamp, including explicit clears. */
export function buildPrivateSetting(owner: string, setting: string, target: string, value: unknown): PrivateEvent {
  const draft = buildFactOpDraft(owner, [
    { predicate: 'type', values: ['private_setting'] },
    { predicate: 'setting', values: [setting] },
    { predicate: 'target', values: [target] },
    { predicate: 'value', values: [canonicalJson(value)] },
  ]);
  return buildPrivateEvent({ ...draft, pubkey: owner, created_at: 0 });
}
export function parsePrivateSetting(value: unknown, owner: string): { setting: string; target: string; value: any } {
  const event = parsePrivateEvent(value);
  const tag = (name: string) => event.tags.find(tag => tag[0] === name)?.[1];
  if (event.pubkey !== owner || event.kind !== 7368 || event.created_at !== 0 || event.content !== '' ||
    event.tags.length > 8 || tag('type') !== 'private_setting' || !tag('setting') || !tag('target')) throw new Error('Invalid private setting');
  const result = { setting: tag('setting')!, target: tag('target')!, value: JSON.parse(tag('value') ?? '') };
  if (buildPrivateSetting(owner, result.setting, result.target, result.value).id !== event.id) throw new Error('Noncanonical private setting');
  return result;
}
function canonicalJson(value: unknown): string {
  const sorted = (v: any): any => v && typeof v === 'object' ? Array.isArray(v) ? v.map(sorted) :
    Object.fromEntries(Object.keys(v).sort().map(key => [key, sorted(v[key])])) : v;
  return JSON.stringify(sorted(value));
}
