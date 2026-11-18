# 3. Event-driven loop

(From `bz-backtest-risk` SKILL.md; open when the task touches this area.)


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
