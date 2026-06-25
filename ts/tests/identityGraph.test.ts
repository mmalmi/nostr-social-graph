import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  IDENTITY_ADMIN_CAPABILITIES,
  IDENTITY_APP_KEY_CAPABILITIES,
  IDENTITY_CAPABILITY_ADMIN,
  IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
  IDENTITY_CAPABILITY_RECOVER,
  IDENTITY_CAPABILITY_WRITE,
  IDENTITY_PURPOSE_APP,
  IDENTITY_PURPOSE_RECOVERY,
  IDENTITY_PURPOSE_REMOTE_SIGNER,
  NOSTR_IDENTITY_KEY_ACCEPTANCE_TYPE,
  NOSTR_IDENTITY_LINK_REQUEST_TYPE,
  NOSTR_IDENTITY_ROSTER_TYPE,
  buildIdentityKeyAcceptanceDraft,
  buildIdentityLinkRequestDraft,
  buildIdentityRosterOpDraft,
  identityKey,
  parseIdentityKeyAcceptanceEvent,
  parseIdentityLinkRequestEvent,
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
const fixtureLinkRequest = JSON.parse(
  readFileSync(new URL('../../testdata/identity-link-request.json', import.meta.url), 'utf8'),
) as NostrEvent;

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

  it('allows recovery keys to add and remove app keys and rewrap secrets', () => {
    const recoveryPubkey = otherPubkey;
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
    const addRecovery = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      parents: [bootstrap.opId],
      createdAt: 11,
      clientNonce: 'nonce-2',
      op: {
        op: 'add_key',
        key: identityKey(recoveryPubkey, {
          addedAt: 11,
          purposes: [IDENTITY_PURPOSE_RECOVERY],
          capabilities: [IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS, IDENTITY_CAPABILITY_RECOVER],
        }),
      },
    }), eventId('2'), adminPubkey));
    const recoverAppKey = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: recoveryPubkey,
      identity,
      parents: [addRecovery.opId],
      createdAt: 12,
      clientNonce: 'nonce-3',
      op: {
        op: 'add_key',
        key: identityKey(appPubkey, {
          addedAt: 12,
          capabilities: IDENTITY_APP_KEY_CAPABILITIES,
        }),
      },
    }), eventId('3'), recoveryPubkey));
    const recoverAdmin = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: recoveryPubkey,
      identity,
      parents: [recoverAppKey.opId],
      createdAt: 13,
      clientNonce: 'nonce-4',
      op: {
        op: 'add_key',
        key: identityKey('d'.repeat(64), {
          addedAt: 13,
          capabilities: IDENTITY_ADMIN_CAPABILITIES,
        }),
      },
    }), eventId('4'), recoveryPubkey));
    const rotate = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: recoveryPubkey,
      identity,
      parents: [recoverAppKey.opId],
      createdAt: 14,
      clientNonce: 'nonce-5',
      op: {
        op: 'rotate_secret_epoch',
        epoch: 2,
        wrappedSecrets: {
          [appPubkey]: 'wrap-app',
        },
      },
    }), eventId('5'), recoveryPubkey));
    const repair = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: recoveryPubkey,
      identity,
      parents: [rotate.opId],
      createdAt: 15,
      clientNonce: 'nonce-6',
      op: {
        op: 'repair_secret_wraps',
        epoch: 2,
        wrappedSecrets: {
          [adminPubkey]: 'wrap-admin',
        },
      },
    }), eventId('6'), recoveryPubkey));
    const removeAppKey = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: recoveryPubkey,
      identity,
      parents: [repair.opId],
      createdAt: 16,
      clientNonce: 'nonce-7',
      op: {
        op: 'tombstone_key',
        pubkey: appPubkey,
        reason: 'recovered',
      },
    }), eventId('7'), recoveryPubkey));

    const projection = projectIdentityRoster(identity, [
      bootstrap,
      addRecovery,
      recoverAppKey,
      recoverAdmin,
      rotate,
      repair,
      removeAppKey,
    ]);
    expect(projection.acceptedOpIds).toEqual([
      bootstrap.opId,
      addRecovery.opId,
      recoverAppKey.opId,
      recoverAdmin.opId,
      rotate.opId,
      repair.opId,
      removeAppKey.opId,
    ]);
    expect(projection.rejectedOpIds).toEqual([]);
    expect(projection.activeKeys[appPubkey]).toBeUndefined();
    expect(projection.activeKeys['d'.repeat(64)]?.capabilities).toContain(IDENTITY_CAPABILITY_ADMIN);
    expect(projection.tombstones[appPubkey]?.reason).toBe('recovered');
    expect(projection.secretEpochs['2']?.wrappedSecrets).toEqual({
      [adminPubkey]: 'wrap-admin',
      [appPubkey]: 'wrap-app',
    });
  });

  it('lets recovery with denied decrypt permission add an app key without rotating secrets', () => {
    const recoveryPubkey = otherPubkey;
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
    const addRecovery = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      parents: [bootstrap.opId],
      createdAt: 11,
      clientNonce: 'nonce-2',
      op: {
        op: 'add_key',
        key: identityKey(recoveryPubkey, {
          addedAt: 11,
          purposes: [IDENTITY_PURPOSE_REMOTE_SIGNER],
          capabilities: [IDENTITY_CAPABILITY_RECOVER],
        }),
      },
    }), eventId('2'), adminPubkey));
    const recoverAppKey = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: recoveryPubkey,
      identity,
      parents: [addRecovery.opId],
      createdAt: 12,
      clientNonce: 'nonce-3',
      op: {
        op: 'add_key',
        key: identityKey(appPubkey, {
          addedAt: 12,
          capabilities: IDENTITY_ADMIN_CAPABILITIES,
        }),
      },
    }), eventId('3'), recoveryPubkey));
    const rotate = parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft({
      signerPubkey: recoveryPubkey,
      identity,
      parents: [recoverAppKey.opId],
      createdAt: 13,
      clientNonce: 'nonce-4',
      op: {
        op: 'rotate_secret_epoch',
        epoch: 2,
        wrappedSecrets: { [appPubkey]: 'wrap-app' },
      },
    }), eventId('4'), recoveryPubkey));

    const projection = projectIdentityRoster(identity, [bootstrap, addRecovery, recoverAppKey, rotate]);
    expect(projection.activeKeys[appPubkey]?.capabilities).toContain(IDENTITY_CAPABILITY_ADMIN);
    expect(projection.secretEpochs['2']).toBeUndefined();
    expect(projection.rejectedOpIds).toEqual([rotate.opId]);
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

  it('builds device link requests as neutral identity fact events', () => {
    const draft = buildIdentityLinkRequestDraft({
      signerPubkey: appPubkey,
      identity,
      adminPubkey,
      linkSecretHash: 'hash-from-invite',
      requestedAt: 21,
      clientNonce: 'nonce-link',
      label: 'Phone',
    });

    expect(draft.kind).toBe(7368);
    expect(draft.content).toBe('');
    expect(draft.created_at).toBe(21);
    expect(draft.tags).toContainEqual(['type', NOSTR_IDENTITY_LINK_REQUEST_TYPE]);
    expect(draft.tags).toContainEqual(['admin_pubkey', adminPubkey]);
    expect(draft.tags).toContainEqual(['key_pubkey', appPubkey]);
    expect(draft.tags).toContainEqual(['link_secret_hash', 'hash-from-invite']);
    expect(draft.tags).toContainEqual(['key_label', 'Phone']);
    expect(draft.tags).toContainEqual(['p', adminPubkey]);
    expect(draft.tags).toContainEqual(['p', appPubkey]);

    const signed = parseIdentityLinkRequestEvent(eventFromDraft(draft, eventId('6'), appPubkey));
    expect(signed.content).toEqual({
      schema: 1,
      identity,
      adminPubkey,
      keyPubkey: appPubkey,
      linkSecretHash: 'hash-from-invite',
      clientNonce: 'nonce-link',
      requestedAt: 21,
      label: 'Phone',
    });
  });

  it('parses the shared TS/Rust identity link-request fixture', () => {
    const signed = parseIdentityLinkRequestEvent(fixtureLinkRequest);

    expect(signed.requestId).toBe('7d8a323b036150abc71a886f71898107306632b126e8eb4e9ac3545790dd22fc');
    expect(signed.signerPubkey).toBe('84bf7562262bbd6940085748f3be6afa52ae317155181ece31b66351ccffa4b0');
    expect(signed.content).toEqual({
      schema: 1,
      identity,
      adminPubkey,
      keyPubkey: '84bf7562262bbd6940085748f3be6afa52ae317155181ece31b66351ccffa4b0',
      linkSecretHash: 'fixture-secret-hash',
      clientNonce: 'fixture-link-request',
      requestedAt: 1720000021,
      label: 'Fixture Phone',
    });
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

    const linkDraft = buildIdentityLinkRequestDraft({
      signerPubkey: appPubkey,
      identity,
      adminPubkey,
      linkSecretHash: 'hash-from-invite',
      requestedAt: 21,
      clientNonce: 'nonce-link',
    });
    expect(() => parseIdentityLinkRequestEvent(eventFromDraft(linkDraft, eventId('6'), adminPubkey))).toThrow(
      /link request signer mismatch/,
    );
  });
});
