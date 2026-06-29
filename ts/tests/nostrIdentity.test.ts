import { describe, expect, it } from 'vitest';
import { generateSecretKey, getPublicKey } from 'nostr-tools';
import {
  APP_KEY_ADMIN_CAPABILITIES,
  NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT,
  appKeyFacet,
  buildNostrIdentityRosterOpEvent,
  encryptedDeviceLabelPayloadsFromNostrIdentityRosterOpEvent,
  parseNostrIdentityRosterOpEvent,
  projectNostrIdentityRoster,
} from '../src';

const profileId = '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a';

describe('NostrIdentity', () => {
  it('stores app-key names only in encrypted extension facts', () => {
    const secretKey = generateSecretKey();
    const pubkey = getPublicKey(secretKey);
    const event = buildNostrIdentityRosterOpEvent({
      signerSecretKey: secretKey,
      profileId,
      createdAt: 10,
      clientNonce: 'nonce-1',
      encryptedDeviceLabels: 'v1.encrypted-label-payload',
      op: {
        op: 'add_facet',
        facet: {
          ...appKeyFacet(pubkey, {
            addedAt: 10,
            capabilities: APP_KEY_ADMIN_CAPABILITIES,
          }),
          label: 'Private laptop',
        },
      },
    });

    expect(event.tags).toContainEqual([
      NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT,
      'v1.encrypted-label-payload',
    ]);
    expect(event.tags).not.toContainEqual(['key_label', 'Private laptop']);
    expect(JSON.stringify(event.tags)).not.toContain('Private laptop');
    expect(encryptedDeviceLabelPayloadsFromNostrIdentityRosterOpEvent(event)).toEqual([
      'v1.encrypted-label-payload',
    ]);

    const signed = parseNostrIdentityRosterOpEvent(event);
    expect(signed.content.op).toMatchObject({
      op: 'add_facet',
      facet: { pubkey },
    });
    if (signed.content.op.op !== 'add_facet') throw new Error('expected add_facet');
    expect(signed.content.op.facet.label).toBeUndefined();

    const projection = projectNostrIdentityRoster(profileId, [signed]);
    expect(projection.active_facets[pubkey]?.label).toBeUndefined();
  });

  it('preserves non-app facet labels while dropping app-key labels', () => {
    const adminSecret = generateSecretKey();
    const adminPubkey = getPublicKey(adminSecret);
    const socialPubkey = getPublicKey(generateSecretKey());
    const bootstrap = parseNostrIdentityRosterOpEvent(buildNostrIdentityRosterOpEvent({
      signerSecretKey: adminSecret,
      profileId,
      createdAt: 20,
      clientNonce: 'nonce-2',
      op: {
        op: 'add_facet',
        facet: appKeyFacet(adminPubkey, {
          addedAt: 20,
          capabilities: APP_KEY_ADMIN_CAPABILITIES,
        }),
      },
    }));
    const socialEvent = buildNostrIdentityRosterOpEvent({
      signerSecretKey: adminSecret,
      profileId,
      parents: [bootstrap.op_id],
      createdAt: 21,
      clientNonce: 'nonce-3',
      op: {
        op: 'add_facet',
        facet: {
          pubkey: socialPubkey,
          purposes: ['social_profile'],
          capabilities: {},
          added_at: 21,
          label: 'Alice',
        },
      },
    });

    expect(socialEvent.tags).toContainEqual(['key_label', 'Alice']);

    const social = parseNostrIdentityRosterOpEvent(socialEvent);
    expect(social.content.op.op === 'add_facet' ? social.content.op.facet.label : undefined).toBe('Alice');

    const projection = projectNostrIdentityRoster(profileId, [bootstrap, social]);
    expect(projection.active_facets[socialPubkey]?.label).toBe('Alice');
  });
});
