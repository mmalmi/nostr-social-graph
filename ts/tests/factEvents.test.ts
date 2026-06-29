import { describe, expect, it } from 'vitest';
import {
  FACT_OP_KIND,
  FACT_SNAPSHOT_KIND,
  buildFactOpDraft,
  buildFactSnapshotDraft,
  compareFactSnapshots,
  fact,
  parseFactOpEvent,
  parseFactSnapshotDraft,
  parseFactSnapshotEvent,
  projectFactOps,
  type FactOpLinks,
} from '../src/factEvents';
import type { NostrEvent } from '../src/utils';

const subject = '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a';
const other = '9f3d3b6f-4f5e-4f8a-9d6e-7c2a7c6c9f11';
const pubkey = '4f355bdcb7c27f8f3e4f6c4ec8f996d45b33f2d9c93a9d5c7aa9a6e1a4e7f8aa';

function eventId(byte: string): string {
  return byte.repeat(64);
}

function eventFromDraft(
  draft: { kind: number; content: string; tags: string[][] },
  id: string,
  createdAt = 123,
): NostrEvent {
  return {
    ...draft,
    id,
    pubkey,
    sig: eventId('b'),
    created_at: createdAt,
  };
}

describe('fact events', () => {
  it('builds and parses tag-only op drafts', () => {
    const draft = buildFactOpDraft(
      subject,
      [
        fact('name', ['Alice']),
        fact('controls', [pubkey]),
        fact('same_as', [other]),
      ],
      { prev: [eventId('1')] },
      { externalIdentifiers: ['nip05:alice@example.com'] },
    );

    expect(draft.kind).toBe(FACT_OP_KIND);
    expect(draft.content).toBe('');
    expect(draft.tags).toContainEqual(['i', subject, 'subject']);
    expect(draft.tags).toContainEqual(['p', pubkey]);
    expect(draft.tags).toContainEqual(['i', other]);
    expect(draft.tags).toContainEqual(['i', 'nip05:alice@example.com']);

    const op = parseFactOpEvent(eventFromDraft(draft, eventId('2')));
    expect(op.subject).toBe(subject);
    expect(op.prev).toEqual([eventId('1')]);
    expect(op.pubkeys.has(pubkey)).toBe(true);
    expect(op.externalIdentifiers.has('nip05:alice@example.com')).toBe(true);
    expect(op.mentionedSubjects.has(other)).toBe(true);
    expect(op.facts).toContainEqual(fact('name', ['Alice']));
  });

  it('builds canonical bare-uuid snapshot tags', () => {
    const draft = buildFactSnapshotDraft(
      subject,
      [
        fact('same_as', [other]),
        fact('name', ['Alice']),
        fact('controls', [pubkey]),
      ],
      [eventId('3')],
    );

    expect(draft.kind).toBe(FACT_SNAPSHOT_KIND);
    expect(draft.tags).toContainEqual(['d', subject]);
    const sorted = [...draft.tags].sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right)));
    expect(draft.tags).toEqual(sorted);

    const snapshot = parseFactSnapshotEvent(eventFromDraft(draft, eventId('4')));
    expect(snapshot.subject).toBe(subject);
    expect(snapshot.heads).toEqual([eventId('3')]);
  });

  it('parses unsigned snapshot drafts locally', () => {
    const draft = buildFactSnapshotDraft(
      subject,
      [fact('name', ['Alice'])],
      [eventId('3')],
      { externalIdentifiers: ['github:alice'] },
    );

    const parsed = parseFactSnapshotDraft(draft);
    expect(parsed.subject).toBe(subject);
    expect(parsed.heads).toEqual([eventId('3')]);
    expect(parsed.externalIdentifiers.has('github:alice')).toBe(true);
    expect(parsed.facts).toContainEqual(fact('name', ['Alice']));
  });

  it('uses snapshot ms tags as metadata for sub-second ordering', () => {
    const draft = buildFactSnapshotDraft(
      subject,
      [fact('name', ['Alice'])],
      [eventId('3')],
      {},
      { createdAtMs: 123456 },
    );

    expect(draft.tags).toContainEqual(['ms', '123456']);

    const snapshot = parseFactSnapshotEvent(eventFromDraft(draft, eventId('4'), 123));
    expect(snapshot.createdAtMs).toBe(123456);
    expect(snapshot.facts).not.toContainEqual(fact('ms', ['123456']));

    const later = parseFactSnapshotEvent(
      eventFromDraft(
        buildFactSnapshotDraft(subject, [fact('name', ['Alice later'])], [], {}, { createdAtMs: 123789 }),
        eventId('5'),
        123,
      ),
    );
    expect(compareFactSnapshots(snapshot, later)).toBeLessThan(0);
  });

  it('rejects snapshot ms tags that do not match created_at seconds', () => {
    const draft = buildFactSnapshotDraft(subject, [fact('name', ['Alice'])], [], {}, { createdAtMs: 124000 });
    expect(() => parseFactSnapshotEvent(eventFromDraft(draft, eventId('4'), 123))).toThrow(
      /does not match created_at/,
    );
  });

  it('rejects single-character predicates', () => {
    expect(() => buildFactOpDraft(subject, [fact('x', ['y'])])).toThrow(
      /predicate must be at least two characters/,
    );
  });

  it('rejects ms as a fact predicate', () => {
    expect(() => buildFactOpDraft(subject, [fact('ms', ['123456'])])).toThrow(
      /reserved fact event tag/,
    );
  });

  it('projects heads through optional prev links', () => {
    const first = parseFactOpEvent(
      eventFromDraft(buildFactOpDraft(subject, [fact('name', ['Alice'])]), eventId('5')),
    );
    const second = parseFactOpEvent(
      eventFromDraft(
        buildFactOpDraft(subject, [fact('picture', ['https://example.com/a.jpg'])], {
          prev: [first.opId],
        }),
        eventId('6'),
      ),
    );

    const projection = projectFactOps(subject, [first, second]);
    expect(projection.appliedOpIds.size).toBe(2);
    expect(projection.heads).toEqual(new Set([second.opId]));
    expect(projection.facts.get('name')?.has(JSON.stringify(['Alice']))).toBe(true);
  });

  it('can mark replaced ops without mutating signed tags', () => {
    const old = parseFactOpEvent(
      eventFromDraft(buildFactOpDraft(subject, [fact('name', ['Alice Old'])]), eventId('7')),
    );
    const links: FactOpLinks = { replace: [old.opId] };
    const replacement = parseFactOpEvent(
      eventFromDraft(buildFactOpDraft(subject, [fact('name', ['Alice'])], links), eventId('8')),
    );

    const projection = projectFactOps(subject, [old, replacement]);
    expect(projection.replacedOpIds.has(old.opId)).toBe(true);
    expect(projection.facts.get('name')?.has(JSON.stringify(['Alice Old']))).toBe(false);
    expect(projection.facts.get('name')?.has(JSON.stringify(['Alice']))).toBe(true);
  });
});
