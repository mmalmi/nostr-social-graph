import { projectIdentityRoster, type SignedIdentityRosterOp } from './identityGraph';
import type {
  NostrIdentityId,
  NostrIdentityRosterProjection,
  NostrIdentityTombstone,
  SignedNostrIdentityRosterOp,
} from './nostrIdentity';
import {
  identityRosterProjectionToNostrIdentity,
  signedNostrIdentityRosterOpToIdentity,
} from './nostrIdentityEvents';
import { normalizeRosterOp } from './nostrIdentityNormalize';
import { signedNostrIdentityRosterOpIsValid } from './nostrIdentityValidation';

export function nostrIdentityRosterParentIds(ops: SignedNostrIdentityRosterOp[]): string[] {
  const profileId = ops[0]?.content.profile_id;
  return profileId ? projectNostrIdentityRoster(profileId, ops).accepted_op_ids : [];
}

export function projectNostrIdentityRoster(
  profileId: NostrIdentityId,
  ops: SignedNostrIdentityRosterOp[],
): NostrIdentityRosterProjection {
  const valid: SignedIdentityRosterOp[] = [];
  const rejected: string[] = [];
  const sorted = ops
    .filter((op) => op.content.profile_id === profileId)
    .slice()
    .sort((a, b) => (a.content.created_at - b.content.created_at) || a.op_id.localeCompare(b.op_id));
  for (const signed of sorted) {
    if (!signedNostrIdentityRosterOpIsValid(signed)) {
      rejected.push(signed.op_id);
      continue;
    }
    valid.push(signedNostrIdentityRosterOpToIdentity(signed));
  }
  const projection = identityRosterProjectionToNostrIdentity(projectIdentityRoster(profileId, valid));
  projection.rejected_op_ids = rejected.concat(projection.rejected_op_ids);
  return projection;
}

export function applyRosterOp(
  projection: NostrIdentityRosterProjection,
  signed: SignedNostrIdentityRosterOp,
): boolean {
  const op = normalizeRosterOp(signed.content.op);
  const signer = signed.signer_pubkey;
  const isBootstrap = projection.accepted_op_ids.length === 0
    && op.op === 'add_facet'
    && op.facet.pubkey === signer
    && Boolean(op.facet.capabilities?.can_admin_profile);
  const canAdmin = isBootstrap || Boolean(projection.active_facets[signer]?.capabilities?.can_admin_profile);
  const signerFacet = projection.active_facets[signer];
  const canRecover = Boolean(signerFacet?.capabilities?.can_recover_app_keys);
  const canDecryptSecretEpochs = Boolean(signerFacet?.capabilities?.can_decrypt_secret_epochs);
  const canRecoverRoster = canRecover && (
    (op.op === 'add_facet' && op.facet.purposes?.includes('app_key'))
    || op.op === 'tombstone_facet'
    || ((op.op === 'rotate_secret_epoch' || op.op === 'repair_secret_wraps') && canDecryptSecretEpochs)
  );
  const canRepairEpoch = op.op === 'repair_secret_wraps'
    && projection.secret_epochs[String(op.epoch)]?.signed_by_pubkey === signer;
  if (!canAdmin && !canRecoverRoster && !canRepairEpoch) return false;

  if (op.op === 'add_facet') {
    delete projection.tombstones[op.facet.pubkey];
    projection.active_facets[op.facet.pubkey] ??= {
      ...op.facet,
      purposes: op.facet.purposes ?? [],
      capabilities: op.facet.capabilities ?? {},
    };
    return true;
  }
  if (op.op === 'tombstone_facet') {
    const profileId = projection.active_facets[op.pubkey]?.profile_id;
    delete projection.active_facets[op.pubkey];
    const tombstone: NostrIdentityTombstone = {
      pubkey: op.pubkey,
      removed_by_pubkey: signed.signer_pubkey,
      removed_at: signed.content.created_at,
      reason: op.reason,
    };
    if (profileId) tombstone.profile_id = profileId;
    projection.tombstones[op.pubkey] = tombstone;
    return true;
  }
  if (op.op === 'set_capabilities') {
    const facet = projection.active_facets[op.pubkey];
    if (!facet || projection.tombstones[op.pubkey]) return false;
    facet.capabilities = op.capabilities;
    return true;
  }
  if (op.op === 'rotate_secret_epoch') {
    projection.secret_epochs[String(op.epoch)] = {
      epoch: op.epoch,
      created_at: signed.content.created_at,
      signed_by_pubkey: signed.signer_pubkey,
      wrapped_secrets: op.wrapped_secrets ?? {},
    };
    return true;
  }
  if (op.op === 'repair_secret_wraps') {
    const epoch = projection.secret_epochs[String(op.epoch)];
    if (!epoch || epoch.signed_by_pubkey !== signed.signer_pubkey) return false;
    epoch.wrapped_secrets = { ...epoch.wrapped_secrets, ...(op.wrapped_secrets ?? {}) };
    return true;
  }
  return false;
}
