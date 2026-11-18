# Default stack and versions

(From `bz-core` SKILL.md; open when adding or bumping a dependency.)

## Default stack (verified 2026-11-18; pin in `[workspace.dependencies]`, re-check with `cargo search` before bumping)

| Concern | Choice | Note |
|---|---|---|
| Language | Rust edition 2024, current stable pinned in `rust-toolchain.toml` | check blog.rust-lang.org for the latest stable when bumping |
| Runtime | `tokio` 1.54, one multi-thread runtime | `bazaar-server` and `bazaar-cli` each own exactly one |
| Money / price | `rust_decimal` 1.43 | never `f64` for rupees |
| Time | `chrono` 0.4.45 + `chrono-tz` 0.10 (`Asia/Kolkata`) | one time crate per repo; do not add `time` |
| Errors | `thiserror` per crate; `anyhow` only in `cli`, `server` main and tests | |
| Config | `serde` + `toml` + `secrecy`; `dotenvy` in the bins only | files in `config/` (dir from `--config-dir` / `BAZAAR_CONFIG_DIR`), data in `BAZAAR_DATA_DIR`; secrets from the Keychain (env fallback) |
| Secrets | `keyring-core` + `apple-native-keyring-store` (macOS Keychain); `.env` via `dotenvy` as dev fallback | behind the `SecretStore` trait, held as `SecretString`; call `keyring_core::set_default_store` at startup; compile-check against the pinned version (`bz-security`) |
| Web server | `axum` 0.8, `tower-http` 0.7, `utoipa` 6 + `utoipa-axum` 0.3, `rand`, `subtle` (launch token; in-memory sessions) | `crates/server` is a thin composition root bound to 127.0.0.1 only, see `bz-web-api`; keeps its own literal `version`, equal to `web/package.json` |
| HTTP / WS | `reqwest` 0.13 (`default-features = false`, features `json`, `rustls`), `tokio-tungstenite` 0.30 | one shared client; timeouts on everything |
| SQL | `sqlx` 0.9 (SQLite, feature `chrono`) | app state, journal, audit, paper account |
| Columnar | `duckdb` (pinned with `~`), `polars` 0.55, `arrow`/`parquet` 60 | candles and backtest data; see `bz-market-data`; added in the commit that first uses them |
| Observability | `tracing`, `tracing-subscriber` | |
| Tests | `proptest` 1.12, `criterion` 0.8, `insta` 1.49, `wiremock` 0.6 | see `bz-testing` |
| UI | React 19.3, Vite 8, TypeScript `~6.0` strict (not 7 yet: tooling; `bz-repo-structure` manifests), React Router, TanStack Query 5, `openapi-fetch` + `openapi-typescript`, `lightweight-charts` 5.3 (ESM only), MSW, Vitest 5, Playwright, ESLint 10 | `pnpm`, project in `web/`; API types generated from the server's OpenAPI document (`bz-web-ui`) |
| Delivery | none: local only, `just run` from the repo | no Docker, no deploy target |

Versions come from the blueprint research. If a build fails on a version, check crates.io or npm for the current release before guessing; record a deliberate downgrade in `DECISIONS.md`.
