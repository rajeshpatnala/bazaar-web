# F1 - Workspace bootstrap

Status: done
Summary: the repo builds, gates and serves: workspace + core types + config + a loopback-only axum server + a React shell with the disclaimer.
Owner area: Cargo.toml, crates/{core,server,cli,...stubs}, config/, fixtures/config/, web/, justfile

<!-- Plan: .agents/plans/F1-workspace-bootstrap.md (10 slices; slice 0 = scaffold, slice 9 fills "What was built"). -->

## Acceptance criteria
- Given a clean checkout, when `just gate` runs, then fmt, clippy `-D warnings`, nextest and web typecheck/lint/vitest pass.
- Given `cargo metadata`, then every crate edge is within the `bz-architecture` table, no domain crate depends on `bazaar-server`, and `live-trading` is in no `default` feature.
- Given `config/`, then it holds exactly the eight files, each with `schema_version`, `verified_on`, `source`; loaders reject unknown keys and float money; `risk.toml` has `execution_mode = "paper"` and enforced `[hard_limits]`.
- Given the fixture calendar, then `MarketCalendar` answers weekend/holiday/special session/out-of-range/expiry shift/session boundary with no hour or date literal in non-test source (Sat/Sun weekend is the only structural weekday fact).
- Given `bazaar-server` with test fakes, then it binds only loopback and `/healthz` answers without a session.
- Given no session cookie, then `/api/v1/**` returns 401 problem+json; wrong Host, Origin or CSRF return 403 with the stable codes; unknown `/api/*` is 404 problem+json.
- Given the launch URL, then `/auth?token=` gives 303 + `bz_session` once; reuse, wrong or expired token gives 403 `launch_token_invalid`.
- Given `bazaar-server openapi`, then the JSON parses, paths start `/api/v1/`, money schemas are strings, `just openapi` leaves `schema.d.ts` unchanged.
- Given any route or API-error state, then the exact disclaimer is visible in `role="contentinfo"`, not dismissible, not sticky/fixed, fits 1280x720 and 1024x600; Rust and TS constants are byte-equal.
- Given 401 from `/api/v1/auth/me`, then the UI shows the session-ended screen; given 200, the PAPER badge shows from `mode`.

## What was built
- Slice 0 (scaffold): root `Cargo.toml` (workspace deps, lints), `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` (wall-clock ban), `.editorconfig`; 15 crate manifests and stubs under `crates/` (core, data, store, indicators, options, regime, strategies, scanner, backtest, risk, execution, paper, broker, server, cli); `core::{clock (trait), segment, mode}`; `bazaar-cli` argument shape (`--config-dir`, `--data-dir`); frozen module lists of `core` and `server`.
- Cargo feature `live-trading`: declared in `bazaar-broker`, forwarded (optional dependency) by `bazaar-cli` and `bazaar-server`, in no default.
- Config keys: none yet (`BAZAAR_PORT`/`BAZAAR_CONFIG_DIR`/`BAZAAR_DATA_DIR` names stay in `.env.example`).

- Slice 3 (core-calendar, PR 21): `core::calendar::MarketCalendar` built from `calendar.toml`: `is_trading_day`, `session_at`, `close_time`, `next/prev_trading_day`, `shift_to_prior_trading_day` (expiry shift). Semantics: a mock session never makes a day tradable; a clearing holiday does not close trading; a year outside the file's covered years is `CalendarError::OutOfRange` (never "open"); special sessions (e.g. Saturday muhurat) override the weekend; instants are UTC and converted to IST inside; no wall-clock reads.
- Slice 6 (server-app, PR 22): `bazaar-server` axum app (`app::build`) with `AppState`, OpenAPI document (`openapi` subcommand), `/healthz` (public, data-free), `/auth` (launch token to `bz_session`), `/api/v1/status`, unknown `/api/*` as 404 problem+json, static UI fallback from `web_dist` when `index.html` exists. Layers outermost first: `security_headers`, trace, `host_guard`, `origin_guard`, `problem_backstop`, timeout (30 s), body limit (1 MiB); session guard before CSRF guard. DTOs are separate from domain types; `live_enabled` comes from `cfg!`. No order route. In-process router tests in `crates/server/tests/app.rs`.
- Slice 7 (server-startup, PR 24): `bazaar-server serve` loads `config/*.toml`, starts tracing (`RUST_LOG`, default info), binds `127.0.0.1` only (`ensure_loopback` assert; no host flag), prints the one-time launch URL to stdout via `writeln!` (never tracing), opens it best effort, and shuts down gracefully on Ctrl-C and SIGTERM. Port: `--port`, else `BAZAAR_PORT`, else 8787 (0 rejected). `BAZAAR_CONFIG_DIR` (default `config`) is a dev/test override of the config dir; the loader still enforces `[hard_limits]` and the mode rules. `BAZAAR_DATA_DIR` is not read yet. The launch URL is also an argument of `open`/`xdg-open` (briefly visible in `ps`; one-time, short-lived token: accepted). TraceLayer logs the path only (`path_only_span`), so `/auth?token=` is never logged. SEC-22-1 done (`web_dist` is the const `web/dist`), SEC-22-2 done (`/healthz` untouched). `deny.toml` reconciled, `cargo deny check` green.
- Slice 8 (web-client, PR 25): web app generated API client (`web/src/api/client.ts`, cookie session, CSRF header on non-GET, 401 session-ended listener), `SessionGate` with a session-ended screen, `ModeBadge` (paper vs other mode, text plus colour, polls status every 30 s), and typed MSW handlers for tests (CR-25-3).

- Slice 9 (workspace-checks): `crates/cli/tests/architecture.rs` (edges from `cargo metadata` against the `bz-architecture` table, no domain crate on a composition root, `live-trading` in no default feature, `bazaar-broker` optional only, no `--all-features` recipe) and `crates/cli/tests/workspace.rs` (env template empty values and no mode var, server/web versions equal, exactly eight config files, Rust vs TS `DISCLAIMER` byte-equal); `README.md`. CR-F1-1 done: the cargo lint allows are removed and the unused dependencies they hid are dropped from crate manifests and `[workspace.dependencies]` (each feature re-adds its edges when it first uses them; dev-deps of `bazaar-cli`: `bazaar-core`, `serde_json`, `toml`).

## Decisions
- AC4 wording: the Sat/Sun weekend is a structural fact in `calendar.rs`, not market data, so it stays a literal (documented in the module doc); hours, holidays, sessions and covered years all come from config (CR-21-1, owner not needed).
- Calendar loader (`config/calendar.rs`) already enforces `start < end` for regular and special sessions and rejects overlapping regular sessions; special-vs-regular overlap is resolved by precedence (SEC-21-1).
- Toolchain pinned to Rust 1.100.0 (owner, 2026-11-18).
- 15 crates (not 14): the `bz-repo-structure` list is authoritative (owner).
- Crate edges are declared when first used (the scaffold's up-front edges were removed in slice 9 with the lint allows); the architecture test checks edges against the allowed table, not that every edge exists.
- `InstrumentKind` has the four concrete kinds; `applies_to = "all"` in `calendar.toml` is a loader wildcard, not a variant.
- Additive `StatusDto` subset, header-only config structs for the six data-less files, synthetic fixtures, no CORS, disclaimer constant duplicated Rust/TS with an equality test (plan decisions).

## Known limits
- Slice 0 contains stubs only: no Money, config loader, calendar, server routes or web app yet.
- `calendar.toml` and `charges.toml` will hold no market data until F2; no WebSocket, no store, cli is a stub, no CI workflow.

## Deferred
- CR-F1-3 / SEC-F1-1: `[hard_limits]` live in the owner-editable `config/risk.toml` (owner decision: file-only). The DecisionGate feature adds an enforced check so a raised ceiling cannot slip through.
- CR-F1-S5-1: slice 6 adds a router-level test: every OpenAPI path returns 401 without a cookie (exempt: `/healthz`, `/auth`, Kite callback). Slice 6 also wires layers with `security_headers` outermost, then `host_guard`, `origin_guard`, `problem_backstop` (see `security.rs` module doc).

- TS-21-1..4, TS-21-6 (tests, F2 calendar-data slice): strengthen the calendar proptest (shift idempotent and never later, lands on a trading day, `session_at` Closed on non-trading days, all segments, `prev(next(x)) == x`); add Saturday-Closed, special-session edges (18:00:00 inclusive, 18:59:59), UTC/IST boundary (2026-01-05T18:29:59Z vs 18:30:00Z) and 2026-12-31T18:30Z OutOfRange, OutOfRange for `close_time`/`shift_to_prior_trading_day`.
- CR-21-3: add a one-line comment in `next/prev_trading_day` that termination relies on the finite covered years (OutOfRange).
- CR-22-1: add a router test for an oversize POST body (413 problem+json); 408 test via an injected tiny timeout or soften the module doc claim (`app.rs`).
- CR-22-3, TS-22-1: first mutating route adds the CSRF success-path test; add foreign-Origin and wrong-CSRF-token cases to `post_without_origin_or_csrf_is_refused` with a comment on guard order.
- CR-22-2: done in this record (slice 6 entry above; the slice 3 and PR 21 text is correct, kept).
- TS-22-2: assert money-typed schemas are strings and the `schema.d.ts` no-diff once the first Decimal DTO lands (`just openapi` no-diff is T2).
- TS-22-3: replace TestApp temp dist dirs with `tempfile::TempDir` so they are removed (`tests/common/mod.rs`).
- SEC-22-1: done in slice 7 (`web_dist` fixed const).
- SEC-22-2: done for now in slice 7; keep `/healthz` data-free as status fields grow.
- SEC-24-1 (owner decision, keep): `BAZAAR_PORT` env fallback stays next to `serve --port` (priority flag > env > default); the host is not configurable and `ensure_loopback` still forces 127.0.0.1. This deviates from the written security rule (port via `--port` only); the rule text should be updated to match.
- SEC-24-2: `BAZAAR_CONFIG_DIR` redirects config loading; treat as dev/test, and confirm the loader still enforces `hard_limits` and refuses `execution_mode = live` without the opt-in (verify in the F1 T2 pass).
- SEC-24-3 (accepted): the launch URL with its one-time token is printed and passed to `open`/`xdg-open`, visible in `ps` briefly; low risk, one-time token.
- SEC-24-4: confirm no TraceLayer logs the URI query so `/auth?token=` never reaches logs (check in the slice 9 / T2 review).
- TS-24-1: add an assert_cmd test that runs `bazaar-server openapi` as a process and parses stdout.
- TS-24-2: non-loopback refusal is tested only via `ensure_loopback` in security.rs; low value, no action unless `bind_addr` takes a host.
- CR-25-1: `unwrap` in `web/src/api/client.ts` throws on an empty 2xx body; check `response.ok` and `error === undefined` before the first non-GET route.
- CR-25-2, SEC-25-1: any 401 fires session-ended but `onSessionEnded` is not wired; `statusQuery` keeps polling after the session ends. Wire it to the `authMe` query cache or add an `enabled` flag, or drop the listener.
- SEC-25-2: `onSessionEnded` is a single module-level slot (last registration wins); use a Set if a second consumer appears.
- TS-25-1: test CSRF header injection in `client.ts` `onRequest` (non-GET gets `X-CSRF-Token`; no token throws) with a temporary msw POST handler, at the latest in the first mutating slice.
- TS-25-2 (optional): assert PAPER vs non-paper differ by class or data attribute, not only text and `data-mode`.

## History
- 2026-11-18: slice 0 scaffold.
- 2026-11-19: slice 9 workspace checks, README, lint allows reverted; status done.
