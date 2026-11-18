# ## 9. Supply chain and build

(From `bz-security` SKILL.md; open when the task touches this area.)


- `deny.toml`: `[advisories]` deny vulnerabilities and unmaintained crates unless an ignore carries a reason and expiry; `[licenses]` allowlist; `[bans]` block `openssl` and `native-tls` (rustls only), duplicate majors reviewed; `[sources]` crates.io only, no git dependencies without an ADR. `cargo deny check` and `pnpm audit --prod` run in CI and `just check`.
- New dependency checklist: maintained recently, `unsafe` footprint, transitive count, licence, and for npm packages install scripts (pnpm `onlyBuiltDependencies` allowlist). Trading-adjacent crates (TA libraries, broker SDKs) are not trusted by default: the Kite client is ours over `reqwest`.
- Critical or high advisories are patched or mitigated within 48 hours; record in `DECISIONS.md`.
- Pin third-party GitHub Actions to a version or SHA; CI holds no broker secrets and no network access to broker APIs. Nothing is packaged or deployed, so there is no image to scan.
