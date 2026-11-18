# Store crate reference (SQLite via sqlx, Parquet, DuckDB)

Read when building or changing `crates/store`. Repository pattern in two lines: ports (traits such as `InstrumentRepo`, `CandleRepo`, `ActionRepo`, `ImportLog`) are defined in `bazaar_core::ports`, adapters live here, and sqlx/duckdb error types are mapped to a crate error enum before leaving the crate. This file is the Bazaar-specific storage design. Record any change of storage format as an ADR (`bz-architecture`).

## 1. What lives where

| Data | Store | Why |
|---|---|---|
| Instruments, snapshots, corporate actions, import log, session metadata, paper account, journal, audit log | SQLite (`sqlx` 0.9) | small, transactional, relational |
| Candles (all intervals), bhavcopy history, delivery data | Parquet files | large, append-mostly, columnar |
| Analytical reads over candles (backtests, scans, resampling) | DuckDB (in-process) querying the Parquet files | fast scans without loading everything into Rust |

DuckDB is a query engine here, not a system of record. If the Parquet dir is lost it is rebuilt from archives and vendor downloads (the import log tells what to refetch).

## 2. sqlx 0.9 with SQLite

Version: sqlx 0.9.0 was released in May 2026 (<https://github.com/launchbadge/sqlx/discussions/4271>, changelog in the repo). 0.9 changed the `Migrate` trait and SQLite bundling/versioning; check the CHANGELOG before upgrading across versions and avoid hidden/doc-hidden migration helpers.

Cargo features (same set as the workspace table in `bz-repo-structure`; check the current feature list in the sqlx docs): `runtime-tokio`, `sqlite`, `migrate`, `macros`, `chrono`. Pin `duckdb` with `~` in the workspace when first used. Do not rely on a native Decimal mapping for SQLite (see Decimal below).

```rust
let opts = SqliteConnectOptions::new()
    .filename(&cfg.sqlite_path)
    .create_if_missing(true)
    .journal_mode(SqliteJournalMode::Wal)
    .synchronous(SqliteSynchronous::Normal)
    .foreign_keys(true)
    .busy_timeout(Duration::from_secs(5));
let pool = SqlitePoolOptions::new().max_connections(cfg.pool_size).connect_with(opts).await?;
sqlx::migrate!("./migrations").run(&pool).await?;   // embedded, forward-only
```

- Migrations: `crates/store/migrations/<timestamp>_<name>.sql`, forward-only, never edited after merge (checksums are verified). Expand-then-contract for schema changes. Add a test that runs all migrations on an empty in-memory/ephemeral DB and asserts expected tables.
- Use `sqlx::query!`/`query_as!` macros with offline data (`cargo sqlx prepare`, commit `.sqlx/`) or runtime `query_as` with explicit column lists; whichever is chosen, no `SELECT *` and no string-built SQL. CI should check offline data is current.
- **Decimal in SQLite:** SQLite has no exact decimal type. Store prices and money as `TEXT` holding the canonical `Decimal` string, or as integer minor units with a documented scale. Pick TEXT for prices that need up to four decimals and convert in the row mapper (`Decimal::from_str`); never store `REAL` for a price. Sorting or range-querying on TEXT prices is wrong; for those queries use DuckDB or integer minor units.
- Dates: trading days as `TEXT` `YYYY-MM-DD` (IST date), instants as UTC RFC 3339 `TEXT` or integer epoch milliseconds; pick one per column family and document it.
- Single-writer reality: WAL allows concurrent readers with one writer; use small transactions, keep imports in batches (for example 5k rows), and `busy_timeout`. Writes from the import task go through one pool.
- Core tables: `instruments`, `instrument_snapshots`, `corporate_actions`, `symbol_history`, `import_log(file_hash, source, layout, date, status, counts, at)`, `data_state(key, value, at)` for freshness markers (`instruments_fetched_on`, `last_candle_ts`).
- `ping()` = `SELECT 1` with a timeout, printed as `OK sqlite` or `FAILED sqlite <category>`; `safe_reason()` returns `locked`, `missing file`, `corrupt`, `permission`, never a path with secrets.

## 3. Parquet layout

```
data/candles/interval=<day|1m|5m|...>/symbol=<SAFE_SYMBOL>/year=<YYYY>.parquet
data/bhavcopy/segment=<cm|fo>/date=<YYYY-MM-DD>.parquet        (optional raw EOD archive)
data/delivery/year=<YYYY>.parquet
```

- Partition by interval, symbol, year: a backtest reads one symbol across years or a scan reads one day across symbols; small intraday files per symbol-year stay reasonably sized. For daily bars across thousands of symbols prefer `year=` only (one file per year for all symbols, sorted by symbol/date) to avoid thousands of tiny files. Decide per interval from measured file sizes and record the choice in an ADR.
- Symbol directory names come from an allowlist function (`[A-Z0-9_&-]`, percent-encode the rest); never put raw vendor strings in paths.
- Schema (stable, versioned in file metadata `bazaar_schema=1`): `ts` (timestamp UTC, ms), `open/high/low/close` as `Decimal128(18,4)` (scale 4 covers equities at 2 dp and currency at 4 dp; document the convention), `volume` UInt64, `oi` Int64 nullable, `source` string (dictionary), `is_partial` bool. Sorted by `ts`; row group size around 100k rows; zstd compression.
- Writes: a single store-owned writer. Read the existing partition, merge by `ts` (new wins), sort, write to `<file>.tmp`, `fsync`, rename. Readers never see a half-written file. Use the `parquet`/`arrow` crates (60.x, check current) or DuckDB `COPY (...) TO ... (FORMAT parquet)`; either is fine, choose one per dataset.
- Compaction is a separate job, not part of the import transaction.
- Time: store UTC instants; the day bucket is the IST trading date computed with `Asia/Kolkata`, never UTC date (a 09:15 IST bar is 03:45 UTC the same day, but 00:00 to 05:30 IST rolls into the previous UTC date).
- `.gitignore`: `data/`, `*.parquet`.

## 4. DuckDB crate

- Crate `duckdb` (duckdb-rs). Since DuckDB 1.5 the crate version encodes the engine version in its second component (for example 1.10500.x for DuckDB 1.5.0). Docs: <https://duckdb.org/docs/current/clients/rust/overview>, crate: <https://docs.rs/crate/duckdb/latest>. crates.io listed 1.20000.2 as latest stable on 2026-11-18 (docs.rs may lag); confirm on crates.io and pin with a tilde requirement.
- Features: `bundled` (builds DuckDB, slow first build, no system dependency) and `parquet` (reading and writing Parquet; it implies `bundled`). Put the heavy build behind the `store` crate only so other crates compile fast.
- Connection per reader: `Connection::open_in_memory()` (reads files, holds no state), or one shared read-only connection behind a `Mutex` with `try_clone()` per task. Queries are blocking: wrap in `tokio::task::spawn_blocking`.
- Query Parquet by glob with partition columns exposed through Hive-style paths (`read_parquet('data/candles/interval=day/*/*.parquet', hive_partitioning = true)`; see the DuckDB Parquet docs for current option names). Bind values as parameters; the glob/path is built from allowlisted components only.
- Decimal handling: select prices as `CAST(close AS VARCHAR)` and parse to `Decimal` in Rust, or read as `i128`/Arrow decimal arrays through the arrow API; never `f64`.
- Typical queries: range read for one symbol; latest bar per symbol; resample `1m` to `15m` (`time_bucket`/`date_trunc` on IST-converted timestamps, first/last/min/max/sum aggregates ordered by `ts`); join day bars with delivery data; apply adjustment factors by joining the factor table (loaded as a temporary table) and multiplying prices by the cumulative factor for dates before each ex-date.
- `ping()` for check-connections: open an in-memory connection, run `SELECT 1`, and check the data dir exists and is readable. The result prints as `OK duckdb` or `FAILED duckdb <category>` (CONTRACTS section 5), category only.
- Polars 0.55 is also in the stack for in-Rust frame work (`bz-core`); use DuckDB for file scans and Polars for in-memory transforms, not both for the same step.

## 5. Caching

- Hot reads (instrument master, recent candles for the watchlist) cached in memory with `moka` or a simple `RwLock<HashMap>` keyed by `(symbol, interval, range)` with a TTL from config and invalidation when the import task writes that partition (emit a `DataChanged` event on a broadcast channel).
- The instrument master cache is rebuilt on each daily refresh and swapped atomically (`ArcSwap` or `RwLock<Arc<..>>`) so readers never see half a master.
- Never cache across a session boundary without checking `as_of`; cached values expose their age so the stale-data rules apply.

## 6. Tests

- `sqlx::test`-style ephemeral SQLite per test (or `tempfile` path); migration test; repository tests for upsert idempotency, Decimal round trip (`0.05`, `1234.5`, 4-decimal currency), date and time zone handling around IST midnight.
- Parquet: write to a temp dir, read through DuckDB, compare to the input; partial-file rename safety (simulate a crash by leaving a `.tmp`, assert readers ignore it); merge with overlapping `ts` keeps the newer row.
- Resample and adjustment queries against hand-computed expected values.
- Allowlist tests: hostile symbol names (`../x`, quotes, unicode) cannot escape the data dir or inject SQL.
- No test uses the developer's real `data/` directory; use `tempfile::tempdir()`.
