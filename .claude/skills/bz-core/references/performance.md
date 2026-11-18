# Performance

(From `bz-core` SKILL.md.)


- Measure first: `criterion` benches for indicators, the backtest loop and a full-universe scan; keep baselines when changing hot code.
- Indicators are streaming (O(1) update per bar) with a batch driver over the same implementation. No re-computing a window from scratch per bar.
- Scans over the universe use `rayon` on a blocking thread; each strategy evaluation borrows a slice view, no per-bar `Vec` allocation or clone in hot loops; pre-size collections.
- Push filters and aggregates into DuckDB/Polars (`SELECT` only the symbols, date range and columns needed); keep one DuckDB connection per task, reuse prepared statements.
- Tick fan-out to the UI is throttled (coalesce per instrument, e.g. a few updates per second) before it is sent over the WebSocket; a slow client is dropped or skipped, never buffered without bound.
- Cache the instrument master and calendar in memory with a daily refresh; never parse the instrument CSV per request.
- Do not optimise at the cost of correctness: Decimal on money paths is not negotiable; a faster `f64` ledger is a defect.
