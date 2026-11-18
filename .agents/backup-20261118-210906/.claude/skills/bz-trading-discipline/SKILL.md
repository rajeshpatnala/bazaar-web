---
name: bz-trading-discipline
description: Trading discipline policy for Bazaar - rules-not-feelings contract, DecisionGate rule catalogue with risk.toml keys, override policy, paper-to-live readiness bar, automation safety, risk-guardian PR checklist, journaling. Use when designing or reviewing risk, gate, approval, execution-mode or recommendation code.
---

# Trading Discipline (rules, not feelings)

The owner's brief: the app must be strong enough to decide by proven rules, because a trade cannot be taken on a feeling. This skill is the policy source for the `DecisionGate` (in the `risk` crate) and the review lens of `risk-guardian`. Implementation lives in `bz-backtest-risk` (sizing, gate code, property tests) and `bz-execution` (pipeline, approvals, kill switch, audit log). Which strategies earn trust, and the "no trade" philosophy, are in `bz-market-strategy`. Market facts (ban list, expiries, lots, events) come from `bz-india-markets`. Defaults below are the owner's profile (₹1,00,000 capital, 1% per trade, 3% per day) plus conservative starting values; every one is configurable in `config/risk.toml`, none is advice.

## 1. The contract

1. **The gate is deterministic.** Same `OrderIntent` + same portfolio snapshot + same config + same clock = same verdict. No LLM, no network call, no randomness, no wall-clock reads (inject `Clock`), no floating-point money.
2. **Thresholds live only in `config/risk.toml`** (file header `schema_version`, `verified_on`, `source = "owner"`; no `reviewed_on`). The table in section 2 is the only list of keys: other skills, agents and rules point at it. Rust code holds no limit literals; tests may use literals, production code may not.
3. **Fail closed.** Missing data, stale quote, unreadable config, stale ban list, parse error, internal panic = `Reject` with a reason, never `Pass`.
4. **The gate runs twice:** at intent creation and again at approval/send time with fresh prices and portfolio state. A stale "Pass" is never trusted.
5. **Only a passing gate can mint an executable order.** The gate mints a `GatedIntent` (constructor private to the `risk` crate); only `execution` can turn a `GatedIntent` plus an approval (gate re-run with a fresh quote) into an `ApprovedOrder`, and an `Executor` accepts nothing else. Bypass is a compile error, not a code-review hope.
6. **Every verdict is explainable:** a `GateVerdict` lists each rule id, observed value, limit, pass/fail, overridable flag, and the config hash. The UI renders it in plain English, rupees first.
7. **Tightening is instant, loosening is slow.** Lowering a limit applies immediately. Raising one takes effect from the next trading session, needs a typed confirmation, and is logged. This blocks "I'll just widen it for today" at 2 pm after a loss.
8. **Hard ceilings:** every rule whose Override level in the section 2 table is **Hard ceiling** has a `[hard_limits]` entry in `risk.toml`; the config loader rejects any value looser than the ceiling. Moving a ceiling needs a `DECISIONS.md` ADR plus the owner's explicit approval.
9. **"No trade" is an outcome**, not an error. The gate and scanner must be able to return "No high-quality setups today", and that day is logged in the journal too.

## 2. DecisionGate rule catalogue

Override column (the only place override levels are defined): **Hard ceiling** = never overridable, and the loader refuses a value looser than its `[hard_limits]` ceiling; **No** = never overridable (behaviour guard or data/mode rule); **Soft** = one typed override allowed under section 3; **Paper** = soft in paper mode only, rejected in `semi_auto`/`auto` (the gate models it as `Overridable` plus a mode check).

| Id | Check | `risk.toml` key | Default | Override |
|---|---|---|---|---|
| G01 Evidence bar | Strategy on this instrument/segment has enough out-of-sample, cost-adjusted evidence | `gate.evidence.min_oos_trades`, `min_profit_factor`, `min_expectancy_r`, `require_walk_forward_pass` | 30, 1.3, 0.1 R, true | Paper (tagged `exploratory`) |
| G02 Per-trade risk | `qty x (entry - stop)` + est. charges + slippage <= risk budget; lot size rounded **down**; if one lot exceeds budget, reject | `limits.max_risk_per_trade_pct` | 1.0 (₹1,000 on ₹1,00,000) | Hard ceiling |
| G03 Daily loss lockout | Realised + unrealised P&L, net of charges, today <= -limit => no new entries until next session; exits and stop moves toward safety stay allowed | `limits.max_daily_loss_pct`, `limits.on_daily_loss` | 3.0, `lockout_entries` | Hard ceiling |
| G04a Max open positions | Count of open positions after this trade | `limits.max_open_positions` | 3 | Soft |
| G04b Total open risk | Sum of open ₹ risk to current stops + this trade <= cap | `limits.max_total_open_risk_pct` | 3.0 | Hard ceiling |
| G05 Sector and correlation | Sector share of capital at risk; count of positions with 60-day return correlation above threshold | `exposure.max_sector_pct`, `max_correlated_positions`, `correlation_threshold`, `correlation_window_days` | 30, 2, 0.7, 60 | Soft |
| G06 F&O ban list | No new F&O position in a stock on today's ban list; exits allowed. Index F&O not affected. List older than one trading day = reject | `events.block_ban_list`, `events.ban_list_max_age_hours` | true, 24 | No |
| G07 Event days | Results day (and the session before), RBI policy, Budget, own expiry day after cutoff for new option buys/sells | `events.block_results_days`, `block_policy_days`, `expiry_day_no_new_after_ist`, `events_file` | true, true, "13:30", `config/events.toml` | Soft |
| G08 Cool-off | After N consecutive closed losses, no new entries for the cool-off window | `behaviour.cooloff_after_losses`, `behaviour.cooloff_until` | 3, `end_of_session` | No |
| G09 No stop widening | A stop may move only toward profit (trail); any change that increases ₹ risk is rejected | `behaviour.allow_stop_widening` | false (loader refuses `true`) | Hard ceiling |
| G10 Entry zone only | Live price must be inside the recommended entry zone; above it = "missed, re-scan", never chase | `behaviour.entry_zone_tolerance_bps` | 0 | No |
| G11 No averaging down | No add to a position whose price is below average cost (long) or above (short) | `behaviour.allow_averaging_down` | false (loader refuses `true`) | Hard ceiling |
| G12 Liquidity | Avg daily traded value, position as share of ADV, spread, and for options min OI and max bid-ask spread | `liquidity.min_adv_inr`, `max_position_pct_of_adv`, `max_spread_bps`, `min_option_oi`, `max_option_spread_pct` | set by dev-quant from data; never zero | No |
| G13 Naked options | Short options with undefined risk are rejected; defined-risk spreads pass if max loss <= G02 budget | `options.allow_naked_short` | false (enabling = config + ADR + owner opt-in) | No |
| G14 Position size cap | Notional of one position as share of capital | `limits.max_position_value_pct` | 25 | Soft |
| G15 Data freshness | Quote age and bar age within limit, market in an order-accepting session | `data.max_quote_age_secs` | 5 | No |
| G16 Mode, kill switch, order rate | Kill switch off; mode allows this order; `live-trading` compiled in if live; orders per minute under the ceiling | `[mode] execution_mode` (edited in the file only, never by env var or CLI flag), `hard_limits.max_orders_per_minute` | `paper`, set from the broker and SEBI limits | Hard ceiling |

Keys outside the gate rules (same file, same table-only rule): `[hard_limits]` holds one ceiling per Hard ceiling row above plus `max_orders_per_minute`; `[limits] max_orders_per_day`, `max_notional_per_order_inr` and `auto_size_factor` (section 5); `[override]` (section 3); `[readiness]` (section 4); `[execution]` used by `bz-execution`: `approval_ttl_secs` (stale approvals expire), `max_drift_pct` (quote drift since recommendation that forces a re-gate), `paper_starting_cash_inr` (default 100000), `kill_allow_reduce_only` (default true: user-approved exits still allowed while killed); `compliance.toml` staleness (`max_age_days`) is defined in `bz-security`.

Worked example for G02: capital ₹1,00,000, budget ₹1,000, entry ₹500, stop ₹490, estimated round-trip charges + slippage ₹60. Per-share risk is ₹10, so `qty = floor((1000 - 60) / 10) = 94`, never 100. Sizing and the gate both use `rust_decimal`; the gate re-derives risk itself and never trusts a quantity or rupee-risk value passed by the UI.

Order of evaluation: G16, G15, G03, G08, then structural rules, then G01 and exposure rules. Evaluate **all** rules even after the first failure so the user sees every reason at once.

## 3. Override policy

Overrides exist so the owner stays in control, but friction and measurement keep them honest. Only rows marked Soft or Paper in the section 2 table can be overridden. Hard ceiling and No rows can never be overridden by any path (UI, CLI, config flag).

- **Typed reason:** a `OverrideReason` enum (`NewInformation`, `SectorViewDiffers`, `EventRiskAccepted`, `ExploratoryPaper`, `Other`) plus a free-text note (min 30 characters, `Other` requires it). Free-text alone is rejected.
- **Audit entry (append-only, hash-chained, written before the order proceeds):** time, rule ids overridden, observed vs limit, reason, note, config hash, intent id, mode. If the audit write fails, the override fails.
- **Daily cap:** `override.max_per_day = 1`, `override.max_per_week = 3`, enforced in the `risk` crate (not the UI). A tripped cap disables the override button until the next period.
- **Smaller size:** overridden trades are sized at `override.size_factor = 0.5` of the normal risk budget.
- **Never in `auto`:** no human is present to give a reason, so `auto` mode has zero overrides.
- **Cool-down on the request:** the dialog shows the rule, the numbers and the ₹ at risk, and enables Confirm only after `override.confirm_delay_secs` (default 10).
- **Analytics (journal screen):** compare system picks vs overridden trades on win rate, average R, expectancy and max drawdown, by reason code. Show sample size; below 20 overrides label it "too few to judge". If override expectancy is below the system-pick expectancy over 20 or more trades, show a persistent banner: "Your overrides are costing money."
- Everything about the override flow is plain-English and non-shaming; the goal is data, not guilt.

## 4. Paper-to-live readiness bar

Evaluated by code from the ledger, journal and audit log (`bazaar-cli readiness`), never self-declared. The report is stored with its inputs hash. Thresholds sit in `risk.toml` under `[readiness]` and may only be raised in the first release.

| Gate to open | Requirements (defaults) |
|---|---|
| `paper` -> `semi_auto` | 40 or more trading days **and** 50 or more closed paper trades; net expectancy after real charges and slippage > 0 and profit factor >= 1.3; max paper drawdown within 10% of capital; 0 hard-limit breaches; 0 gate bypasses; override rate <= 10% of trades; journal completion >= 95%; paper fills reconcile with the backtest cost/slippage model within tolerance; kill switch and daily-loss lockout drills passed (recorded); `DECISIONS.md` ADR; owner types the opt-in phrase |
| `semi_auto` -> `auto` | 60 or more further trading days of `semi_auto` with 50 or more live trades; all of the above on live results; zero reconciliation mismatches in the last 20 sessions; SEBI retail-algo compliance checklist complete (static IP whitelisted for order APIs - a home connection needs a static IP from the ISP, an owner decision - daily 2FA, Algo ID or broker permission for personal automation, OPS limit under broker's - see `bz-security`, `bz-india-markets`); explicit owner opt-in plus ADR |

Rules: paper must behave like real cash (charges, slippage, partial fills, no fill at circuit), otherwise paper results do not count. Paper wins of fewer than 30 trades per strategy remain "low evidence". `live-trading` is never in `default`; passing the readiness row only allows enabling it at build or run time.

## 5. Automation safety (`auto` and any unattended path)

Owned by `bz-execution`; policy here:

1. Kill switch: global, persisted (survives restart), checked immediately before every send, and flipped automatically on reconciliation mismatch, repeated order rejects, or data staleness.
2. Daily-loss circuit breaker (G03) active before the first order of the day; it also cancels working entry orders.
3. Order-rate limiter below both the broker limit and the SEBI per-segment threshold (`bz-india-markets`); the per-minute cap is `hard_limits.max_orders_per_minute` (Hard ceiling, G16); the per-day and per-order notional caps are the `[limits]` keys named under the table.
4. The auto policy is just another approver: the full gate runs at send time, with fresh data, exactly as for a human click. No separate "fast path".
5. Limit orders only (with a marketable-limit band) for illiquid names; idempotent client order ids; reconcile positions with the broker every few minutes.
6. Every position has a protective stop at the broker or exchange side (SL order or GTT), not only inside the server, so a server crash or restart does not leave exposure unprotected.
7. First `auto` sessions run at `limits.auto_size_factor = 0.5` for the first N sessions; the owner gets a browser notification (and Telegram if enabled; WebSocket `order_update` / `gate_alert` messages feed both) for each order, fill and gate rejection.
8. Daily login/2FA is never automated. When the session expires, `auto` stops and says so.
9. No self-modifying behaviour: auto never edits `risk.toml`, strategy params or overrides, and never widens a stop.

## 6. Journaling discipline

The journal is how rules improve; it is also a gate input (G08 cool-off reads it).

- **Every recommendation gets a record**, including skipped ones and "no trade" days. Skipped-by-gate entries keep the failing rule ids.
- **Pre-trade section is written before approval and is immutable (append-only):** recommendation id, strategy and params, evidence snapshot (trades, PF, expectancy, "low evidence" flag), regime, planned entry/stop/target, ₹ risk, R:R, rule results, config hash, chart-state screenshot. Failure to write it blocks the order (fail closed).
- **Post-trade section:** actual fills, exit reason enum (`target`, `stop`, `trail`, `time`, `manual`, `kill_switch`), R-multiple net of charges, mistake tags (`chased`, `oversized`, `moved_stop`, `ignored_cooloff`, `early_exit`, `none`), optional mood tag (`calm`, `anxious`, `fomo`, `revenge`).
- **Review cadence:** a weekly review screen: planned vs achieved R, rule overrides, mistakes by count and ₹ cost, results by strategy, time-of-day and regime. Plain-English summary first, charts second.
- Journal text never contains tokens, account numbers or secrets (`bz-security`).

## 7. risk-guardian PR checklist

Verdict per PR: **VETO** (blocks merge), **CONCERN** (fix or justify in the PR), **PASS**. Scope: `risk/`, `execution/`, `paper/`, `broker/`, `scanner/`, `config/risk.toml`, and recommendation/approval UI. Run the red-flag scan on changed files, then walk the checklist.

```bash
# bypass flags and auto-approve paths
rg -n -i 'skip_?gate|bypass|force_?(approve|execute|send)|auto_?(approve|accept)|no_?gate|disable_?(gate|risk|killswitch)|unsafe_?skip|override_?all' crates web/src
# hardcoded thresholds in production code (1%, 3%, 1,00,000 capital, lot/qty literals)
rg -n '0\.01\b|0\.03\b|\b100_?000\b|1_00_000|max_(risk|loss|positions)\s*[:=]\s*[0-9]' crates --glob '!**/tests/**' --glob '!**/*test*.rs'
# f64 near rupee amounts
rg -n 'f64|f32|as f64' crates/risk crates/execution crates/paper crates/broker crates/backtest/src/cost* crates/server
# live feature or auto mode on by default
rg -n 'live-trading|default\s*=|ExecutionMode::(Auto|SemiAuto)|execution_mode' Cargo.toml crates/*/Cargo.toml config/risk.toml
# fail-open patterns
rg -n 'unwrap_or\((true|Pass|Verdict::Pass)|unwrap_or_default\(\).*gate|\.ok\(\);.*audit|let _ = .*audit|catch_unwind' crates/risk crates/execution
# executor entry points and gate-minted types (hits outside risk/execution/broker are bugs)
rg -n 'fn place\(|Executor::|ApprovedOrder \{|GatedIntent \{' crates
```

**VETO triggers:**
- Any `skip_gate`, bypass/force flag, env var or CLI switch that sends an order without a passing gate, or an `Executor` method that accepts anything other than `ApprovedOrder`, or a way to build `GatedIntent`/`ApprovedOrder` outside their crates.
- Any path where approval is automatic, timed, defaulted to "yes", or taken from the UI without a user action (outside the future `auto` policy that has its own gate re-run).
- Gate not re-run at approval/send time, or approval accepted after the expiry window.
- `live-trading` in default features, `execution_mode` defaulting to anything but `paper`, or `auto` reachable without the readiness report and opt-in.
- Limits as literals in code; limits looser than the `[hard_limits]` ceiling; defaults in `risk.toml` loosened without an ADR; Hard ceiling or No rows made overridable; `execution_mode` settable from anywhere but the config file.
- Rupee, price or quantity maths in `f64`/`f32`, or `Decimal` converted to float and back inside risk, execution, paper or cost code.
- Stop-loss modification that can increase risk; add-to-loser logic; "re-entry" that ignores cool-off.
- Gate failure modes that pass (missing data, errors swallowed, default `Pass`), or rules short-circuited so the user sees only one reason.
- Quantity, ₹ risk or limits trusted from the frontend instead of recomputed in Rust.
- Kill switch in memory only, not checked before send, or resettable by a strategy; audit entry written after the send, swallowed on error, or editable (non-append-only).
- Override without typed reason, cap enforced only in the UI, or override allowed on a Hard ceiling or No row.
- Tests deleted, `#[ignore]`d or loosened to make a risk change pass; no unit tests in the PR.

**CONCERN triggers:** new rule without a boundary test; new `risk.toml` key without validation, default and doc line; Why-panel text that implies certainty or advice; recommendation card missing evidence, ₹ risk or the "low evidence" label; journal fields dropped; threshold changes without a note on the data supporting them; analytics that hide sample size.

**Report format (<= 12 lines):** verdict, VETO ids with `file:line`, CONCERN ids, the exact rule id(s) at stake, the fix expected, and how it was verified (test name or `just gate`).

## 8. Tests and verification

- **Per rule:** boundary tests just below, at and above the limit (G02: one rupee under/over budget; G03: -2.99%, -3.00%, -3.01%), table-driven, using `rust_decimal_macros::dec!`. Use a fake `Clock` for cool-off, event days, expiry cutoff and next-session lockout release.
- **Properties (`proptest`):** no sequence of fills and quotes lets sized risk exceed the budget; gate is pure (same input, same verdict); stop updates never raise ₹ risk; lockout holds until the clock crosses into the next session; override cap never exceeded.
- **Fail-closed tests:** stale ban list, missing quote, empty events file, corrupt config each produce `Reject`.
- **Config tests:** the committed `risk.toml` parses, validates, matches the documented defaults (`insta` snapshot), and the loader refuses values above `[hard_limits]`, `allow_stop_widening = true` and `allow_averaging_down = true`.
- **Pipeline tests:** with a mock `Executor`, assert `Executor::place` is never called when the gate rejects, and that the gate is called a second time at approval; ephemeral in-memory SQLite for the audit log, kill switch persistence across a simulated restart, hash-chain verification.
- **Readiness tests:** synthetic ledgers on each side of every threshold. No live API, no real broker, no secrets; connection status only via `just db-check`.
- **UI:** override dialog cannot confirm without a reason; gate reasons rendered in plain English with ₹ first (`bz-web-ui`, `bz-testing`).

## 9. Checklist (done means)

- [ ] Every limit read from `config/risk.toml`, validated, with default and doc line
- [ ] Gate pure, fail-closed, evaluates all rules, returns a full `GateVerdict`
- [ ] Executor only accepts `ApprovedOrder` (minted from a `GatedIntent`); gate re-run at approval/send time
- [ ] Hard ceiling and No rows non-overridable, ceilings enforced by the loader, tighten-now/loosen-next-session
- [ ] Override: typed reason, audit written first, daily cap in `risk`, analytics vs system picks
- [ ] Money as `Decimal` end to end; sizing re-derived in Rust
- [ ] Default features exclude `live-trading`; default mode `paper`; readiness report required for the next mode
- [ ] Kill switch persisted and checked before every send; circuit breaker and rate limiter in place
- [ ] Journal pre-trade record mandatory and immutable; weekly review data available
- [ ] Unit tests included in the PR; risk-guardian verdict recorded
