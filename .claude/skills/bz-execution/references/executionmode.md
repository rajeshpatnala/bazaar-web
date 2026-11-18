# 4. ExecutionMode

(From `bz-execution` SKILL.md; open when the task touches this area.)


```rust
pub enum ExecutionMode { Paper, SemiAuto, Auto }   // Auto is unconstructible without AutoUnlock
```

| Mode | Approval source | Executor | Notes |
|---|---|---|---|
| `paper` (default) | user click on every order, exactly like `semi_auto`; no auto-accept key or policy exists (bulk simulated runs belong in the backtester) | `PaperExecutor` | UI labels everything "PAPER" |
| `semi_auto` (first live mode) | user click on every order | `LiveExecutor` (feature `live-trading`) | needs the pre-live checklist; **the user approves every order** |
| `auto` | `AutoPolicy` (strategy id + limits from config) | `LiveExecutor` | locked, see below |

`auto` unlock = all of: (1) every item of the pre-live checklist in `bz-security` `references/sebi-algo-compliance.md` is recorded `true` in `config/compliance.toml` and fresh per that file's staleness rule, (2) explicit opt-in row in SQLite written only by the human-only `bazaar-cli live-opt-in --mode auto` (typed `I understand...` phrase in the terminal, timestamp, audited; no HTTP route can write it, CONTRACTS section 5), (3) paper readiness bar from `bz-trading-discipline` met and computed from journal data (`ReadinessReport`, not a boolean in config). `AutoUnlock::try_new(&Checklist, &OptIn, &ReadinessReport) -> Result<AutoUnlock, LockedReason>` is the only way to get `Auto`. If `risk.toml` says `auto` but unlock fails, startup downgrades to `semi_auto` (or `paper` when `live-trading` is not compiled in), logs it and shows a banner. `auto` also requires the `live-trading` feature compiled in. Any change of mode is an audit event and an ADR trigger (`bz-architecture`).
