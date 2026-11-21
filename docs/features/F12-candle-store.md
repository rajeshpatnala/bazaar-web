# F12: Candle store

Plan: `.agents/plans/F12-candle-store.md`. Status: in progress.

## Slice 0 (ports and contract suite)
- `core::ports`: `CandleRange` (symbol, interval, inclusive close-time window, `adjusted`), `CandleSource` (`read_candles`, `last_candle_ts`), `CandleSink` (`write_candles`).
- Write key is `(symbol, interval, adjusted, close_time)`; a batch mixing interval or adjusted is `RepoError::Conflict` and writes nothing. Inverted or empty range reads as empty.
- `store/tests/candle_contract.rs`: `run_contract(make)` shared suite, proven on an in-memory fake. Slices 1 (SQLite) and 2 (Parquet) call it; copy or `#[path]` include it.
- `chrono-tz` added to `store` (already in the workspace; tests build IST times); a normal dependency from slice 1.

## Slice 1 (SQLite adapter)
- `store/migrations/0002_candles.sql`, `store/src/candles.rs`: `SqliteStore` implements `CandleSource` + `CandleSink`. Key `(symbol, interval, adjusted, close_ns)`; prices as canonical decimal TEXT, close time as UTC epoch nanoseconds (rendered back in IST); one transaction per write; `last_candle_ts` is `MAX(close_ns)` so it cannot lag a write.
- Corrupt decimal, negative volume or an invalid OHLC row reads as `RepoError::Corrupt`. Volume above `i64::MAX` is refused (`Other`) before anything is written.
- SEC-104-1 (SQLite half) done: symbol is only ever a bound parameter (test with a hostile symbol). The Parquet path allowlist stays with slice 2.
- `chrono-tz` moved from dev-dependency to dependency of `store` (already in the workspace; no new crate).
- `store/tests/candles_sqlite.rs` runs the shared contract suite plus corrupt-row cases.

- Review fixes: symbol (1..=64 bytes), batch (<= 100000) and read (<= 1000000 rows) are capped with new `RepoError::TooLarge`; write count is rows upserted.

## Slice 2 (Parquet writer)
- `store/src/parquet.rs`: `ParquetStore` implements `CandleSink` (write only; reader and `last_candle_ts` are slice 3). Layout `<root>/<symbol>/<interval>_<raw|adj>.parquet`, one file per `(symbol, interval, adjusted)`; prices as decimal text.
- Write merges into the existing file (new row wins), writes `*.parquet.tmp`, fsyncs, renames, fsyncs the directory. Readers open only the exact `.parquet` name, so a leftover `.tmp` is ignored and replaced on the next write. A corrupt existing file or value refuses the write (`Corrupt`) and is left untouched.
- SEC-104-1 done: symbol allowlist `[A-Z0-9_&-]`, max 64 (uppercase only so case-insensitive filesystems cannot merge two symbols), checked before any path; bad symbol is `Other`. Symbol is never in SQL; the root is rejected if it holds a quote or control character.
- The work runs in `spawn_blocking` (`tokio` is now a normal dependency of `store`). One store per root: the write lock is per instance.
- New dependency `duckdb` (owner-approved, bundled + parquet, `~` patch pin).

## Slice 3 (DuckDB read side)
- `store/src/duck.rs`: `DuckStore` implements `CandleSource` over the files `ParquetStore` writes. One in-memory DuckDB connection per read, on `spawn_blocking`; the exact `.parquet` file is opened (no directory glob), so a leftover `*.tmp` is never read. Path and bounds are bound parameters; the symbol uses the writer's uppercase allowlist (unsafe symbol is `Other`). Read capped at 1,000,000 rows (`TooLarge`).
- `last_candle_ts` is `MAX(close_ns)` of the file, so it cannot lag a write (AC 6, CR-111-4). A damaged file or value is `Corrupt`.
- `DuckStore::ping()` for `check-connections` (`OK duckdb`); the CLI line itself is not wired yet (outside this slice).
- `store/tests/candles_parquet.rs`: shared contract suite, SQLite equivalence, leftover tmp, corrupt file and value, hostile symbol.

## Deferred
- Corrupt-value and leftover `*.tmp` cases are adapter-specific; slices 1 and 2.
- Parquet/DuckDB needs owner approval of new dependencies.
- SEC-104-1 (slices 1 and 2): bind the symbol as a SQL parameter (never format it into SQL); allowlist the symbol before it is used in a Parquet file path.
