import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';
import { finalizeEvent, nip44, type Event } from 'nostr-tools';

import {
  NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_BYTE_LENGTH,
  nostrIdentityDeviceApprovalRequestRelays,
  parseNostrIdentityDeviceApprovalReceiptEvent,
  type NostrIdentityDeviceApprovalRequest,
} from '../src';

interface InteropFixture {
  profileId: string;
  keys: {
    adminSecretKey: string;
    deviceAppKeySecretKey: string;
    deviceAppKeyPubkey: string;
    requestSecretKey: string;
    requestPubkey: string;
  };
  requestSecret: string;
  requestedAt: number;
  expiresAt: number;
  requestType: string;
  resources: Array<{ type: string; id: string; scopes: string[] }>;
  label: string;
  proofEvent: Event;
  receiptEvent: Event;
  expectedReceipt: Record<string, unknown>;
  tamperCases: {
    receipt: Array<{
      name: string;
      eventField?: string;
      field?: string;
      tag?: string[];
      value?: unknown;
    }>;
  };
}

const fixture = JSON.parse(readFileSync(
  new URL('../../testdata/nostr-identity-device-approval-v1.json', import.meta.url),
  'utf8',
)) as InteropFixture;

const secretKey = (hex: string): Uint8Array => Uint8Array.from(Buffer.from(hex, 'hex'));

function fixtureRequest(): NostrIdentityDeviceApprovalRequest {
  const adminAppKeyPubkey = fixture.proofEvent.tags.find((tag) => tag[0] === 'admin_pubkey')?.[1];
  if (!adminAppKeyPubkey) throw new Error('fixture proof is missing admin_pubkey');
  return {
    requestPubkey: fixture.keys.requestPubkey,
    deviceAppKeyPubkey: fixture.keys.deviceAppKeyPubkey,
    requestSecret: fixture.requestSecret,
    deviceAppKeyProof: JSON.stringify(fixture.proofEvent),
    requestedAt: fixture.requestedAt,
    requestType: fixture.requestType,
    resources: fixture.resources,
    expiresAt: fixture.expiresAt,
    profileId: fixture.profileId,
    adminAppKeyPubkey,
    label: fixture.label,
  };
}

function receiptEvent(receipt: Record<string, unknown>): Event {
  const adminSecretKey = secretKey(fixture.keys.adminSecretKey);
  const conversationKey = nip44.v2.utils.getConversationKey(adminSecretKey, fixture.keys.requestPubkey);
  return finalizeEvent({
    kind: fixture.receiptEvent.kind,
    created_at: fixture.receiptEvent.created_at,
    tags: fixture.receiptEvent.tags,
    content: nip44.v2.encrypt(JSON.stringify(receipt), conversationKey),
  }, adminSecretKey);
}

describe('NostrIdentity device approval interop vectors', () => {
  it('parses the shared proof-bound receipt', () => {
    const request = fixtureRequest();
    expect(request).toMatchObject({
      requestPubkey: fixture.keys.requestPubkey,
      deviceAppKeyPubkey: fixture.keys.deviceAppKeyPubkey,
      requestSecret: fixture.requestSecret,
      requestedAt: fixture.requestedAt,
      requestType: fixture.requestType,
      resources: fixture.resources,
      expiresAt: fixture.expiresAt,
      profileId: fixture.profileId,
      label: fixture.label,
    });
    expect(JSON.parse(request.deviceAppKeyProof)).toEqual(fixture.proofEvent);
    expect(nostrIdentityDeviceApprovalRequestRelays(request)).toEqual(['wss://temp.iris.to']);
    expect(Buffer.from(fixture.requestSecret, 'base64url')).toHaveLength(
      NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_BYTE_LENGTH,
    );

    const receipt = parseNostrIdentityDeviceApprovalReceiptEvent(fixture.receiptEvent, {
      requestSecretKey: secretKey(fixture.keys.requestSecretKey),
      request,
    });
    expect(receipt).toEqual(fixture.expectedReceipt);
  });

  it.each(fixture.tamperCases.receipt)('rejects receipt tamper: $name', (tamper) => {
    const request = fixtureRequest();
    const event = tamper.eventField !== undefined
      ? { ...fixture.receiptEvent, [tamper.eventField]: tamper.value } as Event
      : tamper.tag !== undefined
        ? finalizeEvent({
            kind: fixture.receiptEvent.kind,
            created_at: fixture.receiptEvent.created_at,
            tags: [...fixture.receiptEvent.tags, tamper.tag],
            content: fixture.receiptEvent.content,
          }, secretKey(fixture.keys.adminSecretKey))
        : receiptEvent({ ...fixture.expectedReceipt, [tamper.field!]: tamper.value });
    expect(() => parseNostrIdentityDeviceApprovalReceiptEvent(event, {
      requestSecretKey: secretKey(fixture.keys.requestSecretKey),
      request,
    })).toThrow();
  });
});
