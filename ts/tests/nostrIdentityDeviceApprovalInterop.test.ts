import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';
import { finalizeEvent, nip44, type Event } from 'nostr-tools';

import {
  NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_MIN_LENGTH,
  nostrIdentityDeviceApprovalRequestRelays,
  parseNostrIdentityDeviceApprovalReceiptEvent,
  parseNostrIdentityDeviceApprovalRequest,
} from '../src';

interface InteropFixture {
  profileId: string;
  prefix: string;
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
  fullRequest: string;
  proofEvent: Event;
  receiptEvent: Event;
  expectedReceipt: Record<string, unknown>;
  tamperCases: {
    request: Array<{ name: string; field?: string; resourceField?: string; value: unknown }>;
    proof: Array<{ name: string; content?: string; tag?: string[] }>;
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

function requestPayload(): Record<string, unknown> {
  return JSON.parse(Buffer.from(
    fixture.fullRequest.slice(fixture.prefix.length),
    'base64url',
  ).toString('utf8')) as Record<string, unknown>;
}

function encodeRequestPayload(payload: Record<string, unknown>): string {
  return `${fixture.prefix}${Buffer.from(JSON.stringify(payload), 'utf8').toString('base64url')}`;
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
  it('parses the shared full request, proof, and receipt', () => {
    const request = parseNostrIdentityDeviceApprovalRequest(fixture.fullRequest);
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
    expect(JSON.parse(request?.deviceAppKeyProof ?? '')).toEqual(fixture.proofEvent);
    expect(nostrIdentityDeviceApprovalRequestRelays(request!)).toEqual(['wss://temp.iris.to']);
    expect(fixture.requestSecret.length).toBeGreaterThanOrEqual(
      NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_SECRET_MIN_LENGTH,
    );

    const receipt = parseNostrIdentityDeviceApprovalReceiptEvent(fixture.receiptEvent, {
      requestSecretKey: secretKey(fixture.keys.requestSecretKey),
      request: request!,
    });
    expect(receipt).toEqual(fixture.expectedReceipt);
  });

  it.each(fixture.tamperCases.request)('rejects request tamper: $name', (tamper) => {
    const payload = requestPayload();
    if (tamper.resourceField !== undefined) {
      const resources = payload.resources as Array<Record<string, unknown>>;
      resources[0] = { ...resources[0], [tamper.resourceField]: tamper.value };
    } else if (tamper.field !== undefined) {
      payload[tamper.field] = tamper.value;
    }
    expect(parseNostrIdentityDeviceApprovalRequest(encodeRequestPayload(payload))).toBeNull();
  });

  it.each(fixture.tamperCases.proof)('rejects proof tamper: $name', (tamper) => {
    const payload = requestPayload();
    const proof = finalizeEvent({
      kind: fixture.proofEvent.kind,
      created_at: fixture.proofEvent.created_at,
      tags: tamper.tag ? [...fixture.proofEvent.tags, tamper.tag] : fixture.proofEvent.tags,
      content: tamper.content ?? '',
    }, secretKey(fixture.keys.deviceAppKeySecretKey));
    payload.deviceAppKeyProof = JSON.stringify(proof);
    expect(parseNostrIdentityDeviceApprovalRequest(encodeRequestPayload(payload))).toBeNull();
  });

  it.each(fixture.tamperCases.receipt)('rejects receipt tamper: $name', (tamper) => {
    const request = parseNostrIdentityDeviceApprovalRequest(fixture.fullRequest)!;
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
