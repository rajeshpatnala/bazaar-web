# Project profile

The manager fills this in with the user at the start (keep it under 60 lines). Every agent reads it.
Filled 2026-11-18 with the owner; updated 2026-11-21 (live mode, UI direction), 2026-11-27 (reviewed: scope and targets kept; position tracking, DuckDB). Stack and layering: `bz-repo-structure`, `bz-architecture`.

## Purpose
Bazaar is a personal, single-user trading assistant for Indian equities and F&O, running on the owner's Mac.
Scope: NSE/BSE cash equity now (intraday and swing trading); F&O is a later phase (design must not block it).
It scans the market, produces rule-based recommendations with plain-English "Why?" panels, backtests strategies
(Indian cost model), runs a paper account by default (paper and live are both supported, switched in the UI; live is selected server-side),
and routes any order through the DecisionGate and an explicit
user approval. There is no "place order" endpoint. Required, not yet in the build (owner, 2026-11-23): track each open position daily, watching price behaviour and events,
and give rule-based hold / sell / reduce / add suggestions over the trade's life (every order still gated and approved). Not investment advice; the disclaimer stays visible.

## Consumers / API style
One user, in a browser on the same Mac. axum 0.8 server bound to 127.0.0.1 only: REST `/api/v1` (JSON, problem+json
errors, utoipa OpenAPI -> generated TS types) plus a WebSocket feed. Auth: launch-token session; every route except
`/healthz`, `/auth` and the Kite callback needs a session (CONTRACTS section 4). Serves the React 19 + Vite UI.

## Data stores
| store | role | access | notes |
|---|---|---|---|
| SQLite (`data/bazaar.sqlite`) | app state, approvals, kill switch, job runs, journal, paper account, audit log (sessions are in memory only) | sqlx 0.9, migrations in `crates/store/migrations` | |
| Parquet + DuckDB (`data/candles/`) | candles, bhavcopy history, backtest data | duckdb crate, partitioned by symbol/year | |
| Zerodha Kite Connect | live ticks (WebSocket), historical candles, instruments, margins; broker orders only via the `live-trading` feature (never default), after approval | REST + WebSocket, daily login redirect to `/api/v1/kite/callback` | |
| NSE/BSE bhavcopy archives | end-of-day universe, delivery % | official downloads only | |

## Integrations
- Zerodha Kite Connect (REST + WebSocket); secrets in the macOS Keychain (env fallback), never in the repo.
- TradingView: charting only - TradingView widgets/chart links in the UI alongside our own lightweight-charts. No scraping
  and no data extraction from TradingView; any inbound alert/webhook idea needs the owner's approval first.
  Zerodha (Kite) stays the only data and broker source.
- DuckDB links the system library, not the bundled build (BZ-106, 2026-11-21): the version must match the crate (`just --list` has the check); paths in `.cargo/config.toml`. A Dockerfile install step comes later.
- NSE/BSE official bhavcopy downloads. No scraping. Live broker crate only behind the `live-trading` feature (never default); paper stays the default mode.

## Input data / parsing
Kite JSON and binary tick frames; bhavcopy CSV/ZIP; instrument master CSV; Parquet for stored candles; config in
`config/*.toml` (each fact carries `source` and `verified_on`). Reject policy: malformed or out-of-order rows are
logged and skipped, never silently repaired; stale data blocks recommendations.

## Non-functional targets
All targets below are configurable in `config/*.toml` (never hardcoded), with `source`/`verified_on` where applicable.
- Tick to UI < 250 ms p95; data marked stale after 5 s with no tick in market hours.
- Full-universe scan < 60 s.
- Laptop sleep/wake: catch-up with no candle gaps.
- Cost: nothing to host; only the Kite subscription.

## Deployment and CI
Local Mac only (127.0.0.1). GitHub Actions for PR checks (`just gate`), release-please for CHANGELOG and tags.
Branches: integration `develop`, release `main` (both exist). Feature PRs -> `develop`; `main` moves only via
`just promote`. Team mode: worktrees.

## Constraints and conventions
- `Decimal` for all INR money, never `f64`; all times IST through the `Clock` trait.
- No hardcoded lot/tick sizes, expiries, holidays, ban lists or charge rates; load from the instrument master and `config/*.toml`.
- No look-ahead in backtests; `paper` is the default mode; every order path goes through DecisionGate plus approval.
- Agents never touch real data stores, the live broker or `data/`; tests use fakes, mocks and temp dirs.
- Owner rules in `.agents/owner-rules.md` apply everywhere (no sticky/scroll-jacking UI; each section fits the desktop viewport).
- UI: light premium look (owner chose preview B, `previews/B-light-premium.html`, 2026-11-20); React 19 + Vite.
