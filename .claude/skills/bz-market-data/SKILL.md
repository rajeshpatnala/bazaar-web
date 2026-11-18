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

Vendors stay swappable behind one trait in `crates/data`. Domain types (`Instrument`, `Tick`, `Candle`) live in `crates/core`.

```rust
#[async_trait::async_trait]
pub trait MarketData: Send + Sync {
    async fn instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>, DataError>;
    async fn candles(&self, req: CandleRequest) -> Result<Vec<Candle>, DataError>;   // chunked inside
    async fn quote_snapshot(&self, keys: &[InstrumentKey]) -> Result<Vec<Quote>, DataError>;
    async fn subscribe(&self, keys: &[InstrumentKey], mode: StreamMode)
        -> Result<TickStream, DataError>;                                            // Stream<Item = TickEvent>
    fn freshness(&self) -> Freshness;
}

pub enum TickEvent { Tick(Tick), Status(StreamStatus) }  // Connected, Reconnecting{attempt}, SessionExpired, Stale
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("session expired, login required")] SessionExpired,   // HTTP 403 / TokenException: never retried
    #[error("rate limited")] RateLimited { retry_after: Option<std::time::Duration> },
    #[error("vendor unavailable")] Unavailable,                   // 5xx, network: retryable
    #[error("bad request: {0}")] BadRequest(String),
    #[error("decode failed at {path}")] Decode { path: String },
    #[error("plan does not include this data")] NotEntitled,
}
```

- Key instruments by `InstrumentKey { exchange, tradingsymbol }`. Vendor tokens (Kite `instrument_token`) are a lookup detail resolved through the instrument master, because exchanges can reuse derivative tokens after expiry.
- Adapters: `KiteMarketData` (live + historical), `BhavcopyStore` (EOD only; implements `candles` for `day`), `CsvImport` (fallback). A `CompositeMarketData` routes by capability and plan. Candle requests the plan cannot serve return `NotEntitled`, and the UI offers the CSV or bhavcopy path.
- Request pacing lives in the adapter (a token-bucket per endpoint class, values from `config/data.toml`), not in callers.

### 3a. `config/data.toml` (owned here)

Starts with the standard header (`schema_version`, `verified_on`, `source`); each table below also carries its own `verified_on` and `source` (a docs URL or circular number), because vendor pages change independently. Missing key means the adapter refuses to start that feature (fail closed).

| Table | Keys (all values come from the cited vendor page, never from memory) |
|---|---|
| `[kite.urls]` | `api_base`, `login`, `ws` (no keys or tokens, only templates) |
| `[kite.rate_limits]` | requests per second per endpoint class: `quote`, `historical`, `default`; `max_retries`, `backoff_base_ms`, `backoff_cap_ms` |
| `[kite.limits]` | `ws_max_instruments_per_connection`, `ws_max_connections`, `quote_batch_size`, `ltp_batch_size` |
| `[kite.historical]` | `window_days.<interval>` per interval (not stated on the docs page; confirm in the docs or forum before filling) |
| `[bhavcopy.<exchange>_<segment>]` | `url_template` (placeholders like `{YYYYMMDD}`), `publish_after_ist`, `max_download_bytes`, `series_keep` |
| `[refresh]` | `instruments_after_ist`, `stale_after_secs`, `delayed_after_secs`, `max_reject_ratio`, `price_gap_flag_pct` |

Fixture copy: `fixtures/config/data.toml` with fictional values, loaded by tests with `--config-dir fixtures/config`.

Corporate actions are **not** a config file: they live in the store table `corporate_actions` (section 7) and are loaded with `bazaar-cli import-actions <csv>`.

## 4. Kite Connect essentials (details in `references/kite-connect.md`)

- Daily login: browser login (`GET /api/v1/kite/login`) -> `request_token` on the redirect to `/api/v1/kite/callback` on the server -> signed exchange for an `access_token` that expires the next morning. The server must treat "no valid token" as a normal state: `just db-check` prints `FAILED kite-session <category>` (format in section 8), the UI shows a login prompt, the data layer returns `SessionExpired`. Never loop on 403.
- Instrument master: fetch the gzip CSV once per trading day before the open, store a dated snapshot in SQLite, key by `(exchange, tradingsymbol)`.
- Historical candles: chunk long ranges by the per-interval window in `data.toml`, obey the configured rate, upsert into Parquet.
- WebSocket: binary frames (packet count, then length-prefixed packets), three modes (LTP, quote, full), JSON text messages for control and notices. Caps on instruments per connection and connections per key come from `data.toml`.

## 5. Reconnecting WebSocket client

One task owns the socket; everything else talks to it via channels.

```rust
// state: desired subscriptions are the source of truth, not the socket
struct Desired { modes: HashMap<StreamMode, BTreeSet<Token>> }

loop {
    match connect(&cfg).await {                       // connect timeout from config
        Ok(ws) => { backoff.reset_after(stable_for); run(ws, &mut desired, &tx).await }   // returns why it ended
        Err(e) if e.is_auth() => { tx.send(Status(SessionExpired)); break }              // do not hammer
        Err(_) => {}
    }
    tx.send(Status(Reconnecting { attempt }));
    sleep(backoff.next_with_jitter()).await;          // exponential, capped, full jitter
}
```

- Backoff: exponential with cap and jitter (base/cap in config); reset only after the connection stayed healthy for a minimum time, so a flapping server does not reset it every cycle.
- On every (re)connect, resend the whole desired set, grouped per mode and chunked to the vendor's batch size; never assume the server remembers.
- Liveness: any frame counts, including the 1-byte heartbeat. If nothing arrives for `stale_after` seconds during market hours, close and reconnect; outside hours do not alarm (use the calendar from `bz-core`).
- Backpressure: bounded channel; when the consumer lags, drop to "latest tick per instrument" (conflate) and count drops. Never let the socket reader block.
- Shut down on a cancellation token; no detached tasks.
- Parser output is `Vec<Tick>` with `Decimal` prices; the divisor comes from the instrument's segment (looked up), not guessed from the packet.

## 6. Instrument master

- Single source of truth for lot size, tick size, expiry, strike, segment, F&O eligibility. Tables: `instruments` (current), `instrument_snapshots` (dated, for backtests that need what existed then).
- Derived facts are functions over the master, never constants: "next weekly expiry of NIFTY" = smallest expiry >= today among that underlying's options; holiday shifts are already reflected in the dump. Add a test that changes the fixture expiry weekday and asserts the code follows it.
- Freshness: record `fetched_on` (IST date). `check-connections` reports the age; consumers refuse to build an option chain from a master older than the last trading day.
- Parse `tick_size`, `strike` as `Decimal`; `expiry` as `NaiveDate` (exchange date, no time zone). Empty string means none.
- Diff on refresh: log counts of added, removed, changed lots/ticks (counts only) so surprises show up.

## 7. Corporate actions

Kite gives no adjustment data. Corporate actions live in the store (SQLite table below), never in a config file. Sources, in order: a user-reviewed CSV loaded with `bazaar-cli import-actions <csv>` (idempotent upsert on the unique key, with an `ImportReport`), then official exchange corporate-action files if their terms allow download. Every row carries `source` and `verified_on`; nothing is typed in from memory.

```sql
corporate_actions(id, isin, exchange, symbol, ex_date, kind,        -- split | bonus | dividend | rights | demerger
                  ratio_num, ratio_den, amount_text, source, verified_on,
                  UNIQUE(exchange, symbol, ex_date, kind));
```

- Per-action price factor applied to bars **before** `ex_date`: split `old:new` -> `old/new` of prior price (split 1->5 gives 0.2); bonus `a` new for `b` held -> `b/(a+b)`; cash dividend -> `1 - D / close_before_ex` (ratio method). Volume factors are the inverse for splits and bonuses. Rights, demergers, mergers: manual factor entry only, flagged `needs_review`.
- Cumulative factor at bar date `t` = product of factors for all actions with `ex_date > t`. Build the factor table once per symbol and cache; apply in the read path (`AdjustedCandles` view), never mutate stored raw bars.
- Cross-check: after each import, flag any overnight move beyond a configured threshold with no action row (probable unrecorded split/bonus). Report, do not auto-fix.
- Key by ISIN where available (symbols change); keep a `symbol_history` mapping.
- Backtests must declare adjusted or raw data in their config; `bz-backtest-risk` uses the adjusted view by default.

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

- Fixture paths and tolerances come from the tree owned by `bz-testing` (`fixtures/kite/{rest,ws}/`, `fixtures/bhavcopy/`, `fixtures/config/`); do not invent other folders.
- **Binary parser:** hand-built frames as hex fixtures in `fixtures/kite/ws/` for each packet size (LTP, index, quote, full with depth), multi-packet frames, heartbeat, truncated and oversized lengths. `proptest`: arbitrary bytes never panic; encode-then-parse round trips for a test encoder.
- **Fake WebSocket server:** tokio listener on `127.0.0.1:0` that sends scripted frames, drops the connection, rejects auth, goes silent. Assert resubscription after reconnect, backoff growth (`tokio::time::pause`), `SessionExpired` on auth failure, conflation under a slow consumer.
- **REST:** `wiremock` serving recorded JSON/CSV from `fixtures/kite/rest/`: instruments gzip, historical with and without OI, 403 -> `SessionExpired`, 429 -> pacing, 5xx -> retry then `Unavailable`. Assert requests carry the version and auth headers without printing them.
- **Bhavcopy:** golden files (small real-shaped samples, one per layout, plus malformed, empty, BOM, CRLF); assert accepted and reject counts, idempotent re-import, snapshot (`insta`) of the normalised rows.
- **Corporate actions:** table-driven factor tests (split, bonus, dividend, stacked actions, action on the first bar), reference values computed by hand in comments.
- **Store:** `sqlx::test` ephemeral SQLite and a `tempfile` directory for Parquet; migration up-to-date test; DuckDB round trip with a `Decimal` price.
- **Fixtures** contain no tokens or account data; recording from the real service is a human task outside agent sessions, and agents otherwise craft minimal fixtures by hand.

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
