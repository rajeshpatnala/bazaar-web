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
| G04a Max open positions | Count of open positions after this trade | `limits.max_open_positions` | 3 (owner 2026-11-19) | Soft |
| G04b Total open risk | Sum of open ₹ risk to current stops + this trade <= cap | `limits.max_total_open_risk_pct` | 3.0 | Hard ceiling |
| G05 Sector and correlation | Sector share of capital at risk; count of positions with 60-day return correlation above threshold | `exposure.max_sector_pct`, `max_correlated_positions`, `correlation_threshold`, `correlation_window_days` | 30, 2, 0.7, 60 | Soft |
| G06 F&O ban list | No new F&O position in a stock on today's ban list; exits allowed. Index F&O not affected. List older than one trading day = reject | `events.block_ban_list`, `events.ban_list_max_age_hours` | true, 24 | No |
| G07 Event days | Results day (and the session before), RBI policy, Budget, own expiry day after cutoff for new option buys/sells | `events.block_results_days`, `block_policy_days`, `expiry_day_no_new_after_ist`, `events_file` | true, true, "13:30", `config/events.toml` | Soft |
| G08 Cool-off | After N consecutive closed losses, no new entries until the end of the session | `behaviour.cooloff_after_losses`, `behaviour.cooloff_until` | 2, `end_of_session` (owner 2026-11-19) | No |
| G09 No stop widening | A stop may move only toward profit (trail); any change that increases ₹ risk is rejected | `behaviour.allow_stop_widening` | false (loader refuses `true`) | Hard ceiling |
| G10 Entry zone only | Live price must be inside the recommended entry zone; above it = "missed, re-scan", never chase | `behaviour.entry_zone_tolerance_bps` | 0 | No |
| G11 No averaging down | No add to a position whose price is below average cost (long) or above (short) | `behaviour.allow_averaging_down` | false (loader refuses `true`) | Hard ceiling |
| G12 Liquidity | Avg daily traded value, position as share of ADV, spread, and for options min OI and max bid-ask spread | `liquidity.min_adv_inr`, `max_position_pct_of_adv`, `max_spread_bps`, `min_option_oi`, `max_option_spread_pct` | set by dev-quant from data; never zero | No |
| G13 Naked options | Short options with undefined risk are rejected; defined-risk spreads pass if max loss <= G02 budget | `options.allow_naked_short` | false (enabling = config + ADR + owner opt-in) | No |
| G14 Position size cap | Notional of one position as share of capital | `limits.max_position_value_pct` | 25 | Soft |
| G17 Re-entry lockout | After a losing exit in an instrument, no new entry in the same instrument for N minutes; exits never blocked | `behaviour.reentry_lockout_minutes` | 60 | No |
| G18 Entries per day | New entries per day capped; entries only, exits never blocked | `behaviour.max_new_entries_per_day` | 3 | No |
| G19 Weekly loss lockout | Realised + unrealised P&L net of charges in the ISO week <= -limit => no new entries until next Monday; exits allowed | `limits.max_weekly_loss_pct` (+ `[hard_limits]` ceiling) | 6 | Hard ceiling |
| G15 Data freshness | Quote age and bar age within limit, market in an order-accepting session | `data.max_quote_age_secs` | 5 | No |
| G16 Mode, kill switch, order rate | Kill switch off; mode allows this order; `live-trading` compiled in if live; orders per minute under the ceiling | `[mode] execution_mode` (edited in the file only, never by env var or CLI flag), `hard_limits.max_orders_per_minute` | `paper`, set from the broker and SEBI limits | Hard ceiling |

Keys outside the gate rules (same file, same table-only rule): `[hard_limits]` holds one ceiling per Hard ceiling row above (G02, G03, G04b `max_total_open_risk_pct` 3, G19 `max_weekly_loss_pct` 6) plus `max_orders_per_minute`, `override.max_per_day` (1) and `max_per_week` (3), and a ceiling for each No-row behaviour key so the file cannot loosen it past the owner's value (`cooloff_after_losses`, `max_new_entries_per_day`, `reentry_lockout_minutes`, `entry_zone_tolerance_bps`); changing a ceiling needs an ADR and the owner's approval; `[limits] max_orders_per_day`, `max_notional_per_order_inr` and `auto_size_factor` (section 5); `[override]` (section 3); `[readiness]` (section 4); `[execution]` used by `bz-execution`: `approval_ttl_secs` (stale approvals expire), `max_drift_pct` (quote drift since recommendation that forces a re-gate), `paper_starting_cash_inr` (default 100000), `kill_allow_reduce_only` (default true: user-approved exits still allowed while killed); `compliance.toml` staleness (`max_age_days`) is defined in `bz-security`.

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

In short: Evaluated by code from the ledger, journal and audit log (`bazaar-cli readiness`), never self-declared. The report is stored with its inputs hash. Thresholds sit in `risk.toml` under `[readiness]` and may only be raised in the first ...
Full detail: `references/paper-to-live-readiness-bar.md` - read it before changing or reviewing this area.

## 5. Automation safety (`auto` and any unattended path)

In short: Owned by `bz-execution`; policy here:
Full detail: `references/automation-safety.md` - read it before changing or reviewing this area.

## 6. Journaling discipline

In short: The journal is how rules improve; it is also a gate input (G08 cool-off reads it).
Full detail: `references/journaling-discipline.md` - read it before changing or reviewing this area.

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

In short: **Per rule:** boundary tests just below, at and above the limit (G02: one rupee under/over budget; G03: -2.99%, -3.00%, -3.01%), table-driven, using `rust_decimal_macros::dec!`. Use a fake `Clock` for cool-off, event days, expiry cutoff ...
Full detail: `references/tests-and-verification.md` - read it before changing or reviewing this area.

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
