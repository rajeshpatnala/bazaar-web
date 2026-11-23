# 8. Kill switch, circuit breaker, rate limiter

(From `bz-execution` SKILL.md; open when the task touches this area.)


```rust
pub struct SafetyGuard { kill: KillSwitch, breaker: DailyLossBreaker, limiter: RateLimiter }
pub enum GuardVerdict { Allow, Deny(DenyReason) }
// GuardedExecutor::place -> guard.check(&order, now)? then inner.place
```

- **Kill switch**: global, persisted in SQLite (`kill_switch`: engaged, reason, at, by) with an in-memory `AtomicBool` mirror re-read from the DB before each send (survives restarts and a second process). Engaged on: UI button/hotkey, breaker trip, reconciliation mismatch, `SessionExpired` mid-session, repeated `Unknown` states. Effect: no new orders; open orders are cancelled; only user-approved `reduce_only` exits may pass. Release (`POST /api/v1/killswitch/release { phrase }` (engage: `POST /api/v1/killswitch/engage { reason }`; state: `GET /api/v1/killswitch`)) needs a typed confirmation and is audited. Unknown/unreadable state counts as engaged.
- **Daily-loss circuit breaker**: independent runtime net behind the gate's pre-trade check. Day P&L (realised + unrealised, after charges, IST trading day) from a `PnlSource` port; trips at `-max_daily_loss_pct x capital` from `risk.toml` (default 3%). Evaluated on every fill, every mark update and before every send. Latched: it stays tripped until the next trading day per `MarketCalendar`; blocks new entries, allows `reduce_only` exits, raises a notification, engages nothing else automatically unless configured.
- **Rate limiter**: sliding window over the injected clock, per-second, per-minute and per-day caps from config. Our caps are deliberately far **below** the broker/exchange ones (live values and sources in `references/live-phase.md`), and are validated at load: `own_cap <= broker_cap`, and per-second stays below the SEBI retail OPS threshold. `max_orders_per_minute` is a `[hard_limits]` ceiling (the loader refuses to loosen it). A denied order is rejected visibly (`RateLimited`), never silently queued or retried in a loop. Count attempts that reach the executor, including ones that fail with `Unknown`.
- Cancels and modifies are counted too (brokers cap modifications per order).
