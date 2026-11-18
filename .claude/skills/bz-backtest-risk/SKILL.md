---
name: bz-backtest-risk
description: Bazaar backtester, Indian cost model, slippage, metrics, walk-forward/OOS, risk-crate position sizing, portfolio limits and DecisionGate implementation. Use when touching crates/backtest or crates/risk, config/charges.toml or risk.toml, backtest stats, sizing, or pre-trade rules.
---

# Backtesting, costs and risk (crates `backtest` and `risk`)

Scope: the event-driven backtester, `CostModel`, slippage, metrics, walk-forward and out-of-sample testing, position sizing, portfolio limits and the `DecisionGate` mechanism. Neighbours: strategy and indicator code is `bz-quant`; what the rules should be (policy, defaults, override rules) is `bz-trading-discipline`; order pipeline, approvals and paper fills are `bz-execution`; market facts (lots, holidays, charges taxonomy) are `bz-india-markets`; bars, corporate-action factors and the instrument master come from `bz-market-data`; test conventions are `bz-testing`; Decimal, IST, config and error rules are `bz-core`.

Open on demand: `references/charges-toml.md` (schema, loader, computation order, tests) and `references/decision-gate.md` (rule catalogue, types, overrides, `risk.toml` keys).

## 1. Stack

| Concern | Choice | Note |
|---|---|---|
| Money, prices, qty maths | `rust_decimal` (1.43 at time of writing), `rust_decimal_macros::dec!` | never `f64` for rupees; ratios in metrics may be `f64` at the end |
| Time | `chrono` + `chrono-tz` `Asia/Kolkata`; clock injected | no `Utc::now()` in these crates |
| Config | `serde` + `toml`, `deny_unknown_fields`, decimals as strings | `config/charges.toml`, `config/risk.toml` |
| Property tests | `proptest` (1.x) | look-ahead, sizing, cost invariants |
| Table tests, snapshots | `rstest`, `insta` | gate verdicts, golden backtest stats |
| Batch data | bars arrive as `Vec<Candle>` (`bazaar_core::Candle`) from the store (Parquet via DuckDB/polars upstream) | the engine never queries storage itself |
| Parallelism | `rayon` across symbols or parameter sets | each run is single-threaded and deterministic |

## 2. Golden rules

1. **No look-ahead by construction.** The strategy receives a `DataView` whose backing slice ends at the current bar. There is no accessor that reaches later data. A `lookahead_` property test proves it (section 10).
2. **Decide at bar close, fill later.** Signal on bar `i` close; the order fills at bar `i+1` open (or a configured later point) plus slippage. Never fill at the close that produced the signal.
3. **Every P&L is net of costs.** Gross P&L may be shown but metrics, R-multiples, equity and the evidence bar use net P&L.
4. **Rates come from `config/charges.toml`**, each with `verified_on` and `source`; nothing numeric about charges in code or tests-of-app-code. Missing rate set is an error, never a default.
5. **Money is `Decimal`.** Prices round to the instrument tick, quantities are integers (units, lots x lot size), lot and tick sizes come from the instrument master as of the trade date.
6. **Risk never rounds up.** Sizing floors to whole lots; if one lot exceeds the risk budget the answer is "no trade", not one lot anyway.
7. **Backtest and live share code.** The same sizing function and the same `DecisionGate` run in backtest, paper and live. Backtest runs without overrides.
8. **Gate fails closed and cannot be bypassed.** Missing input is a failure; the pipeline only accepts a `GatedIntent` that only the gate can mint (execution then mints `ApprovedOrder` from it; see `bz-execution`).
9. **Deterministic.** Same bars + config + code gives byte-identical results: ordered maps, no wall clock, no unseeded randomness, fixed reduce order under rayon.
10. **Pessimistic ties.** Stop and target both inside one bar: the stop fills. Gap through a stop: fill at the open, not the stop price. Limit orders fill only if price trades through the limit.
11. **Thresholds are config.** Limits live in `risk.toml`, not in code. A threshold literal in `crates/risk` is a review blocker.
12. **Results carry provenance**: config hashes, `verified_on` range of charges used, data range, universe definition, parameter-trial count.

## 3. Event-driven loop

In short: Covers: The data-window design; Data adjustments.
Full detail, examples and patterns: `references/event-driven-loop.md` - read it before changing this area.

## 4. Slippage

Adverse always: buys fill higher, sells lower, stops worse than the trigger on gaps. Configure in `strategies.toml` or a `[slippage]` table (bps, Decimal strings) by liquidity bucket:

```rust
pub fn apply_slippage(price: Decimal, side: Side, bps: Decimal, tick: Decimal) -> Decimal {
    let adj = price * bps / dec!(10000);
    let raw = match side { Side::Buy => price + adj, Side::Sell => price - adj };
    round_to_tick(raw, tick, match side { Side::Buy => Rounding::Up, Side::Sell => Rounding::Down })
}
```

Buckets: liquid index and large-cap equity, mid/small caps, index options by moneyness and distance from ATM, illiquid strikes (widest), plus an expiry-day multiplier. Option slippage is better expressed as a fraction of the bid-ask spread when recorded quotes exist, else wider bps. Provide a `slippage_x` stress knob (0, 1x, 2x) and report results at 2x; a strategy that dies at 2x costs is fragile.

## 5. Cost model

`CostModel::charges(&Leg) -> ChargeBreakdown` per executed leg (brokerage, STT/CTT, exchange transaction, SEBI fee, IPFT, stamp duty, GST, DP charge). Loader, schema, computation order (GST on brokerage plus exchange transaction plus SEBI fee, per the broker's stated base), rounding policy, verification routine and tests are in `references/charges-toml.md`. At implementation time the developer fills every rate from zerodha.com/charges and exchange circulars; third-party calculators disagree with each other and with the broker (F&O STT reportedly changed on 1 Apr 2026; verify), so none is a source. The file schema, including the file-level `schema_version`/`verified_on`/`source` header, is only in `references/charges-toml.md`.

## 6. Metrics

Computed from net trades and the mark-to-market equity curve:

| Metric | Definition |
|---|---|
| trades | closed trades (open-at-end reported separately) |
| win rate | net winners / trades |
| profit factor | gross profit / gross loss after costs; `None` (not infinity) when there are no losers |
| R-multiple | net P&L / initial risk in rupees (qty x stop distance at entry); average R and expectancy = mean R |
| max drawdown | peak-to-trough on equity at bar frequency, in rupees and percent, plus duration |
| CAGR | only when the span is at least one year; otherwise show total return and say so |
| monthly returns | calendar-month table in IST |
| exposure, turnover, costs paid | total charges and slippage as percent of gross P&L (a cost-dominated edge is a warning) |

`low_evidence = oos_trades < gate.evidence.min_oos_trades` (value from `risk.toml`; key per the `bz-trading-discipline` table). Stats are always attached to (strategy, params, instrument or universe, data range, `is`/`oos` label). Keep `Decimal` for rupee totals; convert to `f64` only inside ratio statistics.

## 7. Walk-forward and out-of-sample

- Rolling or anchored windows: optimise parameters on `train_months`, evaluate frozen parameters on the next `test_months`, step forward, stitch the OOS segments into one equity curve. Add an embargo gap of at least the longest indicator lookback between train and test.
- Reserve a final **holdout** period that no tuning, grid or review ever touches; evaluate it once and record that it was spent.
- Record the number of parameter combinations tried; many trials on little data is an overfitting warning surfaced in the result.
- Report IS vs OOS side by side with degradation (OOS profit factor and expectancy relative to IS) and per-window stability (share of profitable windows). The evidence bar uses **OOS** trade count and cost-adjusted stats.
- Results: `WalkForwardResult { windows: Vec<WindowResult>, oos_stats, is_stats, holdout: Option<Stats>, trials: u32 }`.

## 8. Position sizing (risk crate)

In short: Risk rupees divided by stop distance, floored to whole lots, with a cost and slippage buffer so a stopped-out trade cannot exceed the limit.
Full detail, examples and patterns: `references/position-sizing.md` - read it before changing this area.

## 9. DecisionGate (summary)

A pure, deterministic pre-trade checklist that evaluates every rule, fails closed, and mints the only type the executor accepts (`GatedIntent`). Rule results are `Pass` or `Fail { severity: Hard | Overridable, reason, observed, limit }`, with severity derived from the Override column of the `bz-trading-discipline` table (Hard ceiling and No = `Hard`; Soft = `Overridable`; Paper = `Overridable` plus a mode check). An override carries a typed `OverrideReason` (code plus a note of at least `override.min_reason_chars`), must cover every failing rule, is impossible against `Hard` rules and in backtest/auto modes, and is returned as a record for `bz-execution` to append to the audit log. Full types, the rule catalogue (evidence bar, per-trade and daily limits, max positions, sector/correlation, ban list, events, cool-off, no stop widening, entry zone, no averaging down, margin, lot/tick, stale quote) and the `risk.toml` keys are in `references/decision-gate.md`. Policy and default thresholds: `bz-trading-discipline`.

## 10. Tests (what and how)

In short: All tests are deterministic, fixture-driven, offline: bars from `fixtures/` (recorded bhavcopy or Kite JSON samples) or generated by proptest strategies; charges from the synthetic test rates; a fake clock passed in. Unit tests ship in the ...
Full detail, examples and patterns: `references/tests.md` - read it before changing this area.

## 11. Red flags (security and correctness)

```
rg -n 'Utc::now|Local::now|SystemTime::now' crates/backtest crates/risk
rg -n '\b(f32|f64)\b' crates/risk crates/backtest/src/cost* crates/backtest/src/portfolio*   # money paths
rg -n 'skip_gate|bypass|GatedIntent \{|ApprovedOrder \{|fn place\(|Executor::' crates   # hits outside risk/execution/broker are bugs
rg -n 'DataView::ending_at' crates --glob '!crates/backtest/**' --glob '!**/tests/**'                # only the engine builds views
rg -n 'bars\[\s*(i|idx|index)\s*\+|\.iter\(\)\.rev\(\)\.skip|shift\(-' crates/backtest crates/strategies
rg -n '(0\.0[0-9]+|[0-9]+\.[0-9]+)\s*\)?\s*(\*|/)' crates/backtest/src/cost* # rate literals in code
rg -n 'unwrap\(\)|expect\(' crates/risk crates/backtest/src --glob '!**/tests/**'
```

Also reject: gate thresholds hard-coded; overrides accepted in backtest or auto mode; costs set to zero "for speed"; charges edited without `verified_on`/`source` updated; indicators precomputed over the full series without a causality test; reading `.env` or secrets in these crates (they need none).

## 12. Checklist

- [ ] Strategy sees only `bazaar_core::DataView`; `lookahead_` proptests pass for the touched strategy and the engine.
- [ ] Fills at next bar open plus adverse slippage; pessimistic tie and gap handling covered by tests.
- [ ] Every P&L figure is net of `ChargeBreakdown`; `charges.toml` header and entries have `verified_on` and `source`; contract-note goldens pass.
- [ ] Sizing floors to lots, never exceeds the risk budget, errors instead of rounding up; margin never estimated.
- [ ] Gate: all rules run, fail closed, `GatedIntent` is the only way to an `ApprovedOrder`, overrides validated and recorded, same code in backtest.
- [ ] Walk-forward with embargo and a spent-once holdout; IS vs OOS and trial count in the result; `low_evidence` flag set from config.
- [ ] Results carry provenance and the survivorship flag; run is deterministic (golden diff reviewed).
- [ ] Unit tests added in this PR; `just gate` green; red-flag greps clean.
