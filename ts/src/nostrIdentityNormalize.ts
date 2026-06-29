import type {
  NostrIdentityFacet,
  NostrIdentityKeyPurpose,
  NostrIdentityRosterOp,
  NostrIdentityRosterOpContent,
} from './nostrIdentity';
import { normalizeCapabilities } from './nostrIdentity';
import { sortRecord } from './nostrIdentityJson';

export function normalizeNostrIdentityRosterOpContent(
  content: NostrIdentityRosterOpContent,
): NostrIdentityRosterOpContent {
  return {
    schema: content.schema,
    profile_id: content.profile_id,
    actor_pubkey: content.actor_pubkey,
    ...(content.actor_seq !== undefined ? { actor_seq: content.actor_seq } : {}),
    ...(content.parents?.length ? { parents: content.parents.slice() } : {}),
    client_nonce: content.client_nonce,
    created_at: content.created_at,
    op: normalizeRosterOp(content.op),
  };
}

export function normalizeRosterOp(op: NostrIdentityRosterOp): NostrIdentityRosterOp {
  if (op.op === 'add_facet') {
    return {
      op: 'add_facet',
      facet: normalizeFacet(op.facet),
    };
  }
  if (op.op === 'tombstone_facet') {
    return {
      op: 'tombstone_facet',
      pubkey: op.pubkey,
      ...(op.reason !== undefined ? { reason: op.reason } : {}),
    };
  }
  if (op.op === 'set_capabilities') {
    return {
      op: 'set_capabilities',
      pubkey: op.pubkey,
      capabilities: normalizeCapabilities(op.capabilities),
    };
  }
  if (op.op === 'rotate_secret_epoch') {
    const wrapped = sortRecord(op.wrapped_secrets ?? {});
    return {
      op: 'rotate_secret_epoch',
      epoch: op.epoch,
      ...(Object.keys(wrapped).length ? { wrapped_secrets: wrapped } : {}),
    };
  }
  const wrapped = sortRecord(op.wrapped_secrets ?? {});
  return {
    op: 'repair_secret_wraps',
    epoch: op.epoch,
    ...(Object.keys(wrapped).length ? { wrapped_secrets: wrapped } : {}),
  };
}

export function normalizeFacet(facet: NostrIdentityFacet): NostrIdentityFacet {
  const purposes = facet.purposes?.length ? sortPurposes(facet.purposes) : [];
  const isAppKey = purposes.includes('app_key');
  return {
    pubkey: facet.pubkey,
    ...(facet.profile_id !== undefined ? { profile_id: facet.profile_id } : {}),
    ...(purposes.length ? { purposes } : {}),
    capabilities: normalizeCapabilities(facet.capabilities ?? {}),
    added_at: facet.added_at,
    ...(facet.label !== undefined && !isAppKey ? { label: facet.label } : {}),
  };
}

export function rosterOpMentionedPubkeys(op: NostrIdentityRosterOp): string[] {
  if (op.op === 'add_facet') return [op.facet.pubkey];
  if (op.op === 'tombstone_facet' || op.op === 'set_capabilities') return [op.pubkey];
  return Object.keys(op.wrapped_secrets ?? {}).sort();
}

export function sortPurposes(purposes: NostrIdentityKeyPurpose[]): NostrIdentityKeyPurpose[] {
  const unique = Array.from(new Set(purposes));
  if (unique.length === 0) {
    throw new Error('NostrIdentity facet acceptance purposes must not be empty');
  }
  return unique.sort((a, b) => purposeRank(a) - purposeRank(b));
}

export function purposeRank(purpose: NostrIdentityKeyPurpose): number {
  switch (purpose) {
    case 'app_key':
      return 0;
    case 'recovery_phrase':
      return 1;
    case 'nip46_signer':
      return 2;
    case 'social_profile':
      return 3;
  }
}
