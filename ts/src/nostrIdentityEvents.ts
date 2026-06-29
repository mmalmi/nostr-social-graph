import { finalizeEvent, getPublicKey, type Event } from 'nostr-tools';
import {
  IDENTITY_CAPABILITY_ADMIN,
  IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
  IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS,
  IDENTITY_CAPABILITY_RECOVER,
  IDENTITY_CAPABILITY_WRITE,
  IDENTITY_PURPOSE_APP,
  IDENTITY_PURPOSE_PROFILE,
  IDENTITY_PURPOSE_RECOVERY,
  IDENTITY_PURPOSE_REMOTE_SIGNER,
  buildIdentityKeyAcceptanceDraft,
  buildIdentityRosterOpDraft,
  parseIdentityKeyAcceptanceEvent,
  parseIdentityRosterOpEvent,
  normalizeHexPubkey,
  type IdentityKeyAcceptanceContent,
  type IdentityKeyCapability,
  type IdentityKeyPurpose,
  type IdentityRosterProjection,
  type IdentityRosterOp,
  type IdentityRosterOpContent,
  type SignedIdentityRosterOp,
} from './identityGraph';
import { fact } from './factEvents';
import {
  NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT,
  NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA,
  NOSTR_IDENTITY_ROSTER_SCHEMA,
  type BuildNostrIdentityFacetAcceptanceEventDraftOptions,
  type BuildNostrIdentityFacetAcceptanceEventOptions,
  type BuildNostrIdentityRosterOpEventDraftOptions,
  type BuildNostrIdentityRosterOpEventOptions,
  type NostrIdentityEventDraft,
  type NostrIdentityCapabilities,
  type NostrIdentityFacetAcceptanceContent,
  type NostrIdentityKeyPurpose,
  type NostrIdentityRosterProjection,
  type NostrIdentityRosterOp,
  type NostrIdentityRosterOpContent,
  type SignedNostrIdentityFacetAcceptance,
  type SignedNostrIdentityRosterOp,
  normalizeCapabilities,
} from './nostrIdentity';
import {
  currentUnixSeconds,
  randomClientNonce,
  requireValidSignature,
} from './nostrIdentityJson';
import { normalizeRosterOp, sortPurposes } from './nostrIdentityNormalize';

export function buildNostrIdentityRosterOpEventDraft(
  options: BuildNostrIdentityRosterOpEventDraftOptions,
): NostrIdentityEventDraft {
  const signerPubkey = normalizeHexPubkey(options.signerPubkey);
  if (!signerPubkey) throw new Error('roster signer pubkey must be 64-char hex');
  const createdAt = options.createdAt ?? currentUnixSeconds();
  const clientNonce = options.clientNonce ?? randomClientNonce();
  const parents = options.parents?.slice() ?? [];
  const draft = buildIdentityRosterOpDraft({
    signerPubkey,
    identity: options.profileId,
    op: nostrIdentityRosterOpToIdentity(normalizeRosterOp(options.op)),
    parents,
    ...(options.actorSeq !== undefined ? { actorSeq: options.actorSeq } : {}),
    createdAt,
    clientNonce,
    extensionFacts: encryptedDeviceLabelsExtensionFacts(options.encryptedDeviceLabels),
  });

  return {
    kind: draft.kind,
    content: draft.content,
    created_at: createdAt,
    tags: draft.tags,
  };
}

export function buildNostrIdentityRosterOpEvent(options: BuildNostrIdentityRosterOpEventOptions): Event {
  return finalizeEvent(buildNostrIdentityRosterOpEventDraft({
    ...options,
    signerPubkey: getPublicKey(options.signerSecretKey),
  }), options.signerSecretKey);
}

export function signNostrIdentityRosterOp(
  options: BuildNostrIdentityRosterOpEventOptions,
): SignedNostrIdentityRosterOp {
  return parseNostrIdentityRosterOpEvent(buildNostrIdentityRosterOpEvent(options));
}

export function buildNostrIdentityFacetAcceptanceEventDraft(
  options: BuildNostrIdentityFacetAcceptanceEventDraftOptions,
): NostrIdentityEventDraft {
  const facetPubkey = normalizeHexPubkey(options.signerPubkey);
  if (!facetPubkey) throw new Error('facet acceptance signer pubkey must be 64-char hex');
  const acceptedAt = options.acceptedAt ?? currentUnixSeconds();
  const clientNonce = options.clientNonce ?? randomClientNonce();
  const purposes = sortPurposes(options.purposes);
  const draft = buildIdentityKeyAcceptanceDraft({
    signerPubkey: facetPubkey,
    identity: options.profileId,
    purposes: purposes.map(nostrIdentityPurposeToIdentity),
    ...(options.rosterOpId !== undefined ? { rosterOpId: options.rosterOpId } : {}),
    acceptedAt,
    clientNonce,
  });

  return {
    kind: draft.kind,
    content: draft.content,
    created_at: acceptedAt,
    tags: draft.tags,
  };
}

export function buildNostrIdentityFacetAcceptanceEvent(
  options: BuildNostrIdentityFacetAcceptanceEventOptions,
): Event {
  return finalizeEvent(buildNostrIdentityFacetAcceptanceEventDraft({
    ...options,
    signerPubkey: getPublicKey(options.signerSecretKey),
  }), options.signerSecretKey);
}

export function signNostrIdentityFacetAcceptance(
  options: BuildNostrIdentityFacetAcceptanceEventOptions,
): SignedNostrIdentityFacetAcceptance {
  return parseNostrIdentityFacetAcceptanceEvent(buildNostrIdentityFacetAcceptanceEvent(options));
}

export function parseNostrIdentityRosterOpEvent(event: Event): SignedNostrIdentityRosterOp {
  requireValidSignature(event);
  const signed = parseIdentityRosterOpEvent(event);
  const content = identityRosterContentToNostrIdentity(signed.content);
  if (content.actor_pubkey !== event.pubkey) {
    throw new Error('roster actor signer mismatch');
  }
  if (content.created_at !== event.created_at) {
    throw new Error('roster created_at mismatch');
  }
  return {
    op_id: signed.opId,
    signer_pubkey: signed.signerPubkey,
    content,
    event_json: JSON.stringify(event),
  };
}

export function parseNostrIdentityFacetAcceptanceEvent(event: Event): SignedNostrIdentityFacetAcceptance {
  requireValidSignature(event);
  const signed = parseIdentityKeyAcceptanceEvent(event);
  const content = identityKeyAcceptanceContentToNostrIdentity(signed.content);
  if (content.facet_pubkey !== event.pubkey) {
    throw new Error('facet acceptance signer mismatch');
  }
  if (content.accepted_at !== event.created_at) {
    throw new Error('facet acceptance accepted_at mismatch');
  }
  return {
    acceptance_id: signed.acceptanceId,
    signer_pubkey: signed.signerPubkey,
    content,
    event_json: JSON.stringify(event),
  };
}

export function encryptedDeviceLabelPayloadsFromNostrIdentityRosterOpEvent(
  event: Pick<Event, 'tags'>,
): string[] {
  return event.tags
    .filter((tag) => tag[0] === NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT)
    .map((tag) => tag[1]?.trim() ?? '')
    .filter(Boolean);
}

export function nostrIdentityRosterOpToIdentity(op: NostrIdentityRosterOp): IdentityRosterOp {
  if (op.op === 'add_facet') {
    return {
      op: 'add_key',
      key: {
        pubkey: requireHexPubkey(op.facet.pubkey, 'facet'),
        ...(op.facet.profile_id !== undefined ? { subject: op.facet.profile_id } : {}),
        purposes: (op.facet.purposes ?? []).map(nostrIdentityPurposeToIdentity),
        capabilities: nostrIdentityCapabilitiesToIdentity(op.facet.capabilities ?? {}),
        addedAt: op.facet.added_at,
        ...(op.facet.label !== undefined && !op.facet.purposes?.includes('app_key') ? { label: op.facet.label } : {}),
      },
    };
  }
  if (op.op === 'tombstone_facet') {
    return {
      op: 'tombstone_key',
      pubkey: requireHexPubkey(op.pubkey, 'target'),
      ...(op.reason !== undefined ? { reason: op.reason } : {}),
    };
  }
  if (op.op === 'set_capabilities') {
    return {
      op: 'set_key_capabilities',
      pubkey: requireHexPubkey(op.pubkey, 'target'),
      capabilities: nostrIdentityCapabilitiesToIdentity(op.capabilities),
    };
  }
  if (op.op === 'rotate_secret_epoch') {
    return {
      op: 'rotate_secret_epoch',
      epoch: op.epoch,
      wrappedSecrets: normalizeWrappedSecrets(op.wrapped_secrets ?? {}),
    };
  }
  return {
    op: 'repair_secret_wraps',
    epoch: op.epoch,
    wrappedSecrets: normalizeWrappedSecrets(op.wrapped_secrets ?? {}),
  };
}

function identityRosterContentToNostrIdentity(content: IdentityRosterOpContent): NostrIdentityRosterOpContent {
  if (content.schema !== NOSTR_IDENTITY_ROSTER_SCHEMA) {
    throw new Error(`unsupported NostrIdentity roster schema ${content.schema}`);
  }
  return {
    schema: NOSTR_IDENTITY_ROSTER_SCHEMA,
    profile_id: content.identity,
    actor_pubkey: content.actorPubkey,
    ...(content.actorSeq !== undefined ? { actor_seq: content.actorSeq } : {}),
    ...(content.parents?.length ? { parents: content.parents.slice() } : {}),
    client_nonce: content.clientNonce,
    created_at: content.createdAt,
    op: identityRosterOpToNostrIdentity(content.op),
  };
}

export function signedNostrIdentityRosterOpToIdentity(
  signed: SignedNostrIdentityRosterOp,
): SignedIdentityRosterOp {
  return {
    opId: signed.op_id,
    signerPubkey: signed.signer_pubkey,
    content: {
      schema: signed.content.schema,
      identity: signed.content.profile_id,
      actorPubkey: signed.content.actor_pubkey,
      ...(signed.content.actor_seq !== undefined ? { actorSeq: signed.content.actor_seq } : {}),
      parents: signed.content.parents?.slice() ?? [],
      clientNonce: signed.content.client_nonce,
      createdAt: signed.content.created_at,
      op: nostrIdentityRosterOpToIdentity(signed.content.op),
    },
  };
}

export function identityRosterProjectionToNostrIdentity(
  projection: IdentityRosterProjection,
): NostrIdentityRosterProjection {
  return {
    profile_id: projection.identity,
    active_facets: Object.fromEntries(
      Object.entries(projection.activeKeys).map(([pubkey, key]) => {
        const purposes = (key.purposes ?? []).map(identityPurposeToNostrIdentity);
        return [
          pubkey,
          {
            pubkey: key.pubkey,
            ...(key.subject !== undefined ? { profile_id: key.subject } : {}),
            ...(purposes.length ? { purposes } : {}),
            capabilities: identityCapabilitiesToNostrIdentity(key.capabilities),
            added_at: key.addedAt,
            ...(key.label !== undefined && !purposes.includes('app_key') ? { label: key.label } : {}),
          },
        ];
      }),
    ),
    tombstones: Object.fromEntries(
      Object.entries(projection.tombstones).map(([pubkey, tombstone]) => [
        pubkey,
        {
          pubkey: tombstone.pubkey,
          ...(tombstone.subject !== undefined ? { profile_id: tombstone.subject } : {}),
          removed_by_pubkey: tombstone.removedByPubkey,
          removed_at: tombstone.removedAt,
          ...(tombstone.reason !== undefined ? { reason: tombstone.reason } : {}),
        },
      ]),
    ),
    secret_epochs: Object.fromEntries(
      Object.entries(projection.secretEpochs).map(([epoch, secretEpoch]) => [
        epoch,
        {
          epoch: secretEpoch.epoch,
          created_at: secretEpoch.createdAt,
          signed_by_pubkey: secretEpoch.signedByPubkey,
          wrapped_secrets: secretEpoch.wrappedSecrets,
        },
      ]),
    ),
    accepted_op_ids: projection.acceptedOpIds,
    rejected_op_ids: projection.rejectedOpIds,
  };
}

export function identityRosterOpToNostrIdentity(op: IdentityRosterOp): NostrIdentityRosterOp {
  if (op.op === 'add_key') {
    const purposes = (op.key.purposes ?? []).map(identityPurposeToNostrIdentity);
    return {
      op: 'add_facet',
      facet: {
        pubkey: op.key.pubkey,
        ...(op.key.subject !== undefined ? { profile_id: op.key.subject } : {}),
        ...(purposes.length ? { purposes } : {}),
        capabilities: identityCapabilitiesToNostrIdentity(op.key.capabilities),
        added_at: op.key.addedAt,
        ...(op.key.label !== undefined && !purposes.includes('app_key') ? { label: op.key.label } : {}),
      },
    };
  }
  if (op.op === 'tombstone_key') {
    return {
      op: 'tombstone_facet',
      pubkey: op.pubkey,
      ...(op.reason !== undefined ? { reason: op.reason } : {}),
    };
  }
  if (op.op === 'set_key_capabilities') {
    return {
      op: 'set_capabilities',
      pubkey: op.pubkey,
      capabilities: identityCapabilitiesToNostrIdentity(op.capabilities),
    };
  }
  if (op.op === 'rotate_secret_epoch') {
    return {
      op: 'rotate_secret_epoch',
      epoch: op.epoch,
      wrapped_secrets: op.wrappedSecrets,
    };
  }
  return {
    op: 'repair_secret_wraps',
    epoch: op.epoch,
    wrapped_secrets: op.wrappedSecrets,
  };
}

function identityKeyAcceptanceContentToNostrIdentity(
  content: IdentityKeyAcceptanceContent,
): NostrIdentityFacetAcceptanceContent {
  if (content.schema !== NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA) {
    throw new Error(`unsupported NostrIdentity facet acceptance schema ${content.schema}`);
  }
  return {
    schema: NOSTR_IDENTITY_FACET_ACCEPTANCE_SCHEMA,
    profile_id: content.identity,
    facet_pubkey: content.keyPubkey,
    purposes: sortPurposes(content.purposes.map(identityPurposeToNostrIdentity)),
    ...(content.rosterOpId !== undefined ? { roster_op_id: content.rosterOpId } : {}),
    client_nonce: content.clientNonce,
    accepted_at: content.acceptedAt,
  };
}

function nostrIdentityCapabilitiesToIdentity(capabilities: NostrIdentityCapabilities): IdentityKeyCapability[] {
  const normalized = normalizeCapabilities(capabilities);
  return [
    ...(normalized.can_write_roots ? [IDENTITY_CAPABILITY_WRITE] : []),
    ...(normalized.can_admin_profile ? [IDENTITY_CAPABILITY_ADMIN] : []),
    ...(normalized.can_recover_app_keys ? [IDENTITY_CAPABILITY_RECOVER] : []),
    ...(normalized.can_receive_secret_wraps ? [IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS] : []),
    ...(normalized.can_decrypt_secret_epochs ? [IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS] : []),
  ].sort();
}

function identityCapabilitiesToNostrIdentity(capabilities: IdentityKeyCapability[] = []): NostrIdentityCapabilities {
  const nostrIdentity: NostrIdentityCapabilities = {};
  for (const capability of capabilities) {
    if (capability === IDENTITY_CAPABILITY_WRITE) nostrIdentity.can_write_roots = true;
    else if (capability === IDENTITY_CAPABILITY_ADMIN) nostrIdentity.can_admin_profile = true;
    else if (capability === IDENTITY_CAPABILITY_RECOVER) nostrIdentity.can_recover_app_keys = true;
    else if (capability === IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS) nostrIdentity.can_receive_secret_wraps = true;
    else if (capability === IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS) nostrIdentity.can_decrypt_secret_epochs = true;
    else throw new Error(`unsupported NostrIdentity capability ${capability}`);
  }
  return normalizeCapabilities(nostrIdentity);
}

function nostrIdentityPurposeToIdentity(purpose: NostrIdentityKeyPurpose): IdentityKeyPurpose {
  if (purpose === 'app_key') return IDENTITY_PURPOSE_APP;
  if (purpose === 'recovery_phrase') return IDENTITY_PURPOSE_RECOVERY;
  if (purpose === 'nip46_signer') return IDENTITY_PURPOSE_REMOTE_SIGNER;
  return IDENTITY_PURPOSE_PROFILE;
}

function identityPurposeToNostrIdentity(purpose: IdentityKeyPurpose): NostrIdentityKeyPurpose {
  if (purpose === IDENTITY_PURPOSE_APP) return 'app_key';
  if (purpose === IDENTITY_PURPOSE_RECOVERY) return 'recovery_phrase';
  if (purpose === IDENTITY_PURPOSE_REMOTE_SIGNER) return 'nip46_signer';
  if (purpose === IDENTITY_PURPOSE_PROFILE) return 'social_profile';
  throw new Error(`unsupported NostrIdentity purpose ${purpose}`);
}

function normalizeWrappedSecrets(wrappedSecrets: Record<string, string>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(wrappedSecrets)
      .map(([pubkey, wrapped]) => [requireHexPubkey(pubkey, 'wrapped secret recipient'), wrapped] as const)
      .sort(([left], [right]) => left.localeCompare(right)),
  );
}

function encryptedDeviceLabelsExtensionFacts(payload: string | undefined): ReturnType<typeof fact>[] {
  const trimmed = payload?.trim();
  return trimmed ? [fact(NOSTR_IDENTITY_ENCRYPTED_DEVICE_LABELS_FACT, [trimmed])] : [];
}

function requireHexPubkey(value: string, label: string): string {
  const normalized = normalizeHexPubkey(value);
  if (!normalized) throw new Error(`${label} pubkey must be 64-char hex`);
  return normalized;
}
