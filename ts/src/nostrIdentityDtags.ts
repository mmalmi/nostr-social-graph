import type { NostrIdentityId } from './nostrIdentity';
import { isUuid } from './nostrIdentityJson';

export function nostrIdentityRosterOpDTag(profileId: NostrIdentityId, clientNonce: string): string {
  return `nostrIdentity-profile/${profileId}/roster-op/${clientNonce}`;
}

export function nostrIdentityFacetAcceptanceDTag(profileId: NostrIdentityId, clientNonce: string): string {
  return `nostrIdentity-profile/${profileId}/facet-acceptance/${clientNonce}`;
}

export function parseNostrIdentityRosterOpDTag(dTag: string): { profileId: NostrIdentityId; nonce: string } {
  const rest = dTag.startsWith('nostrIdentity-profile/') ? dTag.slice('nostrIdentity-profile/'.length) : '';
  const split = rest.indexOf('/roster-op/');
  if (split <= 0) throw new Error(`invalid NostrIdentity roster d tag: ${dTag}`);
  const profileId = rest.slice(0, split);
  const nonce = rest.slice(split + '/roster-op/'.length);
  if (!isUuid(profileId) || !nonce || nonce.includes('/')) {
    throw new Error(`invalid NostrIdentity roster d tag: ${dTag}`);
  }
  return { profileId, nonce };
}

export function parseNostrIdentityFacetAcceptanceDTag(dTag: string): { profileId: NostrIdentityId; nonce: string } {
  const rest = dTag.startsWith('nostrIdentity-profile/') ? dTag.slice('nostrIdentity-profile/'.length) : '';
  const split = rest.indexOf('/facet-acceptance/');
  if (split <= 0) throw new Error(`invalid NostrIdentity facet acceptance d tag: ${dTag}`);
  const profileId = rest.slice(0, split);
  const nonce = rest.slice(split + '/facet-acceptance/'.length);
  if (!isUuid(profileId) || !nonce || nonce.includes('/')) {
    throw new Error(`invalid NostrIdentity facet acceptance d tag: ${dTag}`);
  }
  return { profileId, nonce };
}
