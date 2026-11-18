---
name: bz-quant
description: Bazaar quant layer in Rust - streaming+batch indicators, Black-76/BS options math, IV solver, Greeks, IV rank, PCR, max pain, OI build-up, payoff and PoP, regime detection, Strategy trait, scanner scoring and the Recommendation struct. Use when touching crates indicators, options, regime, strategies or scanner.
---

# Bazaar quant (indicators, options, regime, strategies, scanner)

Scope: crates `indicators`, `options`, `regime`, `strategies`, `scanner`. Stack, Decimal/IST rules and error handling are in `bz-core`; costs, backtest metrics, sizing and DecisionGate in `bz-backtest-risk`; test pyramid and fixture rules in `bz-testing`; trader judgement (which strategy fits which regime, Why wording, evidence bar) in `bz-market-strategy`; market facts (expiries, lots, sessions) in `bz-india-markets`. Depth: `references/indicators.md`, `references/options-math.md`, `references/strategy-catalog.md`. Open the one you need, not all three.

## 1. Defaults

| Concern | Choice | Note |
|---|---|---|
| Indicator math | `f64`, own implementation | Outputs are ratios/levels, not money. No TA crate unless audited against TA-Lib values |
| Money boundary | `rust_decimal::Decimal` | Entry/stop/target/premium become `Decimal` once, rounded to the instrument tick, inside `strategies` (adapter fn) |
| Options model | Black-76 on the same-expiry future (index and stock) | Black-Scholes with explicit dividend yield only when no future exists |
| IV solver | Newton from a seed, bisection safety net | See options-math reference |
| Parallelism | `rayon` over instruments in `scanner`; one strategy instance per instrument | Strategies hold state, so never share one across instruments |
| Reference values | TA-Lib / StockCharts ChartSchool spreadsheets, Haug and Hull examples, QuantLib grids | Committed under `fixtures/reference/indicators/` and `fixtures/reference/options/` (tree and tolerances owned by `bz-testing`) with source and tool version in a sibling `README` |
| Benchmarks | `criterion` for indicator hot loops and a full-universe scan | Regression gate lives in `bz-testing` |

Dependencies: `indicators` and `options` depend only on `core`. `regime` depends on `indicators`. `strategies` -> `indicators`, `options`, `regime`. `scanner` -> `strategies`, `risk`, backtest stats types. Nothing here can place an order.

## 2. Golden rules

1. **One implementation, two drivers.** Every indicator is a streaming struct (`update`); the batch function is a fold over it. Never write a second vectorised copy.
2. **No NaN in public output.** Warm-up yields `None`; a value is `Some` only when mathematically defined. Zero denominators (flat range, zero volume) return a documented value or `None`, never `NaN`/`inf`.
3. **No look-ahead by construction.** `on_bar` receives a view that ends at the just-closed bar. No index arithmetic that can reach forward; no centred windows; no pivot detection without the confirmation lag being modelled (a swing high is known only N bars later).
4. **No hardcoded market facts or magic numbers.** Periods, thresholds, weights, min-trade counts and the risk-free rate come from `strategies.toml` / `risk.toml` with defaults in a params struct. Lot, tick, expiry come from the instrument master (`bz-india-markets`).
5. **f64 for math, Decimal for rupees.** Convert at one adapter; round stop away from entry and targets toward entry to the tick size.
6. **Reference-value tests for every indicator and every pricing function**, plus property tests (section 8). A formula without a cited reference value is not done.
7. **Every strategy documents** idea, entry, exit, stop, best regime and known weaknesses in its doc comment, and has a row in `references/strategy-catalog.md`.
8. **A signal is not a recommendation.** Strategies emit `Signal`; only `scanner` builds a `Recommendation`, after regime, scoring, sizing (`risk`) and limits. "No high-quality setups today" is a first-class result.
9. **Never claim certainty.** Scores are ranking aids, not probabilities. PoP is a model estimate and is labelled so.
10. **Defined-risk by default** for options; naked short legs carry a warning flag, worst-case loss and margin from the broker margin API (never hand-estimated).
11. **Deterministic.** Same inputs give same ranking (stable sort with tie-break on symbol); no wall-clock reads inside quant crates, time arrives via the bar or an injected `Clock`.

## 3. Indicators (summary; formulas in the reference)

In short: Strategies and the engine speak `bazaar_core::Candle` (Decimal OHLC, `close_time`, adjusted flag). Indicators convert to `f64` internally, through one fallible adapter, so no second public bar type exists:
Full detail, examples and patterns: `references/indicators-detail.md` - read it before changing this area.

## 4. Options math (summary; formulas in the reference)

In short: `price(kind, F, K, T, r, sigma) -> f64` (Black-76), `greeks(...) -> Greeks { delta, gamma, vega, theta, rho }`, `implied_vol(kind, price, F, K, T, r) -> Result<f64, IvError>`.
Full detail, examples and patterns: `references/options-math-detail.md` - read it before changing this area.

## 5. Regime (`regime` crate)

```rust
pub enum Trend { Up, Down, Range }
pub enum VolState { Low, Normal, High }
pub struct RegimeState {
    pub trend: Trend, pub vol: VolState,
    pub breadth_pct_above_50dma: Option<f64>,
    pub india_vix: Option<f64>, pub vix_percentile: Option<f64>,
    pub as_of: i64, pub reasons: Vec<String>,   // plain-English facts for the Why panel
}
```

Inputs: Nifty 50 (and the instrument's own sector/index when available) trend via price vs 50/200 EMA plus ADX threshold, breadth (advance/decline, share of universe above its 50-day mean), India VIX level and its percentile over the stored history, ATR% percentile for the instrument. Apply hysteresis (a flip needs N consecutive confirming closes, N from config) so the badge does not flap. Missing input degrades to `None` plus a reason, never a guess. Regime changes only on completed bars.

## 6. Strategy trait and signal

In short: use bazaar_core::{Candle, DataView};          // DataView design and ownership: bz-backtest-risk section 3.1
Full detail, examples and patterns: `references/strategy-trait-and-signal.md` - read it before changing this area.

## 7. Scanner scoring and Recommendation

In short: Pipeline: regime -> universe filter (liquidity, ban list, event calendar) -> run enabled strategies per instrument (rayon) -> merge signals per instrument/side -> score -> size via `risk` -> portfolio limits via `risk` -> rank -> Why text.
Full detail, examples and patterns: `references/scanner-scoring-and-recommendation.md` - read it before changing this area.

## 8. Tests (required in every PR touching these crates)

In short: **Unit, reference values:** tolerances come from `bz-testing` (closed-form 1e-8 absolute, iterative solvers such as IV 1e-6; one choice, not per-skill). Each indicator vs a TA-Lib or ChartSchool fixture from the first `Some` (fixtures ...
Full detail, examples and patterns: `references/tests.md` - read it before changing this area.

## 9. Red flags (quant-reviewer greps for these)

- `f64`/`f32` in a signature carrying rupees outside the adapter; `as f64` on `Decimal` outside it.
- `partial_cmp(..).unwrap()`, `.unwrap()` on indicator output, `/ x` without a zero check.
- Anything reading `bars[i + n]`, `.shift(-`, `iter().rev()` over the future, centred windows, swing points used before their confirmation lag.
- Hardcoded periods/thresholds/weights, lot sizes, expiry weekdays, `0.06`-style risk-free rate literals.
- IV from last-traded price of an illiquid strike with no `stale` flag; IV solver without bracket; Newton without a vega floor.
- A `Recommendation` built by struct literal elsewhere; missing `low_evidence`; wording like "will", "sure shot", "guaranteed"; confidence shown as a probability.
- Naked option selling without warning, margin and worst-case loss; ratio spreads without the high-risk flag.
- A strategy file without the idea/entry/exit/stop/regime/weaknesses doc comment.

## 10. Done checklist

- [ ] One streaming impl per indicator, batch is a fold, warm-up documented, no NaN out.
- [ ] Reference-value fixture and source recorded for each new indicator/pricing function.
- [ ] Params struct with defaults and validation; values wired to `strategies.toml`.
- [ ] Strategy doc comment plus catalog row; no-look-ahead test passes.
- [ ] Recommendation carries every Hard-rule-6 field; low-evidence rule tested; "no setups" path tested.
- [ ] Money converted once and tick-rounded; no hardcoded market facts.
- [ ] `just gate` green; quant-reviewer sign-off; `bz-backtest-risk` stats still consumed unchanged.
