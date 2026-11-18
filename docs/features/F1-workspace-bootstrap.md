# F1 - Workspace bootstrap

Status: building
Summary: the repo builds, gates and serves: workspace + core types + config + a loopback-only axum server + a React shell with the disclaimer.
Owner area: Cargo.toml, crates/{core,server,cli,...stubs}, config/, fixtures/config/, web/, justfile

<!-- Plan: .agents/plans/F1-workspace-bootstrap.md (10 slices; slice 0 = scaffold, slice 9 fills "What was built"). -->

## Acceptance criteria
- Given a clean checkout, when `just gate` runs, then fmt, clippy `-D warnings`, nextest and web typecheck/lint/vitest pass.
- Given `cargo metadata`, then every crate edge is within the `bz-architecture` table, no domain crate depends on `bazaar-server`, and `live-trading` is in no `default` feature.
- Given `config/`, then it holds exactly the eight files, each with `schema_version`, `verified_on`, `source`; loaders reject unknown keys and float money; `risk.toml` has `execution_mode = "paper"` and enforced `[hard_limits]`.
- Given the fixture calendar, then `MarketCalendar` answers weekend/holiday/special session/out-of-range/expiry shift/session boundary with no hour, date or weekday literal in source.
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

## Decisions
- Toolchain pinned to Rust 1.100.0 (owner, 2026-11-18).
- 15 crates (not 14): the `bz-repo-structure` list is authoritative (owner).
- Crate edges are declared up front in the manifests; the cargo lints `unused_dependencies` and `unused_workspace_dependencies` are set to `allow` in the workspace until F1 ends (reverting is a slice 9 task).
- `InstrumentKind` has the four concrete kinds; `applies_to = "all"` in `calendar.toml` is a loader wildcard, not a variant.
- Additive `StatusDto` subset, header-only config structs for the six data-less files, synthetic fixtures, no CORS, disclaimer constant duplicated Rust/TS with an equality test (plan decisions).

## Known limits
- Slice 0 contains stubs only: no Money, config loader, calendar, server routes or web app yet.
- `calendar.toml` and `charges.toml` will hold no market data until F2; no WebSocket, no store, cli is a stub, no CI workflow.

## Deferred
- CR-F1-1: slice 9 reverts the workspace cargo lint allows `unused_dependencies` and `unused_workspace_dependencies` (Cargo.toml `[workspace.lints.cargo]`).
- CR-F1-S5-1: slice 6 adds a router-level test: every OpenAPI path returns 401 without a cookie (exempt: `/healthz`, `/auth`, Kite callback). Slice 6 also wires layers with `security_headers` outermost, then `host_guard`, `origin_guard`, `problem_backstop` (see `security.rs` module doc).

## History
- 2026-11-18: slice 0 scaffold.
