import type { NostrEvent } from './utils';

export const IDENTITY_OP_KIND = 7368;
export const IDENTITY_SNAPSHOT_KIND = 37368;

const SUBJECT_MARKER = 'subject';
const PREV_MARKER = 'prev';
const REPLACE_MARKER = 'replace';
const DISPUTE_MARKER = 'dispute';
const HEAD_MARKER = 'head';
const RESERVED_TAGS = new Set(['d', 'e', 'i', 'p']);

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const hex64Pattern = /^[0-9a-f]{64}$/;

export type IdentityFact = {
  predicate: string;
  values: string[];
};

export type IdentityOpLinks = {
  prev?: string[];
  replace?: string[];
  dispute?: string[];
};

export type IdentityEventIndexes = {
  externalIdentifiers?: string[];
};

export type IdentityEventDraft = {
  kind: number;
  content: string;
  tags: string[][];
};

export type IdentityOp = {
  opId: string;
  authorPubkey: string;
  subject: string;
  facts: IdentityFact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  prev: string[];
  replace: string[];
  dispute: string[];
  createdAt: number;
};

export type IdentitySnapshot = {
  snapshotId: string;
  authorPubkey: string;
  subject: string;
  facts: IdentityFact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  heads: string[];
  createdAt: number;
};

export type IdentityProjection = {
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

type ParsedIdentityEvent = {
  subject: string;
  facts: IdentityFact[];
  pubkeys: Set<string>;
  externalIdentifiers: Set<string>;
  mentionedSubjects: Set<string>;
  prev: string[];
  replace: string[];
  dispute: string[];
  heads: string[];
};

export function identityFact(predicate: string, values: string[]): IdentityFact {
  return { predicate, values };
}

export function buildIdentityOpDraft(
  subject: string,
  facts: IdentityFact[],
  links: IdentityOpLinks = {},
  indexes: IdentityEventIndexes = {},
): IdentityEventDraft {
  return {
    kind: IDENTITY_OP_KIND,
    content: '',
    tags: buildIdentityOpTags(subject, facts, links, indexes),
  };
}

export function buildIdentitySnapshotDraft(
  subject: string,
  facts: IdentityFact[],
  heads: string[],
  indexes: IdentityEventIndexes = {},
): IdentityEventDraft {
  return {
    kind: IDENTITY_SNAPSHOT_KIND,
    content: '',
    tags: buildIdentitySnapshotTags(subject, facts, heads, indexes),
  };
}

export function buildIdentityOpTags(
  subject: string,
  facts: IdentityFact[],
  links: IdentityOpLinks = {},
  indexes: IdentityEventIndexes = {},
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

export function buildIdentitySnapshotTags(
  subject: string,
  facts: IdentityFact[],
  heads: string[],
  indexes: IdentityEventIndexes = {},
): string[][] {
  const normalizedSubject = normalizeUuid(subject);
  const normalizedFacts = normalizeFacts(facts);
  const externalIdentifiers = normalizeExternalIdentifiers(indexes.externalIdentifiers ?? []);
  const normalizedHeads = normalizeEventIds(heads, HEAD_MARKER);
  const tags: string[][] = [
    ['d', normalizedSubject],
    ['i', normalizedSubject, SUBJECT_MARKER],
  ];
  for (const id of normalizedHeads) tags.push(['e', id, '', HEAD_MARKER]);
  tags.push(...indexTags(normalizedSubject, normalizedFacts, externalIdentifiers));
  tags.push(...normalizedFacts.map(factParts));
  return canonicalizeTags(tags);
}

export function parseIdentityOpEvent(event: NostrEvent): IdentityOp {
  if (event.kind !== IDENTITY_OP_KIND) {
    throw new Error(`wrong identity op kind: expected ${IDENTITY_OP_KIND}, got ${event.kind}`);
  }
  const parsed = parseCommonEvent(event, false);
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

export function parseIdentitySnapshotEvent(event: NostrEvent): IdentitySnapshot {
  if (event.kind !== IDENTITY_SNAPSHOT_KIND) {
    throw new Error(
      `wrong identity snapshot kind: expected ${IDENTITY_SNAPSHOT_KIND}, got ${event.kind}`,
    );
  }
  const parsed = parseCommonEvent(event, true);
  const d = event.tags.find((tag) => tag[0] === 'd')?.[1];
  if (!d) throw new Error('identity snapshot is missing d tag');
  const dSubject = normalizeUuid(d);
  if (dSubject !== parsed.subject) {
    throw new Error(`identity snapshot d tag ${dSubject} does not match subject ${parsed.subject}`);
  }
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
  };
}

export function projectIdentityOps(subject: string, ops: IdentityOp[]): IdentityProjection {
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

export function factsFromProjection(projection: IdentityProjection): IdentityFact[] {
  return [...projection.facts.entries()].flatMap(([predicate, values]) =>
    [...values].map((value) => ({ predicate, values: JSON.parse(value) as string[] })),
  );
}

function parseCommonEvent(event: NostrEvent, snapshot: boolean): ParsedIdentityEvent {
  if (event.content !== '') throw new Error('identity events must have empty content');

  let subject: string | null = null;
  const facts: IdentityFact[] = [];
  const pubkeys = new Set<string>();
  const externalIdentifiers = new Set<string>();
  const mentionedSubjects = new Set<string>();
  const prev: string[] = [];
  const replace: string[] = [];
  const dispute: string[] = [];
  const heads: string[] = [];

  for (const tag of event.tags) {
    const kind = tag[0];
    if (!kind) continue;
    if (kind === 'd') {
      if (!snapshot) throw new Error('identity op event must not use d tag');
      continue;
    }
    if (kind === 'i') {
      const value = tag[1];
      if (!value) throw new Error('identity i tag is missing value');
      if (tag[2] === SUBJECT_MARKER) {
        const uuid = normalizeUuid(value);
        if (subject) throw new Error('identity event has multiple subject i tags');
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
      if (!value) throw new Error('identity p tag is missing pubkey');
      pubkeys.add(normalizePubkey(value));
      continue;
    }
    if (kind === 'e') {
      const value = tag[1];
      if (!value) throw new Error('identity e tag is missing event id');
      const id = normalizeEventId(value, 'linked');
      const marker = tag[3];
      if (marker === PREV_MARKER) prev.push(id);
      else if (marker === REPLACE_MARKER) replace.push(id);
      else if (marker === DISPUTE_MARKER) dispute.push(id);
      else if (marker === HEAD_MARKER && snapshot) heads.push(id);
      else if (marker) throw new Error(`unsupported identity e tag marker: ${marker}`);
      else throw new Error('identity e tag is missing marker');
      continue;
    }
    if (snapshot && kind === 'expiration') continue;
    facts.push({
      predicate: normalizePredicate(kind),
      values: tag.slice(1).map(normalizeValue),
    });
  }

  if (!subject) throw new Error('identity event is missing subject i tag');
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
  };
}

function normalizeLinks(links: IdentityOpLinks): Required<IdentityOpLinks> {
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
  if (!trimmed) throw new Error('external identity identifier cannot be empty');
  if (isUuid(trimmed)) return normalizeUuid(trimmed);
  if (/\s/.test(trimmed)) throw new Error(`external identity identifier cannot contain whitespace: ${trimmed}`);
  return trimmed.toLowerCase();
}

function normalizeFacts(facts: IdentityFact[]): IdentityFact[] {
  const normalized = facts.map((fact) => {
    const predicate = normalizePredicate(fact.predicate);
    if (RESERVED_TAGS.has(predicate)) throw new Error(`${predicate} is a reserved identity event tag`);
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
  if (!trimmed) throw new Error('identity fact predicate cannot be empty');
  if (/\s/.test(trimmed)) throw new Error(`identity fact predicate cannot contain whitespace: ${trimmed}`);
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

function indexTags(subject: string, facts: IdentityFact[], externalIdentifiers: Set<string>): string[][] {
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

function factParts(fact: IdentityFact): string[] {
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

function compareFacts(left: IdentityFact, right: IdentityFact): number {
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
