import { describe, expect, it } from 'vitest';
import {
  IDENTITY_OP_KIND,
  IDENTITY_SNAPSHOT_KIND,
  buildIdentityOpDraft,
  buildIdentitySnapshotDraft,
  identityFact,
  parseIdentityOpEvent,
  parseIdentitySnapshotEvent,
  projectIdentityOps,
  type IdentityOpLinks,
} from '../src/identity';
import type { NostrEvent } from '../src/utils';

const subject = '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a';
const other = '9f3d3b6f-4f5e-4f8a-9d6e-7c2a7c6c9f11';
const pubkey = '4f355bdcb7c27f8f3e4f6c4ec8f996d45b33f2d9c93a9d5c7aa9a6e1a4e7f8aa';

function eventId(byte: string): string {
  return byte.repeat(64);
}

function eventFromDraft(draft: { kind: number; content: string; tags: string[][] }, id: string): NostrEvent {
  return {
    ...draft,
    id,
    pubkey,
    sig: eventId('b'),
    created_at: 123,
  };
}

describe('identity events', () => {
  it('builds and parses tag-only op drafts', () => {
    const draft = buildIdentityOpDraft(
      subject,
      [
        identityFact('name', ['Alice']),
        identityFact('controls', [pubkey]),
        identityFact('same_as', [other]),
      ],
      { prev: [eventId('1')] },
      { externalIdentifiers: ['nip05:alice@example.com'] },
    );

    expect(draft.kind).toBe(IDENTITY_OP_KIND);
    expect(draft.content).toBe('');
    expect(draft.tags).toContainEqual(['i', subject, 'subject']);
    expect(draft.tags).toContainEqual(['p', pubkey]);
    expect(draft.tags).toContainEqual(['i', other]);
    expect(draft.tags).toContainEqual(['i', 'nip05:alice@example.com']);

    const op = parseIdentityOpEvent(eventFromDraft(draft, eventId('2')));
    expect(op.subject).toBe(subject);
    expect(op.prev).toEqual([eventId('1')]);
    expect(op.pubkeys.has(pubkey)).toBe(true);
    expect(op.externalIdentifiers.has('nip05:alice@example.com')).toBe(true);
    expect(op.mentionedSubjects.has(other)).toBe(true);
    expect(op.facts).toContainEqual(identityFact('name', ['Alice']));
  });

  it('builds canonical bare-uuid snapshot tags', () => {
    const draft = buildIdentitySnapshotDraft(
      subject,
      [
        identityFact('same_as', [other]),
        identityFact('name', ['Alice']),
        identityFact('controls', [pubkey]),
      ],
      [eventId('3')],
    );

    expect(draft.kind).toBe(IDENTITY_SNAPSHOT_KIND);
    expect(draft.tags).toContainEqual(['d', subject]);
    const sorted = [...draft.tags].sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right)));
    expect(draft.tags).toEqual(sorted);

    const snapshot = parseIdentitySnapshotEvent(eventFromDraft(draft, eventId('4')));
    expect(snapshot.subject).toBe(subject);
    expect(snapshot.heads).toEqual([eventId('3')]);
  });

  it('projects heads through optional prev links', () => {
    const first = parseIdentityOpEvent(
      eventFromDraft(buildIdentityOpDraft(subject, [identityFact('name', ['Alice'])]), eventId('5')),
    );
    const second = parseIdentityOpEvent(
      eventFromDraft(
        buildIdentityOpDraft(subject, [identityFact('picture', ['https://example.com/a.jpg'])], {
          prev: [first.opId],
        }),
        eventId('6'),
      ),
    );

    const projection = projectIdentityOps(subject, [first, second]);
    expect(projection.appliedOpIds.size).toBe(2);
    expect(projection.heads).toEqual(new Set([second.opId]));
    expect(projection.facts.get('name')?.has(JSON.stringify(['Alice']))).toBe(true);
  });

  it('can mark replaced ops without mutating signed tags', () => {
    const old = parseIdentityOpEvent(
      eventFromDraft(buildIdentityOpDraft(subject, [identityFact('name', ['Alice Old'])]), eventId('7')),
    );
    const links: IdentityOpLinks = { replace: [old.opId] };
    const replacement = parseIdentityOpEvent(
      eventFromDraft(buildIdentityOpDraft(subject, [identityFact('name', ['Alice'])], links), eventId('8')),
    );

    const projection = projectIdentityOps(subject, [old, replacement]);
    expect(projection.replacedOpIds.has(old.opId)).toBe(true);
    expect(projection.facts.get('name')?.has(JSON.stringify(['Alice Old']))).toBe(false);
    expect(projection.facts.get('name')?.has(JSON.stringify(['Alice']))).toBe(true);
  });
});
