# Changelog

## Unreleased

- Refresh the library, example, and test dependency graph to supported versions;
  the frozen workspace lock now has no known OSV advisories.

## 2.0.0 - 2026-07-16

- Replace device-approval request URIs with a strict three-field bootstrap URI.
- Add ephemeral-key-signed request events with domain-separated secret commitments and stable AppKey proofs.
- Reject legacy query links, embedded request events, noncanonical keys and secrets, and unknown bootstrap fields.
- Add signed FIPS transport identity facts with shared Rust/TypeScript vectors and strict normalization.

## 1.0.39

- Require strict secret-bound NostrIdentity device approval requests and receipts.
- Verify receipt signatures and bind approvals to both the ephemeral request key and device AppKey.
- Reject duplicate, unknown, malformed, or tampered approval fields consistently in Rust and TypeScript.

## 1.0.36

- Fix `SocialGraphBinary.fromBinary` to recalculate follow distances
  after deserializing instead of trusting the on-disk ordering, so a
  reloaded graph matches one built incrementally.
- `SocialGraph.handleEvent` and the private follow/mute handlers now
  return a boolean indicating whether the graph changed, letting
  callers skip work (e.g. `saveGraph()`) on no-op events.
- Add `SocialGraph.getMuteListCreatedAt(pubkey)`.

## 1.0.33

- Last release on the pre-pnpm workspace layout.
