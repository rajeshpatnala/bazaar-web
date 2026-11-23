# Top 10: dev-core (from 285 findings, 2026-11-23)
1. A new config key is never `serde(default)` to a looser value; absent = load error [RG-F6-2].
2. A limit lives in one config table; a second copy (other crate/toml) is derived or pinned by an equality test [RISK-102-2, QUAN-83-4].
3. Each new key has a test at 0, 1, the ceiling, ceiling+1 and u32::MAX, and a `[hard_limits]` tie [RISK-96-2, RISK-96-1].
4. A proof-type constructor is private to one module: `compile_fail` doctest plus an rg check for the literal elsewhere [RG-F6-1].
5. Every Err/Unknown branch of a store/port adapter in wiring has its own test, not only the happy path [RISK-136-1].
6. Re-gate after reload is tested per limit (TTL, drift, kill switch with `kill_allow_reduce_only` both ways), not for one [RISK-136-2, CR-136-1].
7. A feature PR carries its `docs/features/F<n>` record with the Deferred list [CR-64-1].
8. Scripts: a helper that returns "" on subprocess failure must not let a check pass; handle rename paths (`old => new`) [CR-143-1, SEC-kit-10].
9. A check that reads a review/state file treats an unparsable file as "not verified", never as "no findings" [CR-143-3, CR-143-2].
10. CI: a value written to `$GITHUB_ENV` is not visible in the same step; pin installed tool versions [CR-122-1, CR-122-2].
