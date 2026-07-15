# Changelog

## 0.4.0 - 2026-07-16

- Add the transport-only `fips_transport` identity purpose and app-facing facet.
- Make the purpose enum non-exhaustive so future purpose additions do not break
  downstream exhaustive matches.
- Resolve FIPS bindings only from an active admin addition and the transport
  key's matching self-acceptance, with no application capabilities or extra
  purposes.
- Invalidate bindings on tombstone, capability grant, superseding acceptance,
  or re-addition without a fresh acceptance, with shared Rust/TypeScript test
  vectors.
