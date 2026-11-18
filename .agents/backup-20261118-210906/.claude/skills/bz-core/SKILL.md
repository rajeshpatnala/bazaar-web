---
name: bz-core
description: Core engineering standard for the Bazaar Rust + axum web trading app - stack and versions, golden rules (Decimal money, IST, no hardcoded market facts), errors, config with verified_on, secrets from the macOS Keychain (env fallback), tracing, async, performance, done checklist. Use when writing or reviewing any Bazaar Rust or TypeScript code.
---

# Bazaar Core Standard

Bazaar is a local-only, single-user web app on the owner's Mac (Rust workspace with an axum server + React/TS UI served from the same origin) for Indian equities and F&O analysis, backtesting and paper trading. Neighbouring skills: `bz-architecture` (crate map, ports, pipeline), `bz-repo-structure` (layout, lints, justfile), `bz-testing`, `bz-security`, `bz-market-data`, `bz-quant`, `bz-backtest-risk`, `bz-execution`, `bz-web-api`, `bz-web-ui`, and the domain skills `bz-india-markets`, `bz-market-strategy`, `bz-trading-discipline`. If another skill conflicts with this file, this file wins; raise it with the tech lead. Longer snippets for money, the `Clock` trait (`SystemClock`, `FakeClock`, feature `test-util`) and config loading are in `references/money-time-config.md`.

## 1. Default stack (verified 2026-11-18; pin in `[workspace.dependencies]`, re-check with `cargo search` before bumping)

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

## 2. Golden rules

1. **Money is `Decimal`.** Rupee amounts, prices, charges, P&L, balances: `rust_decimal`, never `f64`. `f64` is allowed only inside indicator/options math and is converted at one named boundary (`bz-quant`).
2. **Time is IST at the edges, UTC inside.** Store instants as UTC; session logic and display use `chrono_tz::Asia::Kolkata`. Anything time-dependent takes a `Clock`; `Utc::now()` appears only in `SystemClock`.
3. **No hardcoded market facts.** Lot size, tick size, expiry dates, F&O eligibility, ban list, holidays, circuit limits, charge rates and session hours come from the instrument master or `config/*.toml`. A literal like `75`, `0.05`, `"Tuesday"`, `0.0003` in domain code is a bug (`bz-india-markets`).
4. **No panics in library code.** No `unwrap`/`expect`/`panic!`/`todo!`/indexing that can fail outside tests. Startup invariants may `expect` with a comment explaining why.
5. **Legal data only.** Official broker APIs, licensed vendors, official exchange archives, user CSV. No scraping, no unofficial endpoints (`bz-market-data`, `bz-security`).
6. **Secrets never leave the secret types.** `SecretString` in memory, redacting `Debug`, never in logs, URLs, errors, UI events, fixtures or commits.
7. **Every outbound call has a timeout; every loop that waits is cancellable.** Retries only for idempotent calls and with a budget. Order placement is never retried blindly (`bz-execution`).
8. **Look-ahead is a correctness bug.** Strategies and indicators see only data at or before the bar close; see `bz-quant` and `bz-backtest-risk`.
9. **Gate is the single path to an order.** Nothing below `execution` places orders; no `skip_gate`, no approval auto-accept; `live-trading` is never a default feature (`bz-execution`, `bz-trading-discipline`).
10. **Parse, don't validate.** Convert broker JSON, CSV rows and HTTP request bodies into domain types (`Price`, `Qty`, `Symbol`, `Segment`) at the edge with `TryFrom`; the core never handles raw strings for these.
11. **Bound everything.** Channels, caches, batch sizes, concurrent requests, WS subscription count (limit comes from the broker docs, look it up), log volume.
12. **The disclaimer is a product requirement.** Any code that produces a recommendation or screen text must not imply SEBI registration or certainty (`bz-trading-discipline`).

## 3. Idioms

- Newtypes for `Symbol`, `InstrumentToken`, `Money`, `Price`, `Qty`, `Lots`, `OrderId`; exhaustive `match` on `Segment`, `Side`, `OrderState`, `ExecutionMode` (no `_ =>`).
- `Result<T, CrateError>` everywhere; `Option` for "absent", never sentinel values like `0.0` or `-1`.
- Parameters borrow (`&[Candle]`); return owned. Pass `&dyn Clock`/trait objects only at ports; use generics or concrete types elsewhere.
- Private by default; `pub(crate)` before `pub`; docs on every `pub` item; `#[must_use]` on results and builders.
- Enums with data over bool flags (`ExecutionMode::Paper`, not `is_paper: bool`).
- Comments explain why (a market rule, a source link, a unit). Units belong in names (`risk_rupees`, `stop_distance_pts`, `slippage_bps`).
- No `println!`/`dbg!`/`eprintln!`; use `tracing`. In TypeScript, no `console.log` committed, no `any`, no `@ts-ignore` without a reason.
- Strategy modules carry the doc comment required by the product: idea, entry, exit, stop, best regime, known weaknesses.

## 4. Error model

| Layer | Type | Example variants |
|---|---|---|
| `core` | `DomainError` | `InvalidTick`, `NegativeQuantity`, `OutsideSession` |
| `data`, `store` | `DataError`, `StoreError` | `Timeout`, `RateLimited`, `SessionExpired`, `Decode { field }`, `Stale` |
| `risk`, `execution` | `GateRejection`, `ExecError` | `DailyLossLimit`, `KillSwitchOn`, `ApprovalExpired` |
| `server` | `ApiError` (maps to `application/problem+json`) | stable `code` + `message_for_user` only, status from the variant |

- `#[from]` only when the mapping is unambiguous; otherwise map explicitly and add context.
- `GateRejection` is data, not an error to swallow: it carries the rule id and a plain-English reason that the UI shows ("Rejected: daily loss limit of 3% reached").
- Log an error once, at the boundary (HTTP handler, task root), with structured fields. Inner layers return, they do not log-and-return.
- Handlers never return raw `anyhow`/`Debug` text in a response: it may contain paths or tokens. Map to an `ApiError` with a stable `code` (`bz-web-api` owns the problem+json shape).
- Do not match on message strings; do not use `Box<dyn Error>` in library signatures.

## 5. Configuration

- Files in `config/` (eight; schema owners in `bz-repo-structure` section 4): `charges.toml`, `risk.toml`, `strategies.toml`, `watchlists.toml`, `events.toml`, `calendar.toml`, `data.toml`, `compliance.toml`. Each starts with `schema_version`, `verified_on = "YYYY-MM-DD"` and `source = "<url | circular no. | owner>"`. Corporate actions are store data (`bazaar-cli import-actions <csv>`), not a config file.
- Load with `serde` + `#[serde(deny_unknown_fields)]`, then a `validate()` that checks ranges and cross-field rules. Fail at startup naming the key, never the value of a secret.
- The loader resolves the directories as `--config-dir` flag, else `BAZAAR_CONFIG_DIR`, else `./config`; and `--data-dir`, else `BAZAAR_DATA_DIR`, else `./data`. Both are paths, not secrets, and are the only env-driven config besides secrets (`BAZAAR_PORT`, `RUST_LOG` are read by the server entrypoint). Credentials never live in `config/`.
- Defaults for the user profile (capital 1,00,000, 1% risk per trade, 3% daily loss, paper mode) live in the shipped `config/*.toml`, not in code. Code reads config; it does not carry the numbers.
- Charge rates, STT/CTT, exchange fees, brokerage and Kite API fees: never type them from memory or a blog. Fill `charges.toml` from zerodha.com/charges and exchange circulars, set `verified_on`, and put the URLs in `source`. Third-party figures conflict (the F&O STT changed on 1 April 2026), so a rate without a dated official source is rejected in review.
- Staleness: a file whose `verified_on` is older than its configured limit shows a warning badge and a `warn!`; it is never silently trusted for live decisions.
- Execution mode is `[mode] execution_mode` in `risk.toml`, `paper` by default, never set by env var or CLI flag. Key names come from `bz-trading-discipline` only. `auto` is locked; changing the default or unlocking it is an ADR (`bz-architecture`).
- Refreshable market data (instrument master, holiday list, ban list) is data, not config: it lives in the store with a `fetched_at` and is refreshed daily (`bz-market-data`).

## 6. Secrets

- **Agents never open `.env`, key files, credential files, secret mounts or token caches.** To learn whether configuration and connections work, run `just db-check` (prints `OK <check>` / `FAILED <check> <category>` for `sqlite`, `duckdb`, `kite-session`, `instruments`, `audit-chain`; no secrets). Add new keys to `.env.example` as a name with an empty value and document them in the README; there is no mode variable.
- Resolution order inside the program: macOS Keychain (`keyring-core` store behind `SecretStore`) first, then environment variables loaded from a local untracked `.env` by `dotenvy` in the bin entrypoints only (never in libraries or tests). Everything sits behind the `SecretStore` trait, so code asks for a named secret and does not care where it came from; tests use an in-memory fake store.
- Typed `Secrets` struct of `secrecy::SecretString`; expose with `expose_secret()` at the single call site that builds a request header, and nowhere else.
- Kite `api_secret`, `request_token`, `access_token`, the launch token, session cookies and any Telegram bot token: never in logs, `Debug`, error text, URLs, API responses, WebSocket messages, panic messages, snapshots or fixtures. The Kite access token expires daily; it lives in memory and in the Keychain via `SecretStore`, never in SQLite, `data/`, the repo or the browser (`bz-security`).
- Fixtures are recorded from real APIs only after scrubbing; use obviously fake values (`"test-access-token"`). Details in `bz-security` and `bz-market-data`.

## 7. Observability

- `tracing` with a `fmt` layer to stderr (human-readable text, level from `RUST_LOG`); an optional rolling file under `data/logs`. No secrets, no cookies, no request bodies, no full order payloads at `info`. `RUST_LOG` sets the filter.
- `#[tracing::instrument(skip_all, fields(symbol = %sym))]` on adapters, pipeline stages and handlers; skip any argument that might hold a secret or large data.
- Redaction: secrets use `SecretString`; `Debug` on any struct with a token or key field is hand-written or derived through `secrecy`. A tracing layer or test asserts that a sample token string never appears in captured output.
- Levels: `error` needs the user or a developer, `warn` degraded but handled (reconnect, stale data, stale config), `info` lifecycle and each order state change, `debug` detail, `trace` per-tick (off by default).
- High-frequency paths (ticks, per-bar strategy evaluation) never log at `info` or above.
- The order audit log is a product feature (append-only table, `bz-execution`), not a `tracing` output; do not substitute one for the other.

## 8. Async and concurrency

- One runtime. Blocking work (DuckDB queries, Parquet reads, backtests, scans) goes through `spawn_blocking` or `rayon`, never directly on async tasks.
- Structured shutdown: a `CancellationToken` (tokio-util) passed to every long-lived task (WS client, bar builder, scheduler); tasks live in a `JoinSet`, and panics/errors propagate to the supervisor.
- Timeouts: `tokio::time::timeout` around every network call and every `await` that depends on a remote peer. Defaults come from config; do not scatter literals.
- Channels: bounded `mpsc`; for ticks prefer a drop-oldest or "latest value per instrument" policy over unbounded queues. A slow UI must never stall the feed.
- Never hold a lock across `.await`. Prefer message passing to shared mutable state; `std::sync::Mutex` for short critical sections.
- `select!` branches are cancellation-safe or documented as not. Reconnect loops use backoff with jitter and a cap, and re-subscribe after reconnect (`bz-market-data`).
- Time-based logic (market open, square-off, approval expiry) is driven by the `Clock` so tests can advance time without sleeping.

## 9. Performance

- Measure first: `criterion` benches for indicators, the backtest loop and a full-universe scan; keep baselines when changing hot code.
- Indicators are streaming (O(1) update per bar) with a batch driver over the same implementation. No re-computing a window from scratch per bar.
- Scans over the universe use `rayon` on a blocking thread; each strategy evaluation borrows a slice view, no per-bar `Vec` allocation or clone in hot loops; pre-size collections.
- Push filters and aggregates into DuckDB/Polars (`SELECT` only the symbols, date range and columns needed); keep one DuckDB connection per task, reuse prepared statements.
- Tick fan-out to the UI is throttled (coalesce per instrument, e.g. a few updates per second) before it is sent over the WebSocket; a slow client is dropped or skipped, never buffered without bound.
- Cache the instrument master and calendar in memory with a daily refresh; never parse the instrument CSV per request.
- Do not optimise at the cost of correctness: Decimal on money paths is not negotiable; a faster `f64` ledger is a defect.

## 10. TypeScript side (summary; depth in `bz-web-ui`)

- `strict: true`, no `any`. Backend types come from the generated `web/src/api/schema.d.ts` (`just openapi`), never hand-written duplicates.
- Money and prices cross the API as strings; format with `Intl.NumberFormat("en-IN")` and IST-aware formatters; never sum money with `number`.
- Server state in TanStack Query; local UI state in React state. Components do not call `fetch` directly; they use the typed `openapi-fetch` client in `src/api/` so MSW can mock it in tests.
- The disclaimer component is rendered by the app shell on every screen.

## 11. Security red flags (grep before every PR)

```
rg -n "f64" crates --glob '!**/indicators/**' --glob '!**/options/**'     # money drift
rg -n "unwrap\(\)|expect\(|panic!|todo!|unimplemented!" crates --glob '!**/tests/**'
rg -n "Utc::now|SystemTime::now|Local::now" crates                        # must only be in the Clock impl
rg -n "println!|dbg!|eprintln!" crates
rg -n "skip_gate|bypass|force_order|auto_approve"                         # gate bypass
rg -in "(api_secret|access_token|request_token|password|secret_key)\s*[:=]\s*\"" .   # literals
```

Also: any `reqwest` call without a timeout, any URL not on an approved host list, any `unsafe` (the workspace forbids it), any new dependency without a note in the PR, `live-trading` in a default feature list.

## 12. Tests (details in `bz-testing`)

Unit tests ship in every PR: a function with money, time or a market rule gets a table-driven test with edge cases (zero, negative, exactly at the limit, holiday, half-session day). Tests never call live APIs and never read `.env` or secret files; they use fixtures in `fixtures/`, `wiremock`, a local WS server, `FakeClock`, a fake `SecretStore`, in-memory SQLite, temp dirs and in-process router tests (`tower::ServiceExt::oneshot`). Commands: `just gate` (fmt, clippy `-D warnings`, nextest, then `pnpm -C web typecheck`, `lint`, `test:ci` once `web/package.json` exists) and `just check` (adds doc tests, `cargo deny check`, `pnpm -C web audit --prod`).

## 13. Done checklist

1. `just gate` passes; `just check` for a feature-level change.
2. New behaviour, new error variants and each new config key have tests; money and time paths have edge-case tests.
3. No `f64` money, no hardcoded market fact, no `unwrap`/`expect` in library code, no new `allow`, `unsafe` or `todo!`.
4. Config keys documented with `verified_on` and `source`; `.env.example` updated if a key was added; no secret in code, logs, fixtures or snapshots.
5. Timeouts, cancellation and bounded channels present on every new async path.
6. Public items documented; `DECISIONS.md` updated for any ADR trigger (new data vendor, execution mode change, new crate, storage format) per `bz-architecture`.
7. Anything that touches recommendations, risk, execution or `risk.toml` is flagged for `risk-guardian` and `quant-reviewer` review.
8. Rates, limits and rules you were unsure of are looked up from the official source and cited in the PR, or left as an open question. Never guessed.
