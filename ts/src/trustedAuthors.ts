export type TrustedAuthorSelection = {
  /** The explicitly selected perspective, never inferred from incoming claims. */
  rootPubkey: string;
  /** Authors independently admitted by the application's membership policy. */
  eligibleAuthors: Iterable<string>;
  /** The root's own direct trust choices; descendants do not grant authority. */
  directFollows: ReadonlySet<string>;
  mutedAuthors?: ReadonlySet<string>;
};

const HEX_PUBKEY = /^[0-9a-f]{64}$/;

/**
 * Select nondelegated signing authority: eligible root/direct authors only.
 *
 * Admission and authority are deliberately separate. An admitted author cannot
 * mint additional authority by following or endorsing other accounts. Inputs
 * must come from authenticated application state; this function does not verify
 * events or establish that distinct keys represent distinct people. Runtime is
 * linear in eligibleAuthors and retained state is bounded by the trust set.
 */
export function chooseTrustedAuthors({
  rootPubkey,
  eligibleAuthors,
  directFollows,
  mutedAuthors,
}: TrustedAuthorSelection): Set<string> {
  if (!HEX_PUBKEY.test(rootPubkey)) throw new Error("Invalid root public key");
  const trusted = new Set<string>();
  for (const author of eligibleAuthors) {
    if (
      HEX_PUBKEY.test(author) &&
      (author === rootPubkey || directFollows.has(author)) &&
      !mutedAuthors?.has(author)
    ) {
      trusted.add(author);
    }
  }
  return new Set([...trusted].sort());
}

/** Count at most one signal per authorized signing key, regardless of replays. */
export function countDistinctTrustedAuthors(
  authors: Iterable<string>,
  trustedAuthors: ReadonlySet<string>,
): number {
  const seen = new Set<string>();
  for (const author of authors) {
    if (trustedAuthors.has(author)) seen.add(author);
  }
  return seen.size;
}
