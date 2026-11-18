# 6. Event flow (live data to the browser)

(From `bz-architecture` SKILL.md; open when the task touches this area.)


```
Kite WS ticks -> reconnecting client -> bounded mpsc -> BarBuilder (tick -> 1m/5m/...)
   -> streaming indicators -> strategies.on_bar -> scanner (score/rank) -> server WebSocket hub (coalesced ticks) -> browser
                                  \-> PaperAccount mark-to-market -> position P&L -> WebSocket hub
```

(`regime` sits between indicators and strategies where a strategy needs it.) The flow ends at the server's WebSocket hub, which fans out to each connected tab (message types in `bz-web-api` `references/api-contract.md`). The flow runs whether or not any tab is connected; the hub just has zero subscribers.

- Bounded channels everywhere. Policy per hop: **ticks may be conflated** (keep latest per instrument, pushed at a fixed cadence, for example 4 to 10 Hz); **bar-close and order events are never dropped** (apply backpressure or log and halt the stage).
- Each WebSocket session has its own bounded outbound queue. A slow or dead tab loses conflated ticks, is closed when its queue of must-deliver messages overflows, and never blocks the pipeline or other sessions. A reconnecting tab refetches state over REST, then resumes the stream.
- Strategies run on **bar close** only; never per tick. Intraday risk checks (stop hit, trailing stop) may use ticks, in `paper`/`execution`, not in strategies.
- Fan-out with `tokio::sync::broadcast` for in-process observers (WebSocket hub, journal); each subscriber handles `Lagged` explicitly.
- CPU-heavy work (universe scan, backtest) runs on rayon or `spawn_blocking`, never on the async reactor; it is cancellable (check a token between symbols) and reports progress.
- EOD universe scans read Parquet through DuckDB or polars in batch; the live path is streaming. Indicators have one implementation with two drivers (`bz-quant`).
- Everything is IST at the edge: store timestamps with a clear zone convention (UTC instants, IST for display and session logic via `MarketCalendar`).
