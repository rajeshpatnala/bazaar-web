# 3. Ports and adapters

(From `bz-architecture` SKILL.md; open when the task touches this area.)


Define a port from the consumer's need, in domain words. Port traits shared by several crates live in `core::ports`; a port used by one family lives with its owner (`MarketData` in `data`, `Executor` in `execution`).

| Port | Defined in | Adapters (real / test) |
|---|---|---|
| `Clock` | core | `SystemClock` (IST) / `FakeClock` |
| `MarketData` (subscribe ticks, historical, quotes) | data | Kite / `ReplayFeed` over `fixtures/`, local WS test server |
| `InstrumentMaster` (lot, tick, expiry, segment) | data | Kite CSV dump / fixture file |
| `CandleSource`, `CandleSink` | core::ports | Parquet+DuckDB / in-memory `Vec` |
| Persistence repos: intent, approval, ledger/journal, audit, kill switch, job runs | core::ports (implemented in `store`) | SQLite (sqlx) / in-memory SQLite or fake |
| `Executor` | execution | `PaperExecutor` / `LiveExecutor` / `RecordingExecutor` fake |
| `MarginSource` (the one margin seam; not on `Executor`) | core::ports | Kite margins API / fixed fake; no estimate by hand |
| `Notifier` | core::ports | WebSocket `gate_alert` push, Telegram (optional) / `VecNotifier` |
| `SecretStore` | core::ports | macOS Keychain (`keyring-core`); fallback env vars from an untracked `.env` the bin loads / in-memory map |

- `Executor` (trait, `ExecError`, order state machine, `ApprovedOrder`) is defined once, in `bz-execution` section 6 and section 3; do not copy it into designs, link it. The UI's PAPER/LIVE label comes from its `kind()`.
- `ApprovedOrder` has a constructor reachable only from `execution` after `DecisionGate` + approval; `GatedIntent` is minted only in `risk`. Without that value nobody can call `place`; this is the type-level guard behind "no gate bypass".
- Dyn ports use `async-trait` (native async fn in traits is not dyn compatible); use generics only where profiling proves vtable cost.
- Inject `Clock`; never call `Utc::now()` or `Local::now()` in domain code. The scheduler, session expiry and approval TTLs all read the same `Clock`.
- Stores are chosen in ADRs, not assumed: the default (SQLite for state, Parquet partitioned by symbol/year via DuckDB for candles) is recorded in `DECISIONS.md` once the user confirms it. Store specifics live in `bz-market-data`.
