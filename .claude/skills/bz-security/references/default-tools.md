# ## 1. Default tools (as of 2026-11-18)

(From `bz-security` SKILL.md; open when the task touches this area.)


| Concern | Choice | Note |
|---|---|---|
| Server | `axum` 0.8 (0.8.9 current), features `ws`; `axum-extra` with `cookie` | middleware via `axum::middleware::from_fn_with_state` |
| Headers, limits | `tower-http` 0.7: features `set-header`, `timeout`, `trace`, `fs`; axum `DefaultBodyLimit` | no `cors` feature. tower-http 0.7 also ships a `csrf` feature: do not use it as the Origin check, it lets requests with neither `Origin` nor `Sec-Fetch-Site` through; ours rejects them |
| Randomness | `getrandom` 0.4 (`getrandom::fill`) | launch token, session ids, CSRF tokens, nonces; 32 bytes each |
| Constant-time compare | `subtle` 2.6 (`ConstantTimeEq`) | token and CSRF compares |
| Sessions | in-memory `HashMap` behind a `Mutex` in `crates/server` | restart logs out; no `tower-sessions`, `argon2`, `totp-rs` |
| Secret types | `secrecy::SecretString` everywhere + redacting `Debug` | `bz-core` |
| Secret store | `keyring-core` 1.0 + `apple-native-keyring-store` 1.0 (macOS Keychain) behind `SecretStore` | `.env` via `dotenvy` as dev fallback in bin entrypoints only |
| Rust supply chain | `cargo deny check` (advisories, licenses, bans, sources) | `just check` |
| JS supply chain | `pnpm -C web audit --prod`, lockfile committed, `--frozen-lockfile` in CI | |
| Secret scanning | `gitleaks` on every PR (`secrets-scan` CI job) | |
| Lint | `unsafe_code = "forbid"`; clippy `unwrap_used`, `expect_used`, `dbg_macro`, `print_stdout` | `bz-repo-structure` |
