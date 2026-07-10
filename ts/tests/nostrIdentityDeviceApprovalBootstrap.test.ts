import { describe, expect, it } from 'vitest';
import { finalizeEvent, getPublicKey, nip19, type Event } from 'nostr-tools';

import {
  FACT_OP_KIND,
  NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_PREFIX,
  NOSTR_IDENTITY_DEVICE_APPROVAL_LABEL_MAX_BYTES,
  NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_EVENT_TYPE,
  buildNostrIdentityDeviceApprovalRequestEvent,
  createNostrIdentityDeviceApprovalBootstrap,
  createNostrIdentityDeviceApprovalRequest,
  encodeNostrIdentityDeviceApprovalBootstrap,
  nostrIdentityDeviceApprovalBootstrapHasPrefix,
  parseNostrIdentityDeviceApprovalBootstrap,
  parseNostrIdentityDeviceApprovalRequestEvent,
  randomNostrIdentityDeviceApprovalSecret,
} from '../src';

const secretKey = (byte: number): Uint8Array => new Uint8Array(32).fill(byte);
const requestSecret = Buffer.from(Uint8Array.from({ length: 32 }, (_, index) => index)).toString('base64url');

function setup() {
  const deviceAppKeySecretKey = secretKey(1);
  const requestSecretKey = secretKey(2);
  const request = createNostrIdentityDeviceApprovalRequest({
    deviceAppKeySecretKey,
    requestSecretKey,
    requestSecret,
    requestedAt: 1_720_000_000,
    requestType: 'nostr_vpn_join',
    resources: [{ type: 'nostr_vpn', id: 'network', scopes: ['join'] }],
    expiresAt: 1_720_000_300,
    profileId: '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a',
    adminAppKeyPubkey: getPublicKey(secretKey(3)),
    label: 'WebVM',
  });
  const bootstrap = createNostrIdentityDeviceApprovalBootstrap(request);
  return { bootstrap, deviceAppKeySecretKey, request, requestSecretKey };
}

function decodeUriPayload(uri: string, prefix: string): Record<string, unknown> {
  return JSON.parse(Buffer.from(uri.slice(prefix.length), 'base64url').toString('utf8')) as Record<string, unknown>;
}

function encodeUriPayload(prefix: string, payload: Record<string, unknown>): string {
  return `${prefix}${Buffer.from(JSON.stringify(payload), 'utf8').toString('base64url')}`;
}

describe('NostrIdentity compact device approval bootstrap', () => {
  it('encodes exactly the stable npub, ephemeral npub, canonical 32-byte secret, and bounded label', () => {
    const { bootstrap, request } = setup();
    const prefix = NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_PREFIX;
    const uri = encodeNostrIdentityDeviceApprovalBootstrap(bootstrap);

    expect(uri.startsWith(prefix)).toBe(true);
    expect(uri.length).toBeLessThanOrEqual(384);
    expect(decodeUriPayload(uri, prefix)).toEqual({
      deviceAppKeyNpub: nip19.npubEncode(request.deviceAppKeyPubkey),
      requestNpub: nip19.npubEncode(request.requestPubkey),
      requestSecret,
      label: 'WebVM',
    });
    expect(parseNostrIdentityDeviceApprovalBootstrap(uri)).toEqual(bootstrap);
    expect(nostrIdentityDeviceApprovalBootstrapHasPrefix(uri)).toBe(true);
    expect(nostrIdentityDeviceApprovalBootstrapHasPrefix(`nostr:${uri}`)).toBe(false);
  });

  it('generates a distinct ephemeral key and a canonical 32-byte secret by default', () => {
    const request = createNostrIdentityDeviceApprovalRequest({
      deviceAppKeySecretKey: secretKey(4),
      requestedAt: 1_720_000_000,
    });
    expect(request.requestPubkey).not.toBe(request.deviceAppKeyPubkey);
    expect(Buffer.from(request.requestSecret, 'base64url')).toHaveLength(32);
    expect(Buffer.from(request.requestSecret, 'base64url').toString('base64url')).toBe(request.requestSecret);
    const generatedSecret = randomNostrIdentityDeviceApprovalSecret();
    expect(Buffer.from(generatedSecret, 'base64url')).toHaveLength(32);
    expect(Buffer.from(generatedSecret, 'base64url').toString('base64url')).toBe(generatedSecret);
  });

  it('trims labels and bounds them by UTF-8 bytes', () => {
    const { request } = setup();
    const exactAscii = createNostrIdentityDeviceApprovalBootstrap({
      ...request,
      label: ` ${'a'.repeat(NOSTR_IDENTITY_DEVICE_APPROVAL_LABEL_MAX_BYTES)} `,
    });
    const exactUtf8 = createNostrIdentityDeviceApprovalBootstrap({
      ...request,
      label: 'é'.repeat(NOSTR_IDENTITY_DEVICE_APPROVAL_LABEL_MAX_BYTES / 2),
    });

    expect(exactAscii.label).toBe('a'.repeat(16));
    expect(parseNostrIdentityDeviceApprovalBootstrap(
      encodeNostrIdentityDeviceApprovalBootstrap(exactUtf8),
    )?.label).toBe('é'.repeat(8));
    expect(() => createNostrIdentityDeviceApprovalBootstrap({
      ...request,
      label: 'a'.repeat(17),
    })).toThrow('16 UTF-8 bytes');
    expect(() => createNostrIdentityDeviceApprovalBootstrap({
      ...request,
      label: 'é'.repeat(9),
    })).toThrow('16 UTF-8 bytes');
  });

  it('strictly rejects legacy queries, unknown metadata, same keys, and malformed secrets', () => {
    const { bootstrap } = setup();
    const prefix = 'nvpn:';
    const uri = encodeNostrIdentityDeviceApprovalBootstrap(bootstrap, { prefix });
    const payload = decodeUriPayload(uri, prefix);

    expect(parseNostrIdentityDeviceApprovalBootstrap(
      `${prefix}?app_key=${bootstrap.deviceAppKeyNpub}`,
      { prefixes: [prefix] },
    )).toBeNull();
    expect(parseNostrIdentityDeviceApprovalBootstrap(`nostr:${uri}`)).toBeNull();
    expect(parseNostrIdentityDeviceApprovalBootstrap(`${uri}?relay=wss://example.test`)).toBeNull();
    expect(parseNostrIdentityDeviceApprovalBootstrap(`${uri}#scan`)).toBeNull();
    for (const extra of ['v', 'deviceAppKeyProof', 'resources', 'relay', 'requestedAt']) {
      expect(parseNostrIdentityDeviceApprovalBootstrap(
        encodeUriPayload(prefix, { ...payload, [extra]: true }),
        { prefixes: [prefix] },
      )).toBeNull();
    }
    expect(parseNostrIdentityDeviceApprovalBootstrap(encodeUriPayload(prefix, {
      ...payload,
      requestNpub: bootstrap.deviceAppKeyNpub,
    }), { prefixes: [prefix] })).toBeNull();
    expect(parseNostrIdentityDeviceApprovalBootstrap(encodeUriPayload(prefix, {
      ...payload,
      requestNpub: getPublicKey(secretKey(2)),
    }), { prefixes: [prefix] })).toBeNull();
    expect(parseNostrIdentityDeviceApprovalBootstrap(encodeUriPayload(prefix, {
      ...payload,
      requestNpub: bootstrap.requestNpub.toUpperCase(),
    }), { prefixes: [prefix] })).toBeNull();
    const { requestNpub: _requestNpub, ...missingRequestNpub } = payload;
    expect(parseNostrIdentityDeviceApprovalBootstrap(
      encodeUriPayload(prefix, missingRequestNpub),
      { prefixes: [prefix] },
    )).toBeNull();
    expect(parseNostrIdentityDeviceApprovalBootstrap(encodeUriPayload(prefix, {
      ...payload,
      requestSecret: Buffer.alloc(31, 1).toString('base64url'),
    }), { prefixes: [prefix] })).toBeNull();
    expect(parseNostrIdentityDeviceApprovalBootstrap(encodeUriPayload(prefix, {
      ...payload,
      requestSecret: `${requestSecret}=`,
    }), { prefixes: [prefix] })).toBeNull();
    expect(() => encodeNostrIdentityDeviceApprovalBootstrap(bootstrap, {
      prefix: `nvpn:${'x'.repeat(60)}`,
    })).toThrow('384');
  });
});

describe('NostrIdentity device approval request event', () => {
  it('is signed by the ephemeral key and reconstructs the full secret-bound request', () => {
    const { bootstrap, request, requestSecretKey } = setup();
    const event = buildNostrIdentityDeviceApprovalRequestEvent({ request, requestSecretKey });
    const content = JSON.parse(event.content) as Record<string, unknown>;

    expect(event.kind).toBe(FACT_OP_KIND);
    expect(event.pubkey).toBe(request.requestPubkey);
    expect(event.created_at).toBe(request.requestedAt);
    expect(event.tags).toEqual([
      ['type', NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_EVENT_TYPE],
      ['p', request.deviceAppKeyPubkey],
    ]);
    expect(Object.keys(content).sort()).toEqual([
      'adminAppKeyNpub',
      'deviceAppKeyProof',
      'expiresAt',
      'label',
      'profileId',
      'requestSecretCommitment',
      'requestType',
      'requestedAt',
      'resources',
    ].sort());
    expect(content.requestSecretCommitment).toBe(
      '55aeef52b9f3641a8546bd60f0861a5b52f0952df10fecf000d6651b68c3a3ba',
    );
    expect(JSON.stringify(event)).not.toContain(requestSecret);
    const { requestSecretKey: _requestSecretKey, ...publicRequest } = request;
    expect(parseNostrIdentityDeviceApprovalRequestEvent(event, bootstrap)).toEqual(publicRequest);
  });

  it('rejects independently signed outer, commitment, content, tag, and proof tampering', () => {
    const { bootstrap, request, requestSecretKey } = setup();
    const event = buildNostrIdentityDeviceApprovalRequestEvent({ request, requestSecretKey });
    const content = JSON.parse(event.content) as Record<string, unknown>;
    const resign = (changes: Partial<Pick<Event, 'kind' | 'created_at' | 'tags' | 'content'>>): Event => finalizeEvent({
      kind: changes.kind ?? event.kind,
      created_at: changes.created_at ?? event.created_at,
      tags: changes.tags ?? event.tags,
      content: changes.content ?? event.content,
    }, requestSecretKey);

    expect(() => parseNostrIdentityDeviceApprovalRequestEvent(
      finalizeEvent({
        kind: event.kind,
        created_at: event.created_at,
        tags: event.tags,
        content: event.content,
      }, secretKey(9)),
      bootstrap,
    )).toThrow('signer');
    expect(() => parseNostrIdentityDeviceApprovalRequestEvent(resign({
      content: JSON.stringify({ ...content, requestSecretCommitment: '00'.repeat(32) }),
    }), bootstrap)).toThrow('commitment');
    expect(() => parseNostrIdentityDeviceApprovalRequestEvent(resign({
      content: JSON.stringify({ ...content, relay: 'wss://example.test' }),
    }), bootstrap)).toThrow('unknown field');
    expect(() => parseNostrIdentityDeviceApprovalRequestEvent(resign({
      tags: [...event.tags, ['relay', 'wss://example.test']],
    }), bootstrap)).toThrow('tags');

    const otherDeviceRequest = createNostrIdentityDeviceApprovalRequest({
      deviceAppKeySecretKey: secretKey(8),
      requestSecretKey,
      requestSecret,
      requestedAt: request.requestedAt,
      requestType: request.requestType,
      resources: request.resources,
      expiresAt: request.expiresAt,
      profileId: request.profileId,
      adminAppKeyPubkey: request.adminAppKeyPubkey,
      label: request.label,
    });
    expect(() => parseNostrIdentityDeviceApprovalRequestEvent(resign({
      content: JSON.stringify({ ...content, deviceAppKeyProof: otherDeviceRequest.deviceAppKeyProof }),
    }), bootstrap)).toThrow('signer');

    const otherEphemeralRequest = createNostrIdentityDeviceApprovalRequest({
      deviceAppKeySecretKey: secretKey(1),
      requestSecretKey: secretKey(7),
      requestSecret,
      requestedAt: request.requestedAt,
      requestType: request.requestType,
      resources: request.resources,
      expiresAt: request.expiresAt,
      profileId: request.profileId,
      adminAppKeyPubkey: request.adminAppKeyPubkey,
      label: request.label,
    });
    expect(() => parseNostrIdentityDeviceApprovalRequestEvent(resign({
      content: JSON.stringify({ ...content, deviceAppKeyProof: otherEphemeralRequest.deviceAppKeyProof }),
    }), bootstrap)).toThrow('request_pubkey');
  });
});
