# 3. Indicators (summary; formulas in the reference)

(From `bz-quant` SKILL.md; open when the task touches this area.)


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
