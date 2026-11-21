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

## Deferred
- Corrupt-value and leftover `*.tmp` cases are adapter-specific; slices 1 and 2.
- Parquet/DuckDB needs owner approval of new dependencies.
- SEC-104-1 (slices 1 and 2): bind the symbol as a SQL parameter (never format it into SQL); allowlist the symbol before it is used in a Parquet file path.
