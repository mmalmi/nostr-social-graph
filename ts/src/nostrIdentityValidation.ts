import type { Event } from 'nostr-tools';
import type { SignedNostrIdentityRosterOp } from './nostrIdentity';
import { stableStringify } from './nostrIdentityJson';
import { parseNostrIdentityRosterOpEvent } from './nostrIdentityEvents';
import { normalizeNostrIdentityRosterOpContent } from './nostrIdentityNormalize';

export interface NostrIdentityRosterOpsContainer {
  roster_ops?: SignedNostrIdentityRosterOp[];
}

export function validateSignedNostrIdentityRosterOps(container: NostrIdentityRosterOpsContainer): void {
  for (const signed of container.roster_ops ?? []) {
    validateSignedNostrIdentityRosterOp(signed);
  }
}

export function signedNostrIdentityRosterOpIsValid(signed: SignedNostrIdentityRosterOp): boolean {
  try {
    validateSignedNostrIdentityRosterOp(signed);
    return true;
  } catch {
    return false;
  }
}

export function validateSignedNostrIdentityRosterOp(signed: SignedNostrIdentityRosterOp): void {
  try {
    const parsed = parseNostrIdentityRosterOpEvent(JSON.parse(signed.event_json) as Event);
    if (
      parsed.op_id !== signed.op_id
      || parsed.signer_pubkey !== signed.signer_pubkey
      || stableStringify(normalizeNostrIdentityRosterOpContent(parsed.content))
        !== stableStringify(normalizeNostrIdentityRosterOpContent(signed.content))
    ) {
      throw new Error('op event_json does not match op fields');
    }
  } catch (error) {
    throw new Error(`NostrIdentity roster ${error instanceof Error ? error.message : String(error)}`);
  }
}
