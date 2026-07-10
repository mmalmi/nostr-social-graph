import { describe, expect, it } from 'vitest';
import { finalizeEvent, generateSecretKey, getPublicKey, type Event } from 'nostr-tools';
import {
  APP_KEY_ADMIN_CAPABILITIES,
  KIND_NOSTR_IDENTITY_ROSTER_OP,
  KIND_NOSTR_IDENTITY_DEVICE_LINK_REQUEST,
  NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT,
  appKeyFacet,
  buildNostrIdentityDeviceApprovalReceiptEvent,
  buildNostrIdentityRosterOpEvent,
  approveNostrIdentityDeviceApprovalRequest,
  createNostrIdentityDeviceApprovalRequest,
  createNostrIdentityDeviceLinkInvite,
  createNostrIdentityDeviceLinkRequest,
  createNostrIdentityManualDeviceAddRosterOp,
  encryptedDeviceLabelPayloadsFromNostrIdentityRosterOpEvent,
  encodeNostrIdentityDeviceLinkInvite,
  isCompleteNostrIdentityDeviceLinkInviteInput,
  nostrIdentityAppKeyApprovalCandidateFilters,
  nostrIdentityAppKeyApprovalCandidatesFromEvents,
  nostrIdentityDeviceApprovalClientNonce,
  nostrIdentityDeviceApprovalRelayResource,
  nostrIdentityDeviceApprovalRequestRelays,
  nostrIdentityRosterOpMatchesDeviceApprovalReceipt,
  parseNostrIdentityDeviceApprovalReceiptEvent,
  parseNostrIdentityDeviceApprovalReceiptRosterOp,
  parseNostrIdentityDeviceLinkInvite,
  parseNostrIdentityDeviceLinkRequestEvent,
  parseNostrIdentityRosterOpEvent,
  projectNostrIdentityRoster,
  signNostrIdentityDeviceLinkRequestEvent,
} from '../src';

const profileId = '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a';
const requestSecret = Buffer.from(Uint8Array.from({ length: 32 }, (_, index) => index)).toString('base64url');

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

  it('builds admin-invite device link requests as encrypted NostrIdentity fact events', () => {
    const adminSecret = generateSecretKey();
    const adminPubkey = getPublicKey(adminSecret);
    const inviteSecret = generateSecretKey();
    const invite = createNostrIdentityDeviceLinkInvite({
      profileId,
      adminAppKeyPubkey: adminPubkey,
      inviteSecretKey: inviteSecret,
    });
    const encoded = encodeNostrIdentityDeviceLinkInvite(invite, {
      prefix: 'https://chat.iris.to/link-device/',
    });
    const parsedInvite = parseNostrIdentityDeviceLinkInvite(encoded, {
      prefixes: ['https://chat.iris.to/link-device/'],
    });
    expect(parsedInvite).toEqual({
      profileId,
      adminAppKeyPubkey: adminPubkey,
      invitePubkey: invite.invitePubkey,
    });
    expect(isCompleteNostrIdentityDeviceLinkInviteInput(encoded, {
      prefixes: ['https://chat.iris.to/link-device/'],
    })).toBe(true);

    const deviceSecret = generateSecretKey();
    const devicePubkey = getPublicKey(deviceSecret);
    const request = createNostrIdentityDeviceLinkRequest({
      invite,
      deviceAppKeyPubkey: devicePubkey,
      requestedAt: 30,
      label: 'This device',
    });
    const event = signNostrIdentityDeviceLinkRequestEvent({
      signerSecretKey: deviceSecret,
      request,
      clientNonce: 'device-link-request',
    });

    expect(event.kind).toBe(KIND_NOSTR_IDENTITY_DEVICE_LINK_REQUEST);
    expect(event.tags).toContainEqual(['type', 'nostr_identity_link_request']);
    expect(event.content).not.toContain('This device');

    const signedRequest = parseNostrIdentityDeviceLinkRequestEvent(event, {
      profileId,
      adminAppKeyPubkey: adminPubkey,
      inviteSecretKey: inviteSecret,
      invitePubkey: invite.invitePubkey,
    });
    expect(signedRequest.request).toEqual(request);
  });

  it('approves scan-to-approve requests with secret-bound NostrIdentity roster ops', () => {
    const adminSecret = generateSecretKey();
    const adminPubkey = getPublicKey(adminSecret);
    const deviceSecret = generateSecretKey();
    const devicePubkey = getPublicKey(deviceSecret);
    const requestSecretKey = generateSecretKey();
    const requestPubkey = getPublicKey(requestSecretKey);
    const bootstrap = parseNostrIdentityRosterOpEvent(buildNostrIdentityRosterOpEvent({
      signerSecretKey: adminSecret,
      profileId,
      createdAt: 40,
      clientNonce: 'bootstrap-scan-admin',
      op: {
        op: 'add_facet',
        facet: appKeyFacet(adminPubkey, {
          addedAt: 40,
          capabilities: APP_KEY_ADMIN_CAPABILITIES,
        }),
      },
    }));
    const request = createNostrIdentityDeviceApprovalRequest({
      deviceAppKeySecretKey: deviceSecret,
      requestSecretKey,
      requestSecret,
      requestedAt: 41,
      requestType: 'device_link',
      resources: [{ type: 'chat_group', id: profileId, scopes: ['admin'] }],
      expiresAt: 101,
      label: 'This device',
    });
    expect(request.requestPubkey).toBe(requestPubkey);
    expect(request.deviceAppKeyPubkey).toBe(devicePubkey);
    expect(request.deviceAppKeyProof).not.toContain(request.requestSecret);

    const approvalContent = approveNostrIdentityDeviceApprovalRequest({
      request,
      profileId,
      rosterOps: [bootstrap],
      approvedByPubkey: adminPubkey,
      approvedAt: 42,
      clientNonce: nostrIdentityDeviceApprovalClientNonce(Buffer.alloc(32, 9).toString('base64url')),
    });
    const approval = buildNostrIdentityRosterOpEvent({
      signerSecretKey: adminSecret,
      profileId,
      parents: [bootstrap.op_id],
      createdAt: 42,
      clientNonce: approvalContent.client_nonce,
      op: approvalContent.op,
    });
    expect(approval.tags).toContainEqual(['type', 'nostr_identity_roster_op']);
    expect(approval.tags).not.toContainEqual(['type', 'double-ratchet/app-keys']);
    expect(JSON.stringify(approval.tags)).not.toContain(request.requestSecret);
    expect(JSON.stringify(approval.tags)).not.toContain(request.requestPubkey);

    const signedApproval = parseNostrIdentityRosterOpEvent(approval);
    const receiptEvent = buildNostrIdentityDeviceApprovalReceiptEvent({
      signerSecretKey: adminSecret,
      request,
      profileId,
      approvedAt: 42,
      subjectPubkey: adminPubkey,
      rosterOpEvent: approval,
    });
    expect(JSON.stringify(receiptEvent.tags)).not.toContain(request.requestSecret);
    expect(receiptEvent.content).not.toContain(request.requestSecret);
    const receipt = parseNostrIdentityDeviceApprovalReceiptEvent(receiptEvent, {
      requestSecretKey,
      request,
      profileId,
      approvedByPubkey: adminPubkey,
    });
    expect(receipt.requestSecret).toBe(request.requestSecret);
    expect(receipt.subjectPubkey).toBe(adminPubkey);
    expect(receipt.rosterOpId).toBe(signedApproval.op_id);
    const receiptRosterOp = parseNostrIdentityDeviceApprovalReceiptRosterOp(receipt);
    expect(receiptRosterOp.op_id).toBe(signedApproval.op_id);
    expect(nostrIdentityRosterOpMatchesDeviceApprovalReceipt(receiptRosterOp, receipt)).toBe(true);
    expect(() => parseNostrIdentityDeviceApprovalReceiptEvent(receiptEvent, {
      requestSecretKey,
      request: { ...request, requestPubkey: getPublicKey(generateSecretKey()) },
    })).toThrow('request');

    const manualAdd = createNostrIdentityManualDeviceAddRosterOp({
      profileId,
      rosterOps: [bootstrap],
      approvedByPubkey: adminPubkey,
      devicePubkey,
      addedAt: 43,
      clientNonce: 'manual-add',
    });
    expect(manualAdd.client_nonce).toBe('manual-add');
  });

  it('embeds exactly one normalized approval rendezvous relay in request resources', () => {
    const relayResource = nostrIdentityDeviceApprovalRelayResource(
      ' WSS://TEMP.IRIS.TO:443/ ',
    );
    expect(relayResource).toEqual({
      type: 'nostr_relay',
      id: 'wss://temp.iris.to',
      scopes: ['device_approval'],
    });
    expect(nostrIdentityDeviceApprovalRelayResource(
      'ws://EXAMPLE.COM:80/approval//?z=2&a=1#ignored',
    ).id).toBe('ws://example.com/approval?a=1&z=2');

    expect(nostrIdentityDeviceApprovalRequestRelays({
      resources: [
        { type: 'collection', id: 'legacy-resource', scopes: ['read'] },
        relayResource,
        { type: 'nostr_relay', id: 'wss://temp.iris.to/', scopes: ['device_approval'] },
        { type: 'nostr_relay', id: 'wss://ignored.example', scopes: ['read'] },
      ],
    })).toEqual(['wss://temp.iris.to']);
    expect(nostrIdentityDeviceApprovalRequestRelays({})).toEqual([]);

    expect(() => nostrIdentityDeviceApprovalRelayResource('https://temp.iris.to')).toThrow('ws');
    expect(() => nostrIdentityDeviceApprovalRelayResource('wss://user@temp.iris.to')).toThrow('credentials');
    expect(() => nostrIdentityDeviceApprovalRelayResource('wss://@temp.iris.to')).toThrow('credentials');
    expect(() => nostrIdentityDeviceApprovalRelayResource('wss:///approval')).toThrow('URL');
    expect(() => nostrIdentityDeviceApprovalRequestRelays({
      resources: [{
        type: 'nostr_relay',
        id: 'wss://user@temp.iris.to',
        scopes: ['device_approval'],
      }],
    })).toThrow('credentials');
    expect(() => nostrIdentityDeviceApprovalRequestRelays({
      resources: [
        relayResource,
        nostrIdentityDeviceApprovalRelayResource('wss://other.example'),
      ],
    })).toThrow('one relay');
  });

  it('projects app-key approval candidates from real roster events that tag the joining key', () => {
    const adminSecret = generateSecretKey();
    const adminPubkey = getPublicKey(adminSecret);
    const devicePubkey = getPublicKey(generateSecretKey());
    const bootstrap = parseNostrIdentityRosterOpEvent(buildNostrIdentityRosterOpEvent({
      signerSecretKey: adminSecret,
      profileId,
      createdAt: 60,
      clientNonce: 'bootstrap-approval-candidate',
      op: {
        op: 'add_facet',
        facet: appKeyFacet(adminPubkey, {
          addedAt: 60,
          capabilities: APP_KEY_ADMIN_CAPABILITIES,
        }),
      },
    }));
    const approvalContent = createNostrIdentityManualDeviceAddRosterOp({
      profileId,
      rosterOps: [bootstrap],
      approvedByPubkey: adminPubkey,
      devicePubkey,
      addedAt: 61,
      clientNonce: 'manual-candidate',
    });
    const approvalEvent = buildNostrIdentityRosterOpEvent({
      signerSecretKey: adminSecret,
      profileId,
      parents: [bootstrap.op_id],
      createdAt: 61,
      clientNonce: approvalContent.client_nonce,
      op: approvalContent.op,
    });

    expect(nostrIdentityAppKeyApprovalCandidateFilters(devicePubkey)).toEqual([{
      kinds: [KIND_NOSTR_IDENTITY_ROSTER_OP],
      '#p': [devicePubkey],
    }]);

    const candidates = nostrIdentityAppKeyApprovalCandidatesFromEvents(
      devicePubkey,
      [JSON.parse(bootstrap.event_json), approvalEvent],
    );

    expect(candidates).toHaveLength(1);
    expect(candidates[0]).toMatchObject({
      profileId,
      appKeyPubkey: devicePubkey,
      adminAppKeyPubkey: adminPubkey,
      acceptedRosterOpCount: 2,
      activeAppKeyCount: 2,
      latestRosterOpCreatedAt: 61,
    });
    expect(candidates[0]?.profileRosterOps).toHaveLength(2);
  });
});
