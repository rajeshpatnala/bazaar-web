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

Strategies and the engine speak `bazaar_core::Candle` (Decimal OHLC, `close_time`, adjusted flag). Indicators convert to `f64` internally, through one fallible adapter, so no second public bar type exists:

```rust
/// Crate-private f64 working copy of a `Candle`. `TryFrom<&Candle>` is the only Decimal -> f64 conversion
/// in the crate; it rejects values that are not finite instead of propagating NaN.
pub(crate) struct Ohlcv { pub ts_ms: i64, pub o: f64, pub h: f64, pub l: f64, pub c: f64, pub v: f64 }

pub trait Indicator {
    type Input;
    type Output;
    /// Bars needed before the first `Some`.
    fn warmup(&self) -> usize;
    fn update(&mut self, x: Self::Input) -> Option<Self::Output>;
    fn reset(&mut self);
}

pub fn batch<I: Indicator>(mut ind: I, xs: impl IntoIterator<Item = I::Input>) -> Vec<Option<I::Output>> {
    xs.into_iter().map(|x| ind.update(x)).collect()
}
```

Set: SMA, EMA, WMA, RSI (Wilder), MACD, ROC, ATR, ADX with +DI/-DI, Supertrend, Bollinger, Donchian, Keltner, Ichimoku, Stochastic, OBV, volume SMA/spike ratio, session VWAP (resets each IST session) and anchored VWAP, classic and Camarilla pivots, swing high/low with confirmation lag, 52-week high/low, relative strength vs Nifty, gap %, delivery % (EOD input from bhavcopy). Wilder smoothing uses alpha = 1/N seeded with the SMA of the first N values, which is what TA-Lib and StockCharts do; EMA alpha is 2/(N+1). State which seed each indicator uses in its doc comment. Constructor validates params (`period >= 1`) and returns `Result`.

Multi-timeframe: build higher-timeframe bars from lower ones with a bar builder that only emits a bar once it is complete; never feed a partial daily bar to a daily indicator when deciding on an intraday close.

## 4. Options math (summary; formulas in the reference)

- `price(kind, F, K, T, r, sigma) -> f64` (Black-76), `greeks(...) -> Greeks { delta, gamma, vega, theta, rho }`, `implied_vol(kind, price, F, K, T, r) -> Result<f64, IvError>`.
- Forward `F`: use the same-expiry future's price; if absent derive from put-call parity at the strike nearest spot. `T` in years from the valuation instant to the expiry instant, which is `calendar.close_time(expiry_date, segment, kind)` (`bz-india-markets`; never a literal clock time, because session ends change by circular), using minutes, floored at a small epsilon; `r` from config with `verified_on`. Options test fixtures carry the fixture calendar's close time.
- Use **mid** of bid/ask for IV; fall back to LTP only when the book is empty and mark the result `stale`. Compute IV from the OTM side of each strike.
- Solver returns typed errors (`BelowIntrinsic`, `AboveUpperBound`, `ExpiredOrZeroT`, `NoConvergence`); the chain shows "n/a", never a made-up number.
- Chain analytics: IV smile, IV rank and percentile (needs a stored daily ATM-IV or India VIX history; fewer than ~250 observations flags `insufficient_history`). **Structure selection uses IV rank bands** (`bz-market-strategy` section 6); IV percentile is displayed as context only, never as the selection input. Also PCR by OI and by volume, max pain, OI walls, change-in-OI, futures OI build-up, basis and rollover %.
- Payoff engine: `Strategy legs -> Payoff { curve, breakevens, max_profit, max_loss, unbounded flags, pop }` at expiry, exact breakevens from the piecewise-linear structure (not a sampled grid), premiums as `Decimal`. Optional net-of-charges variant calls the `CostModel` from `bz-backtest-risk`.

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

```rust
use bazaar_core::{Candle, DataView};          // DataView design and ownership: bz-backtest-risk section 3.1

pub trait Strategy: Send {
    fn meta(&self) -> &StrategyMeta;          // id, name, horizon, segments, regime_fit, min_history_bars
    /// Called once per completed bar. `view` ends at the closed bar; there is no way to reach later bars.
    /// Use `view.last()` (an Option, no `expect`) for the decision bar and `view.lookback(n)` for history.
    fn on_bar(&mut self, view: &DataView, ctx: &StrategyContext) -> Option<Signal>;
}

/// Same type for the scanner and the backtest engine. "Now" is `view.last().close_time`; no clock field.
pub struct StrategyContext<'a> { pub instrument: &'a Instrument, pub regime: &'a RegimeState,
                                 pub chain: Option<&'a OptionChain>, pub portfolio: &'a PortfolioSnapshot }

pub struct Signal {
    pub strategy_id: &'static str,
    pub side: Side,                           // Buy | Sell (options: legs in `structure`)
    pub entry_low: Decimal, pub entry_high: Decimal,
    pub stop: Decimal, pub targets: Vec<Decimal>,
    pub horizon: Horizon,                     // Intraday | Swing | Positional | LongTerm
    pub structure: Option<OptionStructure>,
    pub facts: Vec<Fact>,                     // evidence for the Why panel: label + value + plain-English meaning
    pub valid_until: DateTime<Tz>,            // entry zone expiry, e.g. `calendar.close_time(..)` for intraday
}

pub trait StrategyFactory { fn id(&self) -> &'static str;
    fn build(&self, params: &toml::Table) -> Result<Box<dyn Strategy>, ParamError>; }
```

Params live in a `serde` struct with `Default`, `validate()` and ranges (also used by the optimiser in `bz-backtest-risk`). Registry maps id to factory. `config/strategies.toml` (this skill owns its schema) starts with the standard header (`schema_version`, `verified_on`, `source = "owner"`), then one `[strategy.<id>]` table per strategy with `enabled = false` by default and a `[strategy.<id>.params]` sub-table whose keys are exactly the params struct fields (unknown key is a load error), plus a `[scanner]` table for score weights, score floor, `min_trades_evidence` and the low-evidence confidence cap. A signal with `stop` on the wrong side of entry or `targets` that fail reward:risk minimum is dropped inside the strategy and logged at `debug`.

## 7. Scanner scoring and Recommendation

Pipeline: regime -> universe filter (liquidity, ban list, event calendar) -> run enabled strategies per instrument (rayon) -> merge signals per instrument/side -> score -> size via `risk` -> portfolio limits via `risk` -> rank -> Why text.

Score 0-100 as a weighted sum, weights in `strategies.toml` (default shape, tune with evidence, not taste):

| Component | Idea |
|---|---|
| Confluence | Distinct strategy *families* agreeing (trend, momentum, breakout, volume ...). Two EMA variants count once |
| Regime fit | Strategy `regime_fit` vs current `RegimeState`; wrong regime gives 0 and usually drops the signal |
| Liquidity | Turnover/volume percentile; for options bid-ask spread % and OI; below the floor drops the signal |
| Backtest quality | Profit factor, expectancy in R, max drawdown, walk-forward stability from `backtest` stats on *this instrument*, shrunk toward the strategy-wide stats when trades are few |
| Event penalty | Results day, expiry day for options, ex-dates, RBI/Budget (from `events.toml`); ban-list names are excluded, not penalised |

**This is the only field list for `Recommendation`.** `RecommendationDto` is derived from it as a utoipa DTO in the OpenAPI schema (`bz-web-api`), and the UI types are generated with `openapi-typescript` into `web/src/api/schema.d.ts` (`bz-web-ui`); no other skill restates the fields, and a snapshot test of the OpenAPI document guards drift.

```rust
pub struct Recommendation {
    pub id: Uuid, pub generated_at: DateTime<Tz>, pub data_as_of: DateTime<Tz>,
    pub instrument: InstrumentRef,
    pub action: Action,                        // Buy | Sell | OptionStructure(..) | NoTrade
    pub entry_zone: (Decimal, Decimal),
    pub stop_loss: Decimal,
    pub targets: Vec<Decimal>,
    pub horizon: Horizon,
    pub size: PositionSize,                    // qty or lots, from risk crate
    pub risk_rupees: Decimal,                  // size x stop distance, incl. estimated charges
    pub reward_risk: Decimal,                  // to first target; also per target in `targets_rr`
    pub agreeing_strategies: Vec<StrategyId>,
    pub confidence: u8,                        // 0-100 score, not a probability
    pub regime: RegimeState,
    pub backtest: BacktestStats,               // trades, win_rate, profit_factor, avg_r, max_drawdown (+ period, costs applied)
    pub low_evidence: bool,                    // backtest.trades < min_trades_evidence (default 30, from config)
    pub why: Vec<WhyFactor>,                   // structured; UI/text renderer produces plain English, risk in rupees first
    pub warnings: Vec<Warning>,                // naked short, ratio spread, event nearby, stale data
    pub gate: GateSummary,                     // derived from the DecisionGate's GateVerdict; blocked ones are shown as blocked, not hidden
}

/// Compact, serialisable view of a `GateVerdict` (defined in `bz-backtest-risk` `references/decision-gate.md`).
/// Built only via `GateSummary::from(&GateVerdict)`; it is not a second verdict type and cannot approve anything.
pub struct GateSummary { pub passed: bool, pub hard_failures: Vec<RuleId>, pub overridable_failures: Vec<RuleId>,
                         pub evaluated_at: DateTime<Tz>, pub config_hash: String }
```

Rules: every field from product Hard rule 6 is mandatory (a constructor `Recommendation::new` returns `Err` on a missing one, there is no `Default`). `low_evidence` also caps `confidence` (cap value in config) and forces the badge. If no candidate clears the score floor and the gate, `scanner` returns `ScanResult { recs: vec![], verdict: NoHighQualitySetups, reasons }`. Why text is generated from `facts`/`WhyFactor` by a pure function so it can be snapshot-tested; wording follows `bz-market-strategy`, with the permanent disclaimer owned by the UI (`bz-web-ui`).

## 8. Tests (required in every PR touching these crates)

- **Unit, reference values:** tolerances come from `bz-testing` (closed-form 1e-8 absolute, iterative solvers such as IV 1e-6; one choice, not per-skill). Each indicator vs a TA-Lib or ChartSchool fixture from the first `Some` (fixtures store full-precision values; a source that seeds differently is compared after the skip window in the indicators reference); Black-76/BS vs machine-generated QuantLib grids at those tolerances; textbook values (Haug, Hull) are published to 2-4 decimals and are checked only to their printed precision; Greeks vs central finite differences of `price`; IV round trip `iv(price(sigma)) == sigma` within the iterative tolerance.
- **Property (`proptest`):** streaming equals batch; RSI/Stochastic within [0,100]; ATR >= 0; Bollinger lower <= mid <= upper; put-call parity; call price monotone in sigma; gamma and vega >= 0; payoff breakevens evaluate to ~0; scanner output sorted and deterministic.
- **No-look-ahead:** test names start with `lookahead_` so `just lookahead` (fails on zero tests) runs them. For each strategy, run on candles `0..=t`, then on candles where everything after `t` is randomised; signals at `<= t` must be identical.
- **Fixtures:** under the tree owned by `bz-testing` (`fixtures/reference/{indicators,options}/`, `fixtures/config/`, ...): recorded candles and option-chain JSON; regime and scanner tests on a small synthetic universe; never live APIs. `FakeClock` for session logic; expiry instants from the fixture calendar's `close_time`.
- **Snapshots (`insta`):** Why text and Recommendation JSON for one buy, one options structure, one low-evidence case, and the no-setups result.
- **Edge cases:** empty/short history, flat prices, zero volume, expired option, IV below intrinsic, empty chain, ban-listed symbol.

Commands: `cargo nextest run -p bazaar-indicators -p bazaar-options -p bazaar-regime -p bazaar-strategies -p bazaar-scanner`, then `just lookahead` and `just gate`. Fixture generation scripts (TA-Lib via Python) are run by a human or in CI tooling once; tests never shell out.

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
