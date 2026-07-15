import { readFileSync } from 'node:fs';
import { finalizeEvent, getPublicKey } from 'nostr-tools';
import { describe, expect, it } from 'vitest';
import {
  IDENTITY_ADMIN_CAPABILITIES,
  IDENTITY_APP_KEY_CAPABILITIES,
  IDENTITY_CAPABILITY_ADMIN,
  IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
  IDENTITY_CAPABILITY_RECOVER,
  IDENTITY_CAPABILITY_WRITE,
  IDENTITY_PURPOSE_APP,
  IDENTITY_PURPOSE_FIPS_TRANSPORT,
  IDENTITY_PURPOSE_RECOVERY,
  IDENTITY_PURPOSE_REMOTE_SIGNER,
  IDENTITY_GRAPH_KEY_ACCEPTANCE_TYPE,
  IDENTITY_GRAPH_LINK_REQUEST_TYPE,
  IDENTITY_GRAPH_ROSTER_TYPE,
  buildIdentityKeyAcceptanceDraft,
  buildIdentityLinkRequestEvent,
  buildIdentityRosterOpDraft,
  identityKey,
  parseIdentityKeyAcceptanceEvent,
  parseIdentityLinkRequestEvent,
  parseIdentityRosterOpEvent,
  projectIdentityKeyAcceptances,
  projectIdentityRoster,
  resolveFipsTransportIdentityBindings,
  resolveTrustedFipsTransportIdentityBindings,
} from '../src/identityGraph';
import { fact } from '../src/factEvents';
import type { IdentityEventDraft } from '../src/identityGraph';
import type { NostrEvent } from '../src/utils';

const identity = '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a';
const adminPubkey = 'a'.repeat(64);
const appPubkey = 'b'.repeat(64);
const otherPubkey = 'c'.repeat(64);
const linkRequestDeviceSecret = new Uint8Array(32).fill(1);
const linkRequestInviteSecret = new Uint8Array(32).fill(2);
const linkRequestDevicePubkey = getPublicKey(linkRequestDeviceSecret);
const linkRequestInvitePubkey = getPublicKey(linkRequestInviteSecret);
const fixtureLinkRequest = JSON.parse(
  readFileSync(new URL('../../testdata/identity-link-request.json', import.meta.url), 'utf8'),
) as NostrEvent;
const fipsTransportFixture = JSON.parse(
  readFileSync(new URL('../../testdata/fips-transport-identity-v1.json', import.meta.url), 'utf8'),
) as {
  identity: string;
  purpose: string;
  keys: Record<'adminSecretKey' | 'transportSecretKey' | 'otherSecretKey' | 'transportPubkey', string>;
  timestamps: Record<'bootstrap' | 'addTransport' | 'acceptTransport' | 'tombstoneTransport', number>;
};

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
    expect(draft.tags).toContainEqual(['type', IDENTITY_GRAPH_ROSTER_TYPE]);
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

  it('signs identity roster extension facts without changing roster parsing', () => {
    const draft = buildIdentityRosterOpDraft({
      signerPubkey: adminPubkey,
      identity,
      createdAt: 10,
      clientNonce: 'nonce-1',
      extensionFacts: [fact('encrypted_device_labels', ['ciphertext-v1'])],
      op: {
        op: 'add_key',
        key: identityKey(adminPubkey, {
          addedAt: 10,
          purposes: [IDENTITY_PURPOSE_APP],
          capabilities: IDENTITY_ADMIN_CAPABILITIES,
        }),
      },
    });

    expect(draft.tags).toContainEqual(['encrypted_device_labels', 'ciphertext-v1']);
    const parsed = parseIdentityRosterOpEvent(eventFromDraft(draft, eventId('9'), adminPubkey));
    expect(parsed.content.op).toMatchObject({
      op: 'add_key',
      key: { pubkey: adminPubkey },
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

    expect(draft.tags).toContainEqual(['type', IDENTITY_GRAPH_KEY_ACCEPTANCE_TYPE]);
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

  it('resolves only active self-accepted FIPS transport bindings', () => {
    const fixture = fipsTransportFixture;
    const pubkey = (name: 'adminSecretKey' | 'transportSecretKey' | 'otherSecretKey') => (
      getPublicKey(Uint8Array.from(Buffer.from(fixture.keys[name], 'hex')))
    );
    const admin = pubkey('adminSecretKey');
    const transport = pubkey('transportSecretKey');
    const other = pubkey('otherSecretKey');
    const signedRoster = (options: Parameters<typeof buildIdentityRosterOpDraft>[0], id: string) => (
      parseIdentityRosterOpEvent(eventFromDraft(buildIdentityRosterOpDraft(options), id, options.signerPubkey))
    );
    expect([fixture.purpose, transport]).toEqual([
      IDENTITY_PURPOSE_FIPS_TRANSPORT,
      fixture.keys.transportPubkey,
    ]);

    const bootstrap = signedRoster({
      signerPubkey: admin,
      identity: fixture.identity,
      createdAt: fixture.timestamps.bootstrap,
      clientNonce: 'fips-bootstrap',
      op: { op: 'add_key', key: identityKey(admin, {
        addedAt: fixture.timestamps.bootstrap,
        purposes: [IDENTITY_PURPOSE_APP],
        capabilities: IDENTITY_ADMIN_CAPABILITIES,
      }) },
    }, eventId('6'));
    const addTransport = signedRoster({
      signerPubkey: admin,
      identity: fixture.identity,
      parents: [bootstrap.opId],
      createdAt: fixture.timestamps.addTransport,
      clientNonce: 'fips-add',
      op: { op: 'add_key', key: identityKey(transport, {
        addedAt: fixture.timestamps.addTransport,
        purposes: [IDENTITY_PURPOSE_FIPS_TRANSPORT],
        capabilities: [],
      }) },
    }, eventId('7'));
    const signedAcceptance = (rosterOpId: string, id: string, acceptedAt: number) => (
      parseIdentityKeyAcceptanceEvent(eventFromDraft(buildIdentityKeyAcceptanceDraft({
        signerPubkey: transport,
        identity: fixture.identity,
        rosterOpId,
        purposes: [IDENTITY_PURPOSE_FIPS_TRANSPORT],
        acceptedAt,
        clientNonce: `fips-${id}`,
      }), eventId(id), transport))
    );
    const acceptance = signedAcceptance(
      addTransport.opId,
      '8',
      fixture.timestamps.acceptTransport,
    );
    const resolve = (ops = [bootstrap, addTransport], acceptances = [acceptance]) => (
      resolveTrustedFipsTransportIdentityBindings(fixture.identity, ops, acceptances)
    );

    expect(resolve()).toEqual([{
      transportPubkey: transport,
      rosterOpId: addTransport.opId,
      acceptanceId: acceptance.acceptanceId,
    }]);
    expect(resolve(undefined, [])).toEqual([]);
    expect(resolve(undefined, [{ ...acceptance, signerPubkey: other }])).toEqual([]);
    expect(resolve(undefined, [{
      ...acceptance,
      content: { ...acceptance.content, rosterOpId: eventId('9') },
    }])).toEqual([]);
    if (addTransport.content.op.op !== 'add_key') throw new Error('expected add_key');
    const extraKeyPurpose = {
      ...addTransport,
      content: {
        ...addTransport.content,
        op: {
          ...addTransport.content.op,
          key: {
            ...addTransport.content.op.key,
            purposes: [IDENTITY_PURPOSE_APP, IDENTITY_PURPOSE_FIPS_TRANSPORT],
          },
        },
      },
    };
    expect(resolve([bootstrap, extraKeyPurpose])).toEqual([]);
    expect(resolve(undefined, [{
      ...acceptance,
      content: {
        ...acceptance.content,
        purposes: [IDENTITY_PURPOSE_APP, IDENTITY_PURPOSE_FIPS_TRANSPORT],
      },
    }])).toEqual([]);
    const supersedingWrongLink = signedAcceptance(
      eventId('9'),
      'd',
      fixture.timestamps.acceptTransport + 1,
    );
    expect(resolve(undefined, [acceptance, supersedingWrongLink])).toEqual([]);
    const renewed = signedAcceptance(
      addTransport.opId,
      'e',
      fixture.timestamps.acceptTransport + 2,
    );
    expect(resolve(undefined, [acceptance, renewed])[0]?.acceptanceId).toBe(renewed.acceptanceId);

    const laterOp = (op: Parameters<typeof buildIdentityRosterOpDraft>[0]['op'], id: string) => signedRoster({
      signerPubkey: admin,
      identity: fixture.identity,
      parents: [addTransport.opId],
      createdAt: fixture.timestamps.tombstoneTransport,
      clientNonce: `fips-${id}`,
      op,
    }, eventId(id));
    const grantWrite = laterOp({
      op: 'set_key_capabilities',
      pubkey: transport,
      capabilities: [IDENTITY_CAPABILITY_WRITE],
    }, 'b');
    const tombstone = laterOp({ op: 'tombstone_key', pubkey: transport }, 'a');
    expect(resolve([bootstrap, addTransport, grantWrite])).toEqual([]);
    expect(resolve([bootstrap, addTransport, tombstone])).toEqual([]);
    const readd = signedRoster({
      signerPubkey: admin,
      identity: fixture.identity,
      parents: [tombstone.opId],
      createdAt: fixture.timestamps.tombstoneTransport + 1,
      clientNonce: 'fips-readd',
      op: { op: 'add_key', key: identityKey(transport, {
        addedAt: fixture.timestamps.tombstoneTransport + 1,
        purposes: [IDENTITY_PURPOSE_FIPS_TRANSPORT],
        capabilities: [],
      }) },
    }, eventId('c'));
    expect(resolve([bootstrap, addTransport, tombstone, readd])).toEqual([]);
  });

  it('verifies relay events before resolving FIPS transport bindings', () => {
    const fixture = fipsTransportFixture;
    const secret = (name: 'adminSecretKey' | 'transportSecretKey') => (
      Uint8Array.from(Buffer.from(fixture.keys[name], 'hex'))
    );
    const adminSecret = secret('adminSecretKey');
    const transportSecret = secret('transportSecretKey');
    const admin = getPublicKey(adminSecret);
    const transport = getPublicKey(transportSecret);
    const signedRoster = (
      options: Parameters<typeof buildIdentityRosterOpDraft>[0],
      signerSecretKey: Uint8Array,
    ) => finalizeEvent(buildIdentityRosterOpDraft(options), signerSecretKey);
    const bootstrap = signedRoster({
      signerPubkey: admin,
      identity: fixture.identity,
      createdAt: fixture.timestamps.bootstrap,
      clientNonce: 'verified-bootstrap',
      op: { op: 'add_key', key: identityKey(admin, {
        addedAt: fixture.timestamps.bootstrap,
        purposes: [IDENTITY_PURPOSE_APP],
        capabilities: IDENTITY_ADMIN_CAPABILITIES,
      }) },
    }, adminSecret);
    const addTransport = signedRoster({
      signerPubkey: admin,
      identity: fixture.identity,
      parents: [bootstrap.id],
      createdAt: fixture.timestamps.addTransport,
      clientNonce: 'verified-add',
      op: { op: 'add_key', key: identityKey(transport, {
        addedAt: fixture.timestamps.addTransport,
        purposes: [IDENTITY_PURPOSE_FIPS_TRANSPORT],
        capabilities: [],
      }) },
    }, adminSecret);
    const signedAcceptance = (rosterOpId: string, nonce: string, acceptedAt: number) => (
      finalizeEvent(buildIdentityKeyAcceptanceDraft({
        signerPubkey: transport,
        identity: fixture.identity,
        rosterOpId,
        purposes: [IDENTITY_PURPOSE_FIPS_TRANSPORT],
        acceptedAt,
        clientNonce: nonce,
      }), transportSecret)
    );
    const acceptance = signedAcceptance(
      addTransport.id,
      'verified-accept',
      fixture.timestamps.acceptTransport,
    );
    const resolve = (
      rosterEvents = [bootstrap, addTransport],
      acceptanceEvents = [acceptance],
    ) => resolveFipsTransportIdentityBindings(fixture.identity, rosterEvents, acceptanceEvents);
    const expected = [{
      transportPubkey: transport,
      rosterOpId: addTransport.id,
      acceptanceId: acceptance.id,
    }];

    expect(resolve()).toEqual(expected);
    expect(resolve([bootstrap, { ...addTransport, sig: '0'.repeat(128) }])).toEqual([]);
    expect(resolve([bootstrap, { ...addTransport, id: eventId('0') }])).toEqual([]);
    expect(resolve(undefined, [{ ...acceptance, sig: '0'.repeat(128) }])).toEqual([]);

    const wrongLink = signedAcceptance(
      eventId('9'),
      'verified-wrong-link',
      fixture.timestamps.acceptTransport + 1,
    );
    expect(resolve(undefined, [acceptance, { ...wrongLink, sig: '0'.repeat(128) }])).toEqual(expected);
    expect(resolve(undefined, [acceptance, wrongLink])).toEqual([]);
    const renewed = signedAcceptance(
      addTransport.id,
      'verified-renewed',
      fixture.timestamps.acceptTransport + 2,
    );
    expect(resolve(undefined, [acceptance, wrongLink, renewed])[0]?.acceptanceId).toBe(renewed.id);

    const tied = [
      signedAcceptance(addTransport.id, 'verified-tie-correct', fixture.timestamps.acceptTransport + 3),
      signedAcceptance(eventId('8'), 'verified-tie-wrong', fixture.timestamps.acceptTransport + 3),
    ].sort((left, right) => left.id.localeCompare(right.id));
    const tiedResult = resolve(undefined, tied);
    const tiedLatestIsCorrect = tied[1].tags.some((tag) => (
      tag[0] === 'roster_op_id' && tag[1] === addTransport.id
    ));
    expect(tiedResult.map((binding) => binding.acceptanceId)).toEqual(
      tiedLatestIsCorrect ? [tied[1].id] : [],
    );

    const nonEmpty = finalizeEvent({
      kind: bootstrap.kind,
      tags: bootstrap.tags,
      created_at: bootstrap.created_at,
      content: 'forbidden',
    }, adminSecret);
    expect(() => parseIdentityRosterOpEvent(nonEmpty)).toThrow(/empty content/);
    const nonEmptyAcceptance = finalizeEvent({
      kind: acceptance.kind,
      tags: acceptance.tags,
      created_at: acceptance.created_at,
      content: 'forbidden',
    }, transportSecret);
    expect(() => parseIdentityKeyAcceptanceEvent(nonEmptyAcceptance)).toThrow(/empty content/);
    expect(resolve(undefined, [acceptance, nonEmptyAcceptance])).toEqual(expected);
  });

  it('builds encrypted device link requests as neutral identity events', () => {
    const event = buildIdentityLinkRequestEvent({
      signerSecretKey: linkRequestDeviceSecret,
      identity,
      adminPubkey,
      invitePubkey: linkRequestInvitePubkey,
      requestedAt: 21,
      clientNonce: 'nonce-link',
      label: 'Phone',
    });

    expect(event.kind).toBe(7368);
    expect(event.content).not.toBe('');
    expect(event.created_at).toBe(21);
    expect(event.pubkey).toBe(linkRequestDevicePubkey);
    expect(event.tags).toContainEqual(['type', IDENTITY_GRAPH_LINK_REQUEST_TYPE]);
    expect(event.tags).toContainEqual(['i', identity, 'subject']);
    expect(event.tags).toContainEqual(['p', linkRequestInvitePubkey]);
    expect(event.tags.some((tag) => tag[0] === 'admin_pubkey')).toBe(false);
    expect(event.tags.some((tag) => tag[0] === 'key_pubkey')).toBe(false);
    expect(event.tags.some((tag) => tag[0] === 'joining_pubkey')).toBe(false);
    expect(event.tags.some((tag) => tag[0] === 'link_secret_hash')).toBe(false);

    const signed = parseIdentityLinkRequestEvent(event, { inviteSecretKey: linkRequestInviteSecret });
    expect(signed.content).toEqual({
      identity,
      adminPubkey,
      invitePubkey: linkRequestInvitePubkey,
      joiningPubkey: linkRequestDevicePubkey,
      clientNonce: 'nonce-link',
      requestedAt: 21,
      label: 'Phone',
    });
  });

  it('parses the shared TS/Rust identity link-request fixture', () => {
    const signed = parseIdentityLinkRequestEvent(fixtureLinkRequest, {
      inviteSecretKey: linkRequestInviteSecret,
    });

    expect(signed.signerPubkey).toBe(linkRequestDevicePubkey);
    expect(signed.content).toEqual({
      identity,
      adminPubkey,
      invitePubkey: linkRequestInvitePubkey,
      joiningPubkey: linkRequestDevicePubkey,
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

    const linkEvent = buildIdentityLinkRequestEvent({
      signerSecretKey: linkRequestDeviceSecret,
      identity,
      adminPubkey,
      invitePubkey: linkRequestInvitePubkey,
      requestedAt: 21,
      clientNonce: 'nonce-link',
    });
    expect(() => parseIdentityLinkRequestEvent({
      ...linkEvent,
      pubkey: adminPubkey,
    }, { inviteSecretKey: linkRequestInviteSecret })).toThrow();
    expect(() => parseIdentityLinkRequestEvent(
      linkEvent,
      { inviteSecretKey: new Uint8Array(32).fill(3) },
    )).toThrow();
  });
});
