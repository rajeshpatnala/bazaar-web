# F12: Candle store

Plan: `.agents/plans/F12-candle-store.md`. Status: in progress.

## Slice 0 (ports and contract suite)
- `core::ports`: `CandleRange` (symbol, interval, inclusive close-time window, `adjusted`), `CandleSource` (`read_candles`, `last_candle_ts`), `CandleSink` (`write_candles`).
- Write key is `(symbol, interval, adjusted, close_time)`; a batch mixing interval or adjusted is `RepoError::Conflict` and writes nothing. Inverted or empty range reads as empty.
- `store/tests/candle_contract.rs`: `run_contract(make)` shared suite, proven on an in-memory fake. Slices 1 (SQLite) and 2 (Parquet) call it; copy or `#[path]` include it.
- Dev-dependency `chrono-tz` added to `store` (already in the workspace; tests build IST times).

## Deferred
- Corrupt-value and leftover `*.tmp` cases are adapter-specific; slices 1 and 2.
- Parquet/DuckDB needs owner approval of new dependencies.
- SEC-104-1 (slices 1 and 2): bind the symbol as a SQL parameter (never format it into SQL); allowlist the symbol before it is used in a Parquet file path.
