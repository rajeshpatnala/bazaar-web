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

Per symbol (or per portfolio for multi-symbol runs, bars merged in timestamp order):

```
for i in 0..bars.len():
  1. fills:   orders queued at bar i-1 close execute at bar i open (+slippage), subject to circuit/volume checks
  2. exits:   for open positions (including ones just filled), test stop/target against bar i high/low (pessimistic ties)
  3. forced:  intraday (MIS) square-off at the configured time; expiry settlement for derivatives
  4. mark:    equity = cash + positions marked at bar i close, minus accrued charges
  5. decide:  view = DataView over candles[..=i]; strategy.on_bar(&view, &ctx) -> Option<Signal>   (ctx: StrategyContext built from the portfolio snapshot)
  6. size + gate: size_position(...) then DecisionGate.evaluate(...); passing intents queue for bar i+1
end: unfilled orders are dropped; open positions close at last close, flagged `open_at_end`
```

### 3.1 The data-window design

`DataView` is defined once, in `bazaar_core` (this skill owns the design; `bz-quant`'s `Strategy::on_bar(&mut self, &DataView, &StrategyContext) -> Option<Signal>` and the engine both use it).

```rust
/// Read-only window over history that ends at the decision bar. There is deliberately no
/// way to see a later candle: the slice itself stops here.
pub struct DataView<'a> {
    candles: &'a [Candle],
}

impl<'a> DataView<'a> {
    /// Engine-only constructor (strategy crates never call it; see the scan in section 11).
    pub fn ending_at(all: &'a [Candle], i: usize) -> Option<Self> {
        all.get(..=i).map(|candles| Self { candles })
    }
    pub fn last(&self) -> Option<&Candle> { self.candles.last() }
    pub fn len(&self) -> usize { self.candles.len() }
    pub fn is_empty(&self) -> bool { self.candles.is_empty() }
    /// Last `n` candles, oldest first (fewer while warming up).
    pub fn lookback(&self, n: usize) -> &[Candle] {
        &self.candles[self.candles.len().saturating_sub(n)..]
    }
    /// Completed higher-timeframe candles only; one still forming at `last()` is excluded.
    pub fn htf(&self, tf: Timeframe) -> Vec<Candle> {
        match self.last() {
            Some(now) => resample_completed(self.candles, tf, now.close_time),
            None => Vec::new(),
        }
    }
}
```

Rules that keep it honest:

- `Candle` carries `close_time` (IST). `last()` is the decision bar's close; anything stamped later does not exist yet. No `.expect`/`.unwrap` in production code (workspace lints).
- The engine owns the full series and the loop; strategies only ever hold a `&DataView`. No `Arc<Vec<Candle>>` handed to strategies, no accessor returning the backing store, no index arithmetic exposed.
- Higher timeframe, news, results dates and corporate-action facts are time-stamped by **when they became known**, and the view filters on that.
- Indicators are fed bar by bar by the engine (streaming drivers from `bz-quant`); batch precomputation over the full series is allowed only for values proven causal by the same property test.
- Fundamentals and universe membership are point-in-time. Current index constituents applied to past dates is survivorship bias: mark such results "survivorship-biased" in `BacktestResult` and show it.

### 3.2 Data adjustments

- Signals and indicators use corporate-action-adjusted prices (factor table from `bz-market-data`). Costs, tick rounding and lot checks use the raw traded price/lot size as of that date; keep both series available (the `Candle` adjusted flag says which one a slice is) and test that a split does not create a phantom gap signal.
- Lot sizes, tick sizes and expiry dates are looked up per date from the instrument master snapshot; never constants.
- Circuit-locked bars (no two-way trade at the limit) produce no fill; wide-range illiquid bars get a participation cap (`max_participation_pct` of bar volume) with partial fill or rejection.

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

Risk rupees divided by stop distance, floored to whole lots, with a cost and slippage buffer so a stopped-out trade cannot exceed the limit.

```rust
pub struct SizingInput {
    pub capital: Decimal,
    pub risk_pct: Decimal,            // e.g. 1 means 1 percent
    pub side: Side,
    pub entry: Decimal,
    pub stop: Decimal,
    pub lot_size: u32,                // 1 for cash equity, from the instrument master otherwise
    pub buffer_per_unit: Decimal,     // slippage + expected per-unit charges at the stop, >= 0
}

pub struct Sized { pub qty: u64, pub lots: u64, pub risk_rupees: Decimal, pub risk_budget: Decimal }

pub fn size_position(i: &SizingInput) -> Result<Sized, SizingError> {
    if i.lot_size == 0 || i.capital <= Decimal::ZERO || i.buffer_per_unit < Decimal::ZERO {
        return Err(SizingError::InvalidInput);
    }
    let stop_ok = match i.side { Side::Buy => i.stop < i.entry, Side::Sell => i.stop > i.entry };
    if !stop_ok { return Err(SizingError::StopOnWrongSide); }

    let budget = i.capital * i.risk_pct / dec!(100);
    let per_unit = (i.entry - i.stop).abs() + i.buffer_per_unit;
    let lot = Decimal::from(i.lot_size);
    let mut lots = (budget / (per_unit * lot)).floor().to_u64().ok_or(SizingError::Overflow)?;
    // Decimal division rounds at 28 digits; re-check so rounding can never push us over budget.
    while lots > 0 && Decimal::from(lots) * lot * per_unit > budget { lots -= 1; }
    if lots == 0 { return Err(SizingError::BudgetBelowOneLot { budget, one_lot_risk: per_unit * lot }); }
    let qty = lots * u64::from(i.lot_size);
    Ok(Sized { qty, lots, risk_rupees: Decimal::from(qty) * per_unit, risk_budget: budget })
}
```

- Options: a bought option's risk is premium x qty (defined); size from the premium stop or the full premium, whichever the strategy spec says. Spreads and condors: risk is the structure's maximum loss from the payoff engine (`bz-quant`), not a stop distance. Naked short options are never sized automatically; they need the margin and worst-case warning path in `bz-trading-discipline`.
- Cash and margin cap: after sizing, cap by available cash (equity) or by margin from the broker margin API (F&O). If margin cannot be fetched, F&O sizing returns an error; never estimate margin by hand.
- `BudgetBelowOneLot` surfaces to the user as "this setup needs more capital than your risk limit allows" with the rupee figures.
- Portfolio limits (max open positions, total open risk, sector concentration, correlation, daily loss) are gate rules; sizing only enforces the per-trade budget.

## 9. DecisionGate (summary)

A pure, deterministic pre-trade checklist that evaluates every rule, fails closed, and mints the only type the executor accepts (`GatedIntent`). Rule results are `Pass` or `Fail { severity: Hard | Overridable, reason, observed, limit }`, with severity derived from the Override column of the `bz-trading-discipline` table (Hard ceiling and No = `Hard`; Soft = `Overridable`; Paper = `Overridable` plus a mode check). An override carries a typed `OverrideReason` (code plus a note of at least `override.min_reason_chars`), must cover every failing rule, is impossible against `Hard` rules and in backtest/auto modes, and is returned as a record for `bz-execution` to append to the audit log. Full types, the rule catalogue (evidence bar, per-trade and daily limits, max positions, sector/correlation, ban list, events, cool-off, no stop widening, entry zone, no averaging down, margin, lot/tick, stale quote) and the `risk.toml` keys are in `references/decision-gate.md`. Policy and default thresholds: `bz-trading-discipline`.

## 10. Tests (what and how)

All tests are deterministic, fixture-driven, offline: bars from `fixtures/` (recorded bhavcopy or Kite JSON samples) or generated by proptest strategies; charges from the synthetic test rates; a fake clock passed in. Unit tests ship in the same PR as the code (`bz-testing`).

The look-ahead proof: two series share a prefix and differ afterwards; every decision and fill that happens inside the prefix must be identical.

```rust
fn candles_from_steps(steps: &[Step]) -> Vec<Candle> {
    let mut close = 10_000i64;                                    // paise
    steps.iter().enumerate().map(|(i, s)| {
        let open = close;
        close = (open + s.delta).max(100);
        Candle::for_test(                                          // `test-util` feature of bazaar-core
            i, paise(open), paise(open.max(close) + s.up), paise((open.min(close) - s.dn).max(1)), paise(close), s.vol)
    }).collect()
}

fn arb_step() -> impl Strategy<Value = Step> {
    (-300i64..=300, 0i64..=100, 0i64..=100, 1u64..=1_000_000)
        .prop_map(|(delta, up, dn, vol)| Step { delta, up, dn, vol })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn lookahead_future_bars_never_change_past_decisions(
        prefix in prop::collection::vec(arb_step(), 40..80),
        tail_a in prop::collection::vec(arb_step(), 1..40),
        tail_b in prop::collection::vec(arb_step(), 1..40),
    ) {
        let n = prefix.len();
        let a = run(&cfg(), || BreakoutStrategy::default(), &candles_from_steps(&[prefix.clone(), tail_a].concat()));
        let b = run(&cfg(), || BreakoutStrategy::default(), &candles_from_steps(&[prefix, tail_b].concat()));
        prop_assert_eq!(&a.decisions[..n], &b.decisions[..n]);        // signal, size and gate verdict per bar
        let inside = |r: &BacktestResult| r.fills.iter().filter(|f| f.bar_index < n).cloned().collect::<Vec<_>>();
        prop_assert_eq!(inside(&a), inside(&b));
    }
}
```

Use a strategy that actually reacts to the window (a Donchian breakout or EMA cross plus a stop), and add the same test per strategy via a shared helper in `bz-quant`'s test support. Every look-ahead test name starts with `lookahead_` so `just lookahead` (nextest filter `test(/lookahead_/)`, `--no-tests=fail`) finds it and cannot pass vacuously. Commit `proptest-regressions/`; if the installed proptest exposes a fixed RNG seed option, set it in CI so runs are reproducible.

Other required tests:

- **Sizing** (proptest): `risk_rupees <= risk_budget`; `qty % lot_size == 0`; more capital never lowers qty; a wider stop never raises qty; one-lot-too-big returns `BudgetBelowOneLot`; stop on the wrong side errors.
- **Cost model**: see `references/charges-toml.md` (synthetic arithmetic, contract-note goldens, loader errors).
- **Engine**: entry fills at next open, never at signal close; stop and target in one bar resolves to stop; gap through stop fills at open; MIS square-off time; circuit bar gives no fill; open-at-end handling; zero-trade series yields empty stats without panic.
- **Metrics**: hand-computed small trade lists (win rate, PF with no losers is `None`, R, drawdown, monthly table in IST).
- **Walk-forward**: window boundaries and embargo on a tiny fixture; holdout untouched by tuning (the tuner receives no holdout bars).
- **Gate**: per `references/decision-gate.md`.
- **Golden backtest**: a committed fixture run per strategy whose stats JSON is diffed (`insta`); an intentional change is reviewed, not auto-accepted. Time-box with `criterion` benchmarks for the loop on a year of 1-minute bars.

Commands: `cargo nextest run -p bazaar-backtest -p bazaar-risk`, then `just gate`. Connection checks only via `just db-check`; backtests read fixtures or stored data through the store port, never a live API.

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
