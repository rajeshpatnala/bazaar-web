# Manifests: Cargo workspace, toolchain, server crate, web/package.json

Open this when bootstrapping, adding a dependency or changing build setup. Versions below were checked on crates.io / npm on 2026-11-18; re-check at bootstrap and record deliberate deviations in `DECISIONS.md`. Items marked "compile-check" were not verified against the exact pinned release (feature names, signatures): build them before relying on them.

## 1. Root `Cargo.toml`

```toml
[workspace]
resolver = "3"                       # default for edition 2024
members = ["crates/*"]

[workspace.package]
edition = "2024"
rust-version = "1.100"               # equals rust-toolchain.toml; raise deliberately
license = "Proprietary"
publish = false
version = "0.1.0"

[workspace.dependencies]
# internal: one entry per crate, features enabled by the consumer
bazaar-core = { path = "crates/core" }
bazaar-data = { path = "crates/data" }
bazaar-store = { path = "crates/store" }
bazaar-risk = { path = "crates/risk" }
# ... indicators, options, regime, strategies, scanner, backtest, execution, paper, broker

# external: versions live ONLY here
tokio = { version = "1.54", features = ["rt-multi-thread", "macros", "time", "sync", "signal"] }
rust_decimal = { version = "1.43", features = ["serde-with-str", "maths"] }
chrono = { version = "0.4.45", default-features = false, features = ["clock", "serde", "std"] }
chrono-tz = { version = "0.10", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "1"
thiserror = "2"
anyhow = "1"                         # binaries only
secrecy = "0.10"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
dotenvy = "0.15"                     # bins only, secrets fallback
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "sqlite", "migrate", "macros", "chrono"] }
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls"] }
tokio-tungstenite = "0.30"

# web server (consumed only by crates/server)
axum = { version = "0.8", features = ["ws", "macros"] }                  # 0.8.9 on crates.io
axum-extra = { version = "0.12", features = ["cookie", "typed-header"] }
tower = "0.5"                        # ServiceExt::oneshot in in-process tests
tower-http = { version = "0.7", default-features = false, features = ["trace", "compression-gzip", "timeout", "fs", "set-header"] }   # 0.7.1; no "cors"
utoipa = { version = "6", features = ["chrono", "decimal", "uuid"] }             # 6.0.0
utoipa-axum = "0.3"                  # 0.3.0
subtle = "2.6"                       # constant-time compare of the launch token
getrandom = "0.4"                    # 32-byte launch token and session ids from the OS RNG

# secrets on the owner's Mac (compile-check against the pinned versions)
keyring-core = "1"                   # 1.0.1
apple-native-keyring-store = { version = "1", default-features = false, features = ["keychain"] }   # 1.0.2; features "keychain" / "protected", none by default

# dev only
proptest = "1.12"
insta = { version = "1.49", features = ["json", "redactions"] }
wiremock = "0.6"
criterion = "0.8"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "warn"
unreachable_pub = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "warn"
panic = "warn"
todo = "warn"
dbg_macro = "deny"
print_stdout = "deny"                # cli writes through its output module
print_stderr = "deny"
float_cmp = "deny"                   # compare Decimals or use tolerances
module_name_repetitions = "allow"
missing_errors_doc = "allow"
must_use_candidate = "allow"

[profile.dev.package."*"]
opt-level = 1
[profile.test]
opt-level = 1                        # numeric tests (backtest, options) stay fast
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
```

Notes:
- Group lints need `priority = -1` so the individual lines override them.
- `reqwest` without `rustls` has no TLS and fails at runtime on HTTPS; the `rustls` feature pulls aws-lc, whose licences `deny.toml` must allow. Never enable `native-tls` or `openssl`.
- `sqlx` carries `chrono` so `DateTime`/`NaiveDate` columns map without manual conversion.
- **Added in the commit that first uses them** (reason in the PR): `polars` 0.55, `duckdb` pinned with a patch-level tilde (`duckdb = "~1.20000"`; its version scheme encodes the DuckDB release, so a minor bump is a DuckDB upgrade), `arrow`/`parquet` 60, `rayon`, `tokio-util`.
- Forbidden in the server stack: `tower-sessions`, `argon2`, `totp-rs`, and the `cors` feature of `tower-http`. Sessions are an in-memory map in `crates/server` (`bz-web-api`, `bz-security`).
- tower-http feature names (`compression-gzip`, `fs`, `set-header`) sit behind the contract's "compression, serve-dir, set-header": compile-check them. Body limit uses axum's `DefaultBodyLimit` (1 MiB), the 30 s timeout the `timeout` feature.
- `utoipa` must render `Decimal` fields as strings in the schema; confirm against the pinned version and the DTO tests in `bz-web-api`.

### Keychain secrets

`keyring-core` is the API crate and `apple-native-keyring-store` the macOS credential store; one default store is installed at process start, then entries are created normally. The adapter implements the `SecretStore` trait (`bz-core`) and returns `SecretString`; nothing outside it touches `keyring_core`. Shape only (compile-check against the pinned version; the constructor returns an `Arc` store and `Entry` takes a service and a user):

```rust
// composition roots only (bazaar-server / bazaar-cli main), never in core
#[cfg(target_os = "macos")]
fn install_keychain() -> keyring_core::Result<()> {
    let store = apple_native_keyring_store::keychain::Store::new()?;   // login keychain
    keyring_core::set_default_store(store);
    Ok(())
}
```

Declare `apple-native-keyring-store` under `[target.'cfg(target_os = "macos")'.dependencies]` in the crate that hosts the adapter so Linux CI still builds; off macOS the env-var fallback (`dotenvy`, bins only) is used. Tests use an in-memory `FakeSecretStore`, never the real Keychain.

## 2. Member manifest

```toml
[package]
name = "bazaar-indicators"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[lints]
workspace = true                     # cannot be combined with a local [lints.*] table

[dependencies]
bazaar-core.workspace = true
thiserror.workspace = true

[dev-dependencies]
bazaar-core = { workspace = true, features = ["test-util"] }   # FakeClock, fixture_path()
proptest.workspace = true
```

**Server manifest and version files:** `crates/server/Cargo.toml` (package `bazaar-server`, a composition root) has its **own literal** `version = "x.y.z"`, not `version.workspace = true`, so release-please can bump it. It must equal `version` in `web/package.json`; release-please bumps both and a workspace test asserts they agree. Other members keep `version.workspace = true`. The binary has an `openapi` subcommand that prints the OpenAPI document to stdout (`just openapi` uses it) and a `--port` flag; nothing else changes the bind address.

```toml
[package]
name = "bazaar-server"
version = "0.1.0"                    # literal: release-please bumps it
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[features]
default = []
live-trading = ["bazaar-broker/live-trading"]
```

Feature rules: `live-trading` is declared in `bazaar-broker` and forwarded by `bazaar-cli` and `bazaar-server`. It is never in `default`, never in a `--all-features` recipe, and CI compiles and lints it separately (`cargo check` and `cargo clippy -p bazaar-broker --features live-trading -- -D warnings`). Review hook: `bz-trading-discipline`.

Lint exceptions:
- Tests may `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]` in `lib.rs`.
- `clippy.toml` bans wall-clock reads outside the clock port: `disallowed-methods = ["chrono::Utc::now", "chrono::Local::now", "std::time::SystemTime::now"]`. `SystemClock` in `core` is the single `#[allow(clippy::disallowed_methods)]`. Verify the paths resolve by temporarily calling one.

## 3. Toolchain and formatting

```toml
# rust-toolchain.toml (stable 1.100.0, released 2026-11-12; bump with rust-version in its own PR)
[toolchain]
channel = "1.100.0"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

`rustfmt.toml`: `edition = "2024"`, `max_width = 100`, `use_field_init_shorthand = true` (stable options only). `.editorconfig`: LF, UTF-8, final newline, 4 spaces for `.rs`, 2 for `.ts/.tsx/.json/.toml/.yml`. Gate tools: `cargo-nextest`, `just`, `cargo-deny` (install notes live in the kit README, not in recipes). Edition 2024: `gen` is reserved, RPIT captures lifetimes implicitly, `unsafe extern` syntax. `.cargo/config.toml` is created only for a concrete need (an alias or linker setting) and then explained in `DECISIONS.md`.

## 4. `web/package.json`

```json
{
  "name": "bazaar-web",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "packageManager": "pnpm@<version you bootstrapped with>",
  "scripts": {
    "dev": "vite",
    "dev:mock": "VITE_MOCK_API=1 vite --port 5173 --strictPort",
    "build": "pnpm typecheck && vite build",
    "typecheck": "tsc --noEmit",
    "lint": "eslint . --max-warnings 0",
    "format": "prettier --write .",
    "format:check": "prettier --check .",
    "test": "vitest",
    "test:ci": "vitest run --coverage",
    "e2e": "playwright test",
    "gen:api": "openapi-typescript openapi.json -o src/api/schema.d.ts"
  },
  "dependencies": {
    "react": "^19.3", "react-dom": "^19.3", "react-router": "^8.5",
    "@tanstack/react-query": "^5", "openapi-fetch": "^0.17", "lightweight-charts": "^5.3"
  },
  "devDependencies": {
    "typescript": "~6.0", "vite": "^8", "vitest": "^5", "eslint": "^10", "prettier": "^3",
    "msw": "^3", "@playwright/test": "^1.65", "openapi-typescript": "^7.13"
  }
}
```

- `web/vite.config.ts` proxy (matches the contract's dev wiring; `just ui-dev`, human only, starts a debug server and Vite together):

```ts
server: {
  port: 5173, strictPort: true,
  proxy: {
    "/api": { target: "http://127.0.0.1:8787", changeOrigin: true, ws: true },
    "/auth": { target: "http://127.0.0.1:8787", changeOrigin: true },
  },
},
```

  Caveat: the server's Host allowlist accepts only `127.0.0.1:<port>`/`localhost:<port>` of its own port, so the proxy must rewrite Host (`changeOrigin: true`); the Origin header stays `http://localhost:5173`, which the server accepts in debug builds only (`bz-web-api`, `bz-security`). In debug builds the server prints and opens the launch link on `http://localhost:5173`, so the cookie belongs to the page you browse.
- `dev:mock`, the MSW handlers and the Playwright `baseURL` are defined once in `bz-testing` `references/ui-mocked-api.md`; keep the script identical to the contract (port 5173, `--strictPort`). `just app-dev` runs it.
- `test` (`vitest` watch) is for humans; agents and CI use `test:ci`.
- `gen:api` reads `web/openapi.json` (gitignored, produced by `just openapi`) and writes `web/src/api/schema.d.ts`, which is committed, generated, never hand-edited and excluded from lint, format and coverage. CI regenerates it and fails on drift.
- Versions: React 19.3, Vite 8, TypeScript `~6.0` (as of 2026-11-18 typescript-eslint 8.73 needs `<6.1` and TS 7 has no compiler API; openapi-typescript 7.13 still declares peer `^5`, so pnpm warns: if `gen:api` breaks, run it with `pnpm dlx -p typescript@5 -p openapi-typescript@7.13 openapi-typescript ...` and log it in `DECISIONS.md`), Vitest 5, ESLint 10 (flat config `eslint.config.js`), lightweight-charts 5.3 (ESM only), TanStack Query 5, React Router 8.5, MSW 3, openapi-typescript 7.13, openapi-fetch 0.17, Playwright 1.65. Check Prettier's current major at bootstrap. Usage: `bz-web-ui`.
- `tsconfig.json`: `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `noEmit`.
- `web/.nvmrc` holds the Node major used locally and by CI (`actions/setup-node` reads it): use the current LTS at bootstrap. Commit `web/pnpm-lock.yaml`; CI and `just setup` install with `--frozen-lockfile`.

## 5. Build wiring

- `pnpm -C web build` writes `web/dist/`. The server is a plain Rust binary; nothing in `cargo` needs `web/dist` at compile time, so clean-checkout clippy and tests need no pre-created directory.
- `just run` builds the UI, then runs the release server from the repo root, which serves `web/dist` (tower-http `ServeDir`, same origin). No environment variable selects the directory and the server cannot bind a non-loopback address.
- There is no image, installer or deploy step. Headers, cookies, CSP and rate limits: `bz-security` only.
