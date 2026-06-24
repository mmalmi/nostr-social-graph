import { describe, expect, it } from 'vitest';
import {
  IDENTITY_ADMIN_CAPABILITIES,
  IDENTITY_APP_KEY_CAPABILITIES,
  IDENTITY_CAPABILITY_ADMIN,
  IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
  IDENTITY_CAPABILITY_WRITE,
  IDENTITY_PURPOSE_APP,
  IDENTITY_PURPOSE_REMOTE_SIGNER,
  NOSTR_IDENTITY_KEY_ACCEPTANCE_TYPE,
  NOSTR_IDENTITY_ROSTER_TYPE,
  buildIdentityKeyAcceptanceDraft,
  buildIdentityRosterOpDraft,
  identityKey,
  parseIdentityKeyAcceptanceEvent,
  parseIdentityRosterOpEvent,
  projectIdentityKeyAcceptances,
  projectIdentityRoster,
} from '../src/identityGraph';
import type { IdentityEventDraft } from '../src/identityGraph';
import type { NostrEvent } from '../src/utils';

const identity = '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a';
const adminPubkey = 'a'.repeat(64);
const appPubkey = 'b'.repeat(64);
const otherPubkey = 'c'.repeat(64);

function eventId(byte: string): string {
  return byte.repeat(64);
}

function eventFromDraft(draft: IdentityEventDraft, id: string, pubkey: string): NostrEvent {
  return {
    ...draft,
    id,
    pubkey,
    sig: eventId('f'),
  };
}

describe('identity graph', () => {
  it('builds and parses neutral roster fact events', () => {
    const draft = buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      createdAt: 10,
      clientNonce: 'nonce-1',
      op: {
        op: 'add_key',
        key: identityKey(adminPubkey, {
          addedAt: 10,
          label: 'Admin key',
          purposes: [IDENTITY_PURPOSE_APP],
          capabilities: IDENTITY_ADMIN_CAPABILITIES,
        }),
      },
    });

    expect(draft.content).toBe('');
    expect(draft.tags).toContainEqual(['type', NOSTR_IDENTITY_ROSTER_TYPE]);
    expect(draft.tags).toContainEqual(['op', 'add_key']);
    expect(draft.tags).toContainEqual(['key_pubkey', adminPubkey]);
    expect(draft.tags).toContainEqual(['key_capability', IDENTITY_CAPABILITY_ADMIN]);
    expect(draft.tags).toContainEqual(['p', adminPubkey]);

    const parsed = parseIdentityRosterOpEvent(eventFromDraft(draft, eventId('1'), adminPubkey));
    expect(parsed.content.identity).toBe(identity);
    expect(parsed.content.actorPubkey).toBe(adminPubkey);
    expect(parsed.content.op).toEqual({
      op: 'add_key',
      key: {
        pubkey: adminPubkey,
        purposes: [IDENTITY_PURPOSE_APP],
        capabilities: IDENTITY_ADMIN_CAPABILITIES,
        addedAt: 10,
        label: 'Admin key',
      },
    });
  });

  it('projects admin-authorized app keys and rejects non-admin roster edits', () => {
    const bootstrap = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      createdAt: 10,
      clientNonce: 'nonce-1',
      op: {
        op: 'add_key',
        key: identityKey(adminPubkey, {
          addedAt: 10,
          capabilities: IDENTITY_ADMIN_CAPABILITIES,
        }),
      },
    }), eventId('1'), adminPubkey));
    const addApp = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      parents: [bootstrap.opId],
      createdAt: 11,
      clientNonce: 'nonce-2',
      op: {
        op: 'add_key',
        key: identityKey(appPubkey, {
          addedAt: 11,
          label: 'Phone',
          capabilities: IDENTITY_APP_KEY_CAPABILITIES,
        }),
      },
    }), eventId('2'), adminPubkey));
    const rejected = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: appPubkey,
      identity,
      parents: [addApp.opId],
      createdAt: 12,
      clientNonce: 'nonce-3',
      op: {
        op: 'set_key_capabilities',
        pubkey: appPubkey,
        capabilities: [IDENTITY_CAPABILITY_ADMIN],
      },
    }), eventId('3'), appPubkey));

    const projection = projectIdentityRoster(identity, [rejected, addApp, bootstrap]);
    expect(projection.acceptedOpIds).toEqual([bootstrap.opId, addApp.opId]);
    expect(projection.rejectedOpIds).toEqual([rejected.opId]);
    expect(projection.activeKeys[adminPubkey]?.capabilities).toContain(IDENTITY_CAPABILITY_ADMIN);
    expect(projection.activeKeys[appPubkey]?.capabilities).toEqual(IDENTITY_APP_KEY_CAPABILITIES);
  });

  it('projects secret epochs and same-signer wrap repairs', () => {
    const bootstrap = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      createdAt: 10,
      clientNonce: 'nonce-1',
      op: {
        op: 'add_key',
        key: identityKey(adminPubkey, {
          addedAt: 10,
          capabilities: IDENTITY_ADMIN_CAPABILITIES,
        }),
      },
    }), eventId('1'), adminPubkey));
    const rotate = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      parents: [bootstrap.opId],
      createdAt: 11,
      clientNonce: 'nonce-2',
      op: {
        op: 'rotate_secret_epoch',
        epoch: 1,
        wrappedSecrets: {
          [adminPubkey]: 'wrap-admin',
        },
      },
    }), eventId('2'), adminPubkey));
    const repair = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      parents: [rotate.opId],
      createdAt: 12,
      clientNonce: 'nonce-3',
      op: {
        op: 'repair_secret_wraps',
        epoch: 1,
        wrappedSecrets: {
          [appPubkey]: 'wrap-app',
        },
      },
    }), eventId('3'), adminPubkey));

    const projection = projectIdentityRoster(identity, [bootstrap, rotate, repair]);
    expect(projection.secretEpochs['1']).toEqual({
      epoch: 1,
      createdAt: 11,
      signedByPubkey: adminPubkey,
      wrappedSecrets: {
        [adminPubkey]: 'wrap-admin',
        [appPubkey]: 'wrap-app',
      },
    });
  });

  it('builds key self-acceptance fact events', () => {
    const rosterOpId = eventId('2');
    const draft = buildIdentityKeyAcceptanceDraft({
      signerPubkey: appPubkey,
      identity,
      rosterOpId,
      purposes: [IDENTITY_PURPOSE_REMOTE_SIGNER, IDENTITY_PURPOSE_APP],
      acceptedAt: 20,
      clientNonce: 'nonce-4',
    });

    expect(draft.tags).toContainEqual(['type', NOSTR_IDENTITY_KEY_ACCEPTANCE_TYPE]);
    expect(draft.tags).toContainEqual(['key_pubkey', appPubkey]);
    expect(draft.tags).toContainEqual(['purpose', IDENTITY_PURPOSE_APP]);
    expect(draft.tags).toContainEqual(['purpose', IDENTITY_PURPOSE_REMOTE_SIGNER]);
    expect(draft.tags).toContainEqual(['roster_op_id', rosterOpId]);

    const signed = parseIdentityKeyAcceptanceEvent(eventFromDraft(draft, eventId('4'), appPubkey));
    const projection = projectIdentityKeyAcceptances(identity, [signed]);
    expect(projection.acceptedAcceptanceIds).toEqual([signed.acceptanceId]);
    expect(projection.acceptedKeys[appPubkey]?.purposes).toEqual([
      IDENTITY_PURPOSE_APP,
      IDENTITY_PURPOSE_REMOTE_SIGNER,
    ]);
  });

  it('rejects invalid identity facts', () => {
    expect(() => buildIdentityKeyAcceptanceDraft({
      signerPubkey: appPubkey,
      identity,
      purposes: [],
      acceptedAt: 20,
      clientNonce: 'nonce-4',
    })).toThrow(/purposes must not be empty/);

    const draft = buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      createdAt: 10,
      clientNonce: 'nonce-1',
      op: {
        op: 'add_key',
        key: identityKey(otherPubkey, {
          addedAt: 10,
          capabilities: [IDENTITY_CAPABILITY_WRITE, IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS],
        }),
      },
    });
    expect(() => parseIdentityRosterOpEvent(eventFromDraft(draft, eventId('5'), appPubkey))).toThrow(
      /actor signer mismatch/,
    );
  });
});
