# 2. Crate map and dependency rule

(From `bz-architecture` SKILL.md; open when the task touches this area.)


| Crate | Owns | May depend on |
|---|---|---|
| `core` | domain types, `Money`/Decimal, IST `Clock` + `MarketCalendar`, config types, shared ports in `core::ports` (candles, persistence repos, `MarginSource`, `Notifier`, `SecretStore`) | std, serde, rust_decimal, chrono(-tz), thiserror |
| `indicators`, `options` | pure maths, no I/O | `core` |
| `regime` | trend/range + volatility state | `core`, `indicators` |
| `strategies` | `trait Strategy`, one module each | `core`, `indicators`, `options`, `regime` |
| `data` | `MarketData` trait, Kite read-only client, bhavcopy, instrument master, corporate actions, WS client, bar builder | `core` |
| `store` | SQLite (sqlx) + Parquet/DuckDB repositories; implements the `core::ports` persistence repos (intent, approval, ledger, audit, kill switch, job runs) | `core` |
| `risk` | sizing, limits, `DecisionGate`, behaviour guards | `core` |
| `backtest` | event loop, `CostModel`, slippage, walk-forward | `core`, `strategies`, `risk`, `indicators` |
| `scanner` | run strategies over universe, score, rank, explain | `core`, `strategies`, `regime`, `risk`, `backtest` (stats types) |
| `execution` | pipeline, `ApprovalQueue`, `trait Executor` + order state machine (owned by `bz-execution`), kill switch, rate limiter, audit-chain logic, `ExecutionMode`; persists only through `core::ports` repos | `core`, `risk` (never `store`) |
| `paper` | `PaperExecutor`, `PaperAccount` | `core`, `execution` |
| `broker` | `LiveExecutor` (Kite orders), cargo feature `live-trading`, off by default | `core`, `execution`, `data::kite::session` only |
| `cli`, `server` (`bazaar-cli`, `bazaar-server`) | the only composition roots; `server` (`crates/server`) also owns routes, DTOs, security layer, WebSocket hub, scheduler. The UI lives in `web/`, not in a crate | everything |

Rules, enforced by crate boundaries and checks, not by review alone:

1. Dependencies point toward `core`. Domain crates (everything above `cli`/`server`) never depend on `server`. `indicators`/`options` never see `data`, `store`, tokio or I/O.
2. **No crate below `execution` can place an order.** `data`'s Kite client exposes read endpoints only (instruments, quotes, historical, margins, profile). Order endpoints exist only in `broker`.
3. Only `cli` and `server` pick concrete adapters (SQLite vs in-memory, Paper vs Live, Kite vs fixture feed).
4. `data` and `store` never depend on each other, and `execution` never depends on `store`; they meet through ports in `core::ports` (`CandleSource`, `CandleSink`, the persistence repos) and the composition root wires the `store` implementations in and runs ingestion.
5. `broker` is compiled into the server only behind `live-trading`; default features never enable it. Restrict who may depend on it with cargo-deny `bans` plus `wrappers` (check the current cargo-deny docs for exact syntax) and a CI step that compiles the feature but never runs live calls.
6. Prefer enforcement that fails the build: a test (section 14) that parses `cargo metadata` and asserts the allowed-edges table above.
7. New crate or new edge = ADR (section 11). Resist `utils`/`common` crates; shared things go to `core` only if two or more crates need them.
