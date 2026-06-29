import type { NostrEvent } from './utils';

export const FACT_OP_KIND = 7368;
export const FACT_SNAPSHOT_KIND = 37368;

const SUBJECT_MARKER = 'subject';
const PREV_MARKER = 'prev';
const REPLACE_MARKER = 'replace';
const DISPUTE_MARKER = 'dispute';
const HEAD_MARKER = 'head';
const SNAPSHOT_MS_TAG = 'ms';
const RESERVED_TAGS = new Set(['d', 'e', 'i', 'p', SNAPSHOT_MS_TAG]);

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const hex64Pattern = /^[0-9a-f]{64}$/;

export type Fact = {
  predicate: string;
  values: string[];
};

export type FactOpLinks = {
  prev?: string[];
  replace?: string[];
  dispute?: string[];
};

export type FactEventIndexes = {
  externalIdentifiers?: string[];
};

export type FactSnapshotMetadata = {
  createdAtMs?: number;
};

export type FactEventDraft = {
  kind: number;
  content: string;
  tags: string[][];
};

export type FactOp = {
  opId: string;
  authorPubkey: string;
  subject: string;
  facts: Fact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  prev: string[];
  replace: string[];
  dispute: string[];
  createdAt: number;
};

export type FactSnapshot = {
  snapshotId: string;
  authorPubkey: string;
  subject: string;
  facts: Fact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  heads: string[];
  createdAt: number;
  createdAtMs?: number;
};

export type ParsedFactOpDraft = {
  subject: string;
  facts: Fact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  prev: string[];
  replace: string[];
  dispute: string[];
};

export type ParsedFactSnapshotDraft = {
  subject: string;
  facts: Fact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  heads: string[];
  createdAtMs?: number;
};

export type FactProjection = {
  subject: string;
  facts: Map<string, Set<string>>;
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  appliedOpIds: Set<string>;
  replacedOpIds: Set<string>;
  disputedOpIds: Set<string>;
  heads: Set<string>;
};

type ParsedFactEvent = {
  subject: string;
  facts: Fact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  prev: string[];
  replace: string[];
  dispute: string[];
  heads: string[];
  createdAtMs?: number;
};

export function fact(predicate: string, values: string[]): Fact {
  return { predicate, values };
}

export function buildFactOpDraft(
  subject: string,
  facts: Fact[],
  links: FactOpLinks = {},
  indexes: FactEventIndexes = {},
): FactEventDraft {
  return {
    kind: FACT_OP_KIND,
    content: '',
    tags: buildFactOpTags(subject, facts, links, indexes),
  };
}

export function buildFactSnapshotDraft(
  subject: string,
  facts: Fact[],
  heads: string[],
  indexes: FactEventIndexes = {},
  metadata: FactSnapshotMetadata = {},
): FactEventDraft {
  return {
    kind: FACT_SNAPSHOT_KIND,
    content: '',
    tags: buildFactSnapshotTags(subject, facts, heads, indexes, metadata),
  };
}

export function buildFactOpTags(
  subject: string,
  facts: Fact[],
  links: FactOpLinks = {},
  indexes: FactEventIndexes = {},
): string[][] {
  const normalizedSubject = normalizeUuid(subject);
  const normalizedFacts = normalizeFacts(facts);
  const normalizedLinks = normalizeLinks(links);
  const externalIdentifiers = normalizeExternalIdentifiers(indexes.externalIdentifiers ?? []);
  const tags: string[][] = [['i', normalizedSubject, SUBJECT_MARKER]];
  for (const id of normalizedLinks.prev) tags.push(['e', id, '', PREV_MARKER]);
  for (const id of normalizedLinks.replace) tags.push(['e', id, '', REPLACE_MARKER]);
  for (const id of normalizedLinks.dispute) tags.push(['e', id, '', DISPUTE_MARKER]);
  tags.push(...indexTags(normalizedSubject, normalizedFacts, externalIdentifiers));
  tags.push(...normalizedFacts.map(factParts));
  return tags;
}

export function buildFactSnapshotTags(
  subject: string,
  facts: Fact[],
  heads: string[],
  indexes: FactEventIndexes = {},
  metadata: FactSnapshotMetadata = {},
): string[][] {
  const normalizedSubject = normalizeUuid(subject);
  const normalizedFacts = normalizeFacts(facts);
  const externalIdentifiers = normalizeExternalIdentifiers(indexes.externalIdentifiers ?? []);
  const normalizedHeads = normalizeEventIds(heads, HEAD_MARKER);
  const createdAtMs = normalizeCreatedAtMs(metadata.createdAtMs);
  const tags: string[][] = [
    ['d', normalizedSubject],
    ['i', normalizedSubject, SUBJECT_MARKER],
  ];
  if (createdAtMs !== undefined) tags.push([SNAPSHOT_MS_TAG, createdAtMs.toString()]);
  for (const id of normalizedHeads) tags.push(['e', id, '', HEAD_MARKER]);
  tags.push(...indexTags(normalizedSubject, normalizedFacts, externalIdentifiers));
  tags.push(...normalizedFacts.map(factParts));
  return canonicalizeTags(tags);
}

export function parseFactOpEvent(event: NostrEvent): FactOp {
  if (event.kind !== FACT_OP_KIND) {
    throw new Error(`wrong fact op kind: expected ${FACT_OP_KIND}, got ${event.kind}`);
  }
  const parsed = parseCommonTags(event, false);
  return {
    opId: normalizeEventId(event.id, 'op id'),
    authorPubkey: normalizePubkey(event.pubkey),
    subject: parsed.subject,
    facts: parsed.facts,
    pubkeys: parsed.pubkeys,
    externalIdentifiers: parsed.externalIdentifiers,
    mentionedSubjects: parsed.mentionedSubjects,
    prev: parsed.prev,
    replace: parsed.replace,
    dispute: parsed.dispute,
    createdAt: event.created_at,
  };
}

export function parseFactSnapshotEvent(event: NostrEvent): FactSnapshot {
  if (event.kind !== FACT_SNAPSHOT_KIND) {
    throw new Error(
      `wrong fact snapshot kind: expected ${FACT_SNAPSHOT_KIND}, got ${event.kind}`,
    );
  }
  const parsed = parseCommonTags(event, true);
  validateSnapshotCreatedAtMs(parsed.createdAtMs, event.created_at);
  return {
    snapshotId: normalizeEventId(event.id, 'snapshot id'),
    authorPubkey: normalizePubkey(event.pubkey),
    subject: parsed.subject,
    facts: parsed.facts,
    pubkeys: parsed.pubkeys,
    externalIdentifiers: parsed.externalIdentifiers,
    mentionedSubjects: parsed.mentionedSubjects,
    heads: parsed.heads,
    createdAt: event.created_at,
    ...(parsed.createdAtMs !== undefined ? { createdAtMs: parsed.createdAtMs } : {}),
  };
}

export function parseFactOpDraft(draft: FactEventDraft): ParsedFactOpDraft {
  if (draft.kind !== FACT_OP_KIND) {
    throw new Error(`wrong fact op draft kind: expected ${FACT_OP_KIND}, got ${draft.kind}`);
  }
  const parsed = parseCommonTags(draft, false);
  return {
    subject: parsed.subject,
    facts: parsed.facts,
    pubkeys: parsed.pubkeys,
    externalIdentifiers: parsed.externalIdentifiers,
    mentionedSubjects: parsed.mentionedSubjects,
    prev: parsed.prev,
    replace: parsed.replace,
    dispute: parsed.dispute,
  };
}

export function parseFactSnapshotDraft(draft: FactEventDraft): ParsedFactSnapshotDraft {
  if (draft.kind !== FACT_SNAPSHOT_KIND) {
    throw new Error(`wrong fact snapshot draft kind: expected ${FACT_SNAPSHOT_KIND}, got ${draft.kind}`);
  }
  const parsed = parseCommonTags(draft, true);
  return {
    subject: parsed.subject,
    facts: parsed.facts,
    pubkeys: parsed.pubkeys,
    externalIdentifiers: parsed.externalIdentifiers,
    mentionedSubjects: parsed.mentionedSubjects,
    heads: parsed.heads,
    ...(parsed.createdAtMs !== undefined ? { createdAtMs: parsed.createdAtMs } : {}),
  };
}

export function compareFactSnapshots(left: FactSnapshot, right: FactSnapshot): number {
  const leftMs = left.createdAtMs ?? left.createdAt * 1000;
  const rightMs = right.createdAtMs ?? right.createdAt * 1000;
  return left.createdAt - right.createdAt
    || leftMs - rightMs
    || left.snapshotId.localeCompare(right.snapshotId);
}

export function projectFactOps(subject: string, ops: FactOp[]): FactProjection {
  const normalizedSubject = normalizeUuid(subject);
  const matching = ops
    .filter((op) => op.subject === normalizedSubject)
    .slice()
    .sort((left, right) => left.createdAt - right.createdAt || left.opId.localeCompare(right.opId));

  const replacedOpIds = new Set<string>();
  const disputedOpIds = new Set<string>();
  for (const op of matching) {
    for (const id of op.replace) replacedOpIds.add(id);
    for (const id of op.dispute) disputedOpIds.add(id);
  }

  const facts = new Map<string, Set<string>>();
  const pubkeys = new Set<string>();
  const externalIdentifiers = new Set<string>();
  const mentionedSubjects = new Set<string>();
  const appliedOpIds = new Set<string>();
  const previousIds = new Set<string>();

  for (const op of matching) {
    if (replacedOpIds.has(op.opId)) continue;
    for (const id of op.prev) previousIds.add(id);
    for (const pubkey of op.pubkeys) pubkeys.add(pubkey);
    for (const identifier of op.externalIdentifiers) externalIdentifiers.add(identifier);
    for (const mentioned of op.mentionedSubjects) mentionedSubjects.add(mentioned);
    for (const fact of op.facts) {
      const set = facts.get(fact.predicate) ?? new Set<string>();
      set.add(factKey(fact.values));
      facts.set(fact.predicate, set);
    }
    appliedOpIds.add(op.opId);
  }

  const heads = new Set([...appliedOpIds].filter((id) => !previousIds.has(id)));
  return {
    subject: normalizedSubject,
    facts,
    pubkeys,
    externalIdentifiers,
    mentionedSubjects,
    appliedOpIds,
    replacedOpIds,
    disputedOpIds,
    heads,
  };
}

export function factsFromProjection(projection: FactProjection): Fact[] {
  return [...projection.facts.entries()].flatMap(([predicate, values]) =>
    [...values].map((value) => ({ predicate, values: JSON.parse(value) as string[] })),
  );
}

function parseCommonTags(event: Pick<FactEventDraft, 'content' | 'tags'>, snapshot: boolean): ParsedFactEvent {
  if (event.content !== '') throw new Error('fact events must have empty content');

  let subject: string | null = null;
  const facts: Fact[] = [];
  const pubkeys = new Set<string>();
  const externalIdentifiers = new Set<string>();
  const mentionedSubjects = new Set<string>();
  const prev: string[] = [];
  const replace: string[] = [];
  const dispute: string[] = [];
  const heads: string[] = [];
  let createdAtMs: number | undefined;

  for (const tag of event.tags) {
    const kind = tag[0];
    if (!kind) continue;
    if (kind === 'd') {
      if (!snapshot) throw new Error('fact op event must not use d tag');
      continue;
    }
    if (kind === 'i') {
      const value = tag[1];
      if (!value) throw new Error('fact i tag is missing value');
      if (tag[2] === SUBJECT_MARKER) {
        const uuid = normalizeUuid(value);
        if (subject) throw new Error('fact event has multiple subject i tags');
        subject = uuid;
      } else if (isUuid(value)) {
        mentionedSubjects.add(normalizeUuid(value));
      } else {
        externalIdentifiers.add(normalizeExternalIdentifier(value));
      }
      continue;
    }
    if (kind === 'p') {
      const value = tag[1];
      if (!value) throw new Error('fact p tag is missing pubkey');
      pubkeys.add(normalizePubkey(value));
      continue;
    }
    if (kind === 'e') {
      const value = tag[1];
      if (!value) throw new Error('fact e tag is missing event id');
      const id = normalizeEventId(value, 'linked');
      const marker = tag[3];
      if (marker === PREV_MARKER) prev.push(id);
      else if (marker === REPLACE_MARKER) replace.push(id);
      else if (marker === DISPUTE_MARKER) dispute.push(id);
      else if (marker === HEAD_MARKER && snapshot) heads.push(id);
      else if (marker) throw new Error(`unsupported fact e tag marker: ${marker}`);
      else throw new Error('fact e tag is missing marker');
      continue;
    }
    if (kind === SNAPSHOT_MS_TAG) {
      if (!snapshot) throw new Error('fact op event must not use ms tag');
      if (createdAtMs !== undefined) throw new Error('fact snapshot has multiple ms tags');
      createdAtMs = normalizeCreatedAtMsTag(tag[1]);
      continue;
    }
    if ([...kind].length === 1) continue;
    if (snapshot && kind === 'expiration') continue;
    facts.push({
      predicate: normalizePredicate(kind),
      values: tag.slice(1).map(normalizeValue),
    });
  }

  if (!subject) throw new Error('fact event is missing subject i tag');
  if (snapshot) {
    const d = event.tags.find((tag) => tag[0] === 'd')?.[1];
    if (!d) throw new Error('fact snapshot is missing d tag');
    const dSubject = normalizeUuid(d);
    if (dSubject !== subject) {
      throw new Error(`fact snapshot d tag ${dSubject} does not match subject ${subject}`);
    }
  }
  for (const fact of facts) {
    for (const value of fact.values) {
      if (isUuid(value)) {
        if (value !== subject) mentionedSubjects.add(value);
      } else if (isPubkey(value)) {
        pubkeys.add(value);
      }
    }
  }

  return {
    subject,
    facts: normalizeFacts(facts),
    pubkeys,
    externalIdentifiers,
    mentionedSubjects,
    prev: uniqueSorted(prev),
    replace: uniqueSorted(replace),
    dispute: uniqueSorted(dispute),
    heads: uniqueSorted(heads),
    ...(createdAtMs !== undefined ? { createdAtMs } : {}),
  };
}

function normalizeLinks(links: FactOpLinks): Required<FactOpLinks> {
  return {
    prev: normalizeEventIds(links.prev ?? [], PREV_MARKER),
    replace: normalizeEventIds(links.replace ?? [], REPLACE_MARKER),
    dispute: normalizeEventIds(links.dispute ?? [], DISPUTE_MARKER),
  };
}

function normalizeExternalIdentifiers(values: string[]): Set<string> {
  return new Set(values.map(normalizeExternalIdentifier).sort());
}

function normalizeExternalIdentifier(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) throw new Error('external identifier cannot be empty');
  if (isUuid(trimmed)) return normalizeUuid(trimmed);
  if (/\s/.test(trimmed)) throw new Error(`external identifier cannot contain whitespace: ${trimmed}`);
  return trimmed.toLowerCase();
}

function normalizeFacts(facts: Fact[]): Fact[] {
  const normalized = facts.map((fact) => {
    const predicate = normalizePredicate(fact.predicate);
    if (RESERVED_TAGS.has(predicate)) throw new Error(`${predicate} is a reserved fact event tag`);
    return {
      predicate,
      values: fact.values.map(normalizeValue),
    };
  });
  const seen = new Set<string>();
  return normalized
    .sort(compareFacts)
    .filter((fact) => {
      const key = `${fact.predicate}\0${factKey(fact.values)}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

function normalizePredicate(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) throw new Error('fact predicate cannot be empty');
  if ([...trimmed].length < 2) {
    throw new Error(`fact predicate must be at least two characters: ${trimmed}`);
  }
  if (/\s/.test(trimmed)) throw new Error(`fact predicate cannot contain whitespace: ${trimmed}`);
  return trimmed;
}

function normalizeValue(value: string): string {
  const trimmed = value.trim();
  if (isUuid(trimmed)) return normalizeUuid(trimmed);
  if (isPubkey(trimmed)) return normalizePubkey(trimmed);
  return trimmed;
}

function normalizeUuid(value: string): string {
  const trimmed = value.trim();
  const lower = trimmed.toLowerCase();
  if (!uuidPattern.test(lower) || trimmed !== lower) {
    throw new Error(`UUID must be canonical lowercase hyphenated text: ${value}`);
  }
  return lower;
}

function normalizePubkey(value: string): string {
  const lower = value.trim().toLowerCase();
  if (!hex64Pattern.test(lower)) throw new Error(`invalid pubkey: ${value}`);
  return lower;
}

function normalizeEventId(value: string, role: string): string {
  const lower = value.trim().toLowerCase();
  if (!hex64Pattern.test(lower)) throw new Error(`invalid ${role} event id: ${value}`);
  return lower;
}

function normalizeEventIds(values: string[], role: string): string[] {
  return uniqueSorted(values.map((value) => normalizeEventId(value, role)));
}

function normalizeCreatedAtMs(value: number | undefined): number | undefined {
  if (value === undefined) return undefined;
  if (!Number.isInteger(value) || value < 0) {
    throw new Error(`createdAtMs must be a non-negative integer: ${value}`);
  }
  return value;
}

function normalizeCreatedAtMsTag(value: string | undefined): number {
  if (!value) throw new Error('fact snapshot ms tag is missing value');
  if (!/^[0-9]+$/.test(value)) throw new Error(`fact snapshot ms tag must be decimal milliseconds: ${value}`);
  const ms = Number(value);
  if (!Number.isSafeInteger(ms)) throw new Error(`fact snapshot ms tag is not a safe integer: ${value}`);
  return normalizeCreatedAtMs(ms)!;
}

function validateSnapshotCreatedAtMs(createdAtMs: number | undefined, createdAt: number): void {
  if (createdAtMs === undefined) return;
  if (Math.floor(createdAtMs / 1000) !== createdAt) {
    throw new Error(`fact snapshot ms tag ${createdAtMs} does not match created_at ${createdAt}`);
  }
}

function indexTags(subject: string, facts: Fact[], externalIdentifiers: Set<string>): string[][] {
  const uuids = new Set<string>();
  const pubkeys = new Set<string>();
  for (const fact of facts) {
    for (const value of fact.values) {
      if (isUuid(value) && value !== subject) uuids.add(value);
      else if (isPubkey(value)) pubkeys.add(value);
    }
  }
  for (const identifier of externalIdentifiers) {
    if (isUuid(identifier) && identifier !== subject) uuids.add(identifier);
  }
  return [
    ...[...uuids].sort().map((uuid) => ['i', uuid]),
    ...[...externalIdentifiers]
      .filter((identifier) => !isUuid(identifier))
      .sort()
      .map((identifier) => ['i', identifier]),
    ...[...pubkeys].sort().map((pubkey) => ['p', pubkey]),
  ];
}

function factParts(fact: Fact): string[] {
  return [fact.predicate, ...fact.values];
}

function canonicalizeTags(tags: string[][]): string[][] {
  const unique = new Map(tags.map((tag) => [tagKey(tag), tag]));
  return [...unique.values()].sort(compareTags);
}

function compareTags(left: string[], right: string[]): number {
  const len = Math.max(left.length, right.length);
  for (let index = 0; index < len; index += 1) {
    const diff = (left[index] ?? '').localeCompare(right[index] ?? '');
    if (diff !== 0) return diff;
  }
  return 0;
}

function compareFacts(left: Fact, right: Fact): number {
  return left.predicate.localeCompare(right.predicate) || compareTags([left.predicate, ...left.values], [right.predicate, ...right.values]);
}

function uniqueSorted(values: string[]): string[] {
  return [...new Set(values)].sort();
}

function factKey(values: string[]): string {
  return JSON.stringify(values);
}

function tagKey(tag: string[]): string {
  return JSON.stringify(tag);
}

function isUuid(value: string): boolean {
  return uuidPattern.test(value);
}

function isPubkey(value: string): boolean {
  return hex64Pattern.test(value);
}
