---
name: bz-market-data
description: Bazaar market-data layer - MarketData trait, Kite Connect REST and WebSocket adapter, daily login, NSE/BSE bhavcopy importer, instrument master, corporate-action adjustment, store crate (SQLite, Parquet, DuckDB), stale-data detection. Use when touching crates/data or crates/store.
---

# Bazaar Market Data (dev-data)

Scope: `crates/data` (trait, Kite adapter, WebSocket client, bhavcopy importer, instrument master, corporate actions) and `crates/store` (SQLite + Parquet/DuckDB). Neighbours: money, IST and error rules are in `bz-core`; fixtures, wiremock and fake WebSocket patterns in `bz-testing`; crate boundaries in `bz-architecture`; secrets and legality in `bz-security`; sessions, expiries, lot/tick facts in `bz-india-markets`; order placement is `bz-execution` (nothing here places orders).

Open a reference only for the topic you are changing:

| Topic | Reference |
|---|---|
| Kite login, REST, instruments dump, historical, WebSocket packets, limits | `references/kite-connect.md` |
| NSE/BSE EOD archives, UDiFF, delivery data, import pipeline | `references/bhavcopy.md` |
| sqlx 0.9 SQLite, Parquet layout, DuckDB, caching | `references/storage.md` |

## 1. Default stack

| Concern | Choice | Note |
|---|---|---|
| HTTP | `reqwest` 0.13 (rustls), per-call timeout | base URL injectable so wiremock can stand in |
| WebSocket | `tokio-tungstenite` 0.30 | own reconnect loop in `data::ws`; no auto-retry crate hiding behaviour |
| Parsing | `serde`, `csv`, `zip`/`flate2`, hand-written binary parser | pipeline: bounded read -> parse -> clean -> validate -> upsert -> `CleaningReport` (rule 6 here, steps in `references/bhavcopy.md`) |
| Money | `rust_decimal` `Decimal` | never `f64` for a price; wire paise convert exactly |
| Time | `chrono` + `chrono-tz` `Asia/Kolkata` | store UTC instants or IST-dated trading days, never naive local |
| App state | `sqlx` 0.9 SQLite | instruments, corporate actions, import log, session metadata |
| Bulk data | Parquet via `parquet`/`arrow` 60, read by `duckdb` crate | check crates.io for the current duckdb crate version before pinning |
| Tests | `wiremock` 0.6, local tokio WS server, `proptest`, `insta` | fixtures in `fixtures/` |

## 2. Golden rules

1. **Legal data only.** Official broker APIs, licensed vendors, official exchange archives, or user-supplied CSV. No scraping HTML pages, no browser impersonation, no cookie tricks, no IP rotation. If an official archive refuses a request, stop and surface "import this file manually"; do not work around it.
2. **No hardcoded market facts.** Lot size, tick size, expiry dates, F&O eligibility, ban list, holidays, price bands and WebSocket limits come from the instrument master, `config/*.toml` (with `verified_on`) or the current vendor docs. A literal like `75`, `0.05` or `Thursday` in a data crate is a bug.
3. **Vendor limits are looked up, not remembered.** Rate limits, subscription caps, URL templates, per-interval history windows and refresh windows live only in `config/data.toml` (CONTRACTS section 1; this skill owns the schema, section 3a), each table with `verified_on` and `source`. No such number appears in skill text, code or tests outside the fixture config. When you cannot verify one, write "check the current docs at <url>" and leave the key out so the loader fails closed; never invent a number.
4. **Raw is sacred.** Store unadjusted prices exactly as received; apply corporate-action factors at read time. Re-running an import with the same file must yield identical rows (idempotent upsert on the natural key).
5. **Decimal at the boundary.** Convert wire integers (paise) and CSV strings to `Decimal` in the parser. Compute volumes and OI as `u64`/`i64`. Indicators may use `f64` downstream (`bz-quant`), not this layer.
6. **Every external value is validated.** Reject rows with non-positive prices, `high < low`, close outside `[low, high]`, negative volume, unknown instrument; count and report rejects (`CleaningReport`), abort above a configured reject ratio.
7. **Never panic on bytes from the network.** Binary packet parsing is bounds-checked and returns `Result`; unknown lengths are skipped with a counter, not a crash.
8. **Secrets stay out of the data layer's output.** `api_key`, `api_secret`, `request_token`, `access_token` are held as `secrecy::SecretString` (redacting `Debug`; exposed only at the point of use), are never in log fields, error text, URLs that get logged, or fixtures. Agents never open `.env`; the program loads secrets through the `SecretStore` port (`bz-core`, `bz-security`).
9. **Stale data is a visible state.** Every live stream and every cached dataset exposes `as_of`; consumers get `Freshness::{Live, Delayed, Stale}` and the UI shows a badge (`bz-web-ui`; pushed as the `data_status` WebSocket message, `GET /api/v1/status`).
10. **Tests never touch the live API.** Fixtures + wiremock + local fake WebSocket only. Connection status for agents is `just db-check` and nothing else.

## 3. The MarketData port

In short: Vendors stay swappable behind one trait in `crates/data`. Domain types (`Instrument`, `Tick`, `Candle`) live in `crates/core`.
Full detail, examples and patterns: `references/the-marketdata-port.md` - read it before changing this area.

## 4. Kite Connect essentials (details in `references/kite-connect.md`)

- Daily login: browser login (`GET /api/v1/kite/login`) -> `request_token` on the redirect to `/api/v1/kite/callback` on the server -> signed exchange for an `access_token` that expires the next morning. The server must treat "no valid token" as a normal state: `just db-check` prints `FAILED kite-session <category>` (format in section 8), the UI shows a login prompt, the data layer returns `SessionExpired`. Never loop on 403.
- Instrument master: fetch the gzip CSV once per trading day before the open, store a dated snapshot in SQLite, key by `(exchange, tradingsymbol)`.
- Historical candles: chunk long ranges by the per-interval window in `data.toml`, obey the configured rate, upsert into Parquet.
- WebSocket: binary frames (packet count, then length-prefixed packets), three modes (LTP, quote, full), JSON text messages for control and notices. Caps on instruments per connection and connections per key come from `data.toml`.

## 5. Reconnecting WebSocket client

In short: One task owns the socket; everything else talks to it via channels.
Full detail, examples and patterns: `references/reconnecting-websocket-client.md` - read it before changing this area.

## 6. Instrument master

- Single source of truth for lot size, tick size, expiry, strike, segment, F&O eligibility. Tables: `instruments` (current), `instrument_snapshots` (dated, for backtests that need what existed then).
- Derived facts are functions over the master, never constants: "next weekly expiry of NIFTY" = smallest expiry >= today among that underlying's options; holiday shifts are already reflected in the dump. Add a test that changes the fixture expiry weekday and asserts the code follows it.
- Freshness: record `fetched_on` (IST date). `check-connections` reports the age; consumers refuse to build an option chain from a master older than the last trading day.
- Parse `tick_size`, `strike` as `Decimal`; `expiry` as `NaiveDate` (exchange date, no time zone). Empty string means none.
- Diff on refresh: log counts of added, removed, changed lots/ticks (counts only) so surprises show up.

## 7. Corporate actions

In short: Kite gives no adjustment data. Corporate actions live in the store (SQLite table below), never in a config file. Sources, in order: a user-reviewed CSV loaded with `bazaar-cli import-actions <csv>` (idempotent upsert on the unique key, ...
Full detail, examples and patterns: `references/corporate-actions.md` - read it before changing this area.

## 8. Store crate (details in `references/storage.md`)

- `crates/store` exposes repository traits (`InstrumentRepo`, `CandleRepo`, `ActionRepo`, `ImportLog`) plus SQLite and Parquet/DuckDB adapters; no sqlx or duckdb type leaks past the crate; persistence ports are defined in `bazaar_core::ports` and implemented here (`bz-architecture`).
- SQLite: app state, import log, instruments, corporate actions. Parquet: candles and bhavcopy history, partitioned by interval and symbol/year, written atomically (temp file then rename), single writer task. DuckDB: read-side queries over Parquet, in-memory connection per reader, never the system of record.
- Per-process `DataDir` path from `--data-dir` or config; `ping()` for each store feeds `bazaar-cli check-connections` (contract owned by `bz-repo-structure`, CONTRACTS section 5): one line per check, `OK <check>` or `FAILED <check> <category>`, checks `sqlite`, `duckdb`, `kite-session`, `instruments` (freshness) and `audit-chain`. Categories only (`locked`, `missing`, `login-required`, `stale`, ...), never paths with secrets or token text. Agents run it only as `just db-check` or with `--config-dir fixtures/config --data-dir <tmp>`; `just refresh-instruments` and `just import-bhavcopy` are human-only (they touch live services).

## 9. Stale-data detection

| Signal | Rule |
|---|---|
| Live tick age | `now - last_frame_at > stale_after` during session -> `Stale`; between 1 and N ticks late -> `Delayed` |
| Quote vs exchange time | exchange timestamp far behind wall clock -> `Delayed` even if frames arrive |
| Candle store | last stored bar older than the latest completed session -> backfill needed |
| EOD import | latest imported trading day < latest trading day per calendar -> `import-bhavcopy` hint |
| Instrument master | `fetched_on` < last trading day -> refresh before use |

Thresholds are `data.toml` `[refresh]` values; "last trading day" and "session" come from `MarketCalendar` (`bz-india-markets`). Use an injectable `Clock` so tests control time (`bz-testing`).

## 10. Tests (every PR needs unit tests)

In short: Fixture paths and tolerances come from the tree owned by `bz-testing` (`fixtures/kite/{rest,ws}/`, `fixtures/bhavcopy/`, `fixtures/config/`); do not invent other folders.
Full detail, examples and patterns: `references/tests.md` - read it before changing this area.

## 11. Security red flags (`bz-security` has the full list)

- Any `reqwest`/`tungstenite` URL, header or debug print containing `access_token`, `api_secret`, `request_token`, `Authorization` values.
- `std::fs::read*(".env")` or `dotenvy` calls outside the program entry point; agents or tests reading `.env`.
- Browser-like `User-Agent` spoofing, cookie scraping, proxy rotation, or fetching non-archive HTML pages for data.
- Hardcoded lot sizes, tick sizes, weekdays for expiry, rate limits, subscription caps, plan prices.
- `format!`-built SQL or DuckDB paths from user input (use bound parameters; allowlist partition names).
- `unwrap`/indexing on network bytes; unbounded buffers, unbounded channels, unbounded downloads (cap size).
- `f64` price fields in `Tick`, `Candle`, or stored columns.
- A test that opens a real socket to a vendor host or calls the real login URL.

## 12. Checklist

- [ ] Vendor facts (limits, URLs, formats) come from `config/data.toml` with `verified_on`, or are marked "check the current docs", with source URLs in `DECISIONS.md` for any new vendor.
- [ ] No token or secret reaches logs, errors, fixtures or the UI.
- [ ] Parser rejects are counted and reported; imports are idempotent.
- [ ] Stored data is raw; adjustment is applied at read time and covered by factor tests.
- [ ] Reconnect, resubscribe, auth-expiry and stale paths have tests on the fake server.
- [ ] `just db-check` still prints the five lines (`sqlite`, `duckdb`, `kite-session`, `instruments`, `audit-chain`) in the `OK <check>` / `FAILED <check> <category>` format.
- [ ] Unit tests for the changed code; `just gate` green.
