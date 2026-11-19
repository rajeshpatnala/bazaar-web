# F4 - Candle type, indicator library and bar builder

Status: building
Summary: shared `Candle` type, streaming indicator library and a complete-bars-only bar builder, offline.
Owner area: `crates/indicators`, `core/src/candle.rs`, `crates/data/src/bars.rs`, `fixtures/indicators/`.

<!-- Plan: .agents/plans/F4-candles-indicators.md (slices 0..5). -->

## Acceptance criteria
1. `bazaar_core::Candle` (Decimal OHLC, `Qty` volume, IST `close_time`, `adjusted`) and `Interval`: invalid data returns `DomainError`; serde round-trips exactly.
2. Every indicator yields `None` for exactly its documented warm-up, `Some` after, never NaN/inf; `batch` equals streaming; period 0 rejected with `Result`.
3. Each indicator agrees with an independent naive implementation and 2+ hand-computed cases within 1e-8; proptests for length, determinism, no look-ahead.
4. The Decimal-to-f64 adapter errors on non-finite/out-of-range values and is the only such conversion in the crate.
5. Session VWAP resets at the IST session boundary (from `MarketCalendar`); swing pivots visible only after their confirmation lag.
6. Bar builder aligns buckets to the calendar's session open, never emits a forming bucket, counts and skips gaps and out-of-order bars.
7. Architecture test still passes (indicators -> core only; data -> core only).

## What was built
- Slice 0: `core::candle` (`Candle` with validating `new` and validating `Deserialize`, `Interval`), `DomainError::InvalidCandle`; `bazaar-indicators` with `Indicator` trait, `batch`, `Ohlcv` + fallible `TryFrom<&Candle>`, stub modules; `fixtures/indicators/synthetic_60.csv`.

## Decisions
- F3 is reserved for Kite; offline work starts at F4. One streaming implementation, batch is a fold. Expected values come from hand cases and independent naive implementations (no TA-Lib offline).
- Candle volume is `u64` (not `Qty`/u32) so summed daily volume cannot overflow; `Candle::new` normalises `close_time` to IST; time vs interval sanity is the bar builder's job (slice 5). Every indicator slice must call `testkit::check_contract`.
- `Candle` fields are private; `close_time` serialises as RFC 3339 and is read back in `Asia/Kolkata`.
- Flat-series RSI is 50 (neutral), not TA-Lib's 0 (ChartSchool: undefined), so a flat market never reads as oversold/overbought; pinned by a test and stated in the `momentum` module doc (QUAN-39-3, tech lead decision).
- RSI, ATR and ADX use Wilder smoothing and depend on all history: they need about 200+ bars before the seed effect fades; strategies must set `min_history_bars` accordingly.
- MACD seeds the fast and slow EMAs independently (StockCharts style); TA-Lib seeds both at the slow index, so early values differ slightly (skip the first few multiples of `fast` when comparing).
- Indicators ignore non-finite input (`update` returns `None`, state untouched) and periods are capped at `MAX_PERIOD` = 10_000 (`ParamError::PeriodTooLarge`); ROC needs a positive base.
- Published reference: RSI(14) matches the StockCharts ChartSchool worked example (70.53, 66.32, 66.55, 69.41, 66.36, 57.97) within 0.01.
- 60-minute buckets align to the session open (odd first bucket possible); no owner preference stated.

- Supertrend initial direction: up when close >= hl2 (tie counts as up); other platforms may differ, which only affects bars before the first flip. Pinned by hand cases in `trend.rs`.

## Known limits
- No Ichimoku, 52-week, relative strength, anchored VWAP or delivery %; no TA-Lib cross-check; no persistence.

## Deferred
- QUAN-39-1: published/TA-Lib reference values for EMA and MACD (RSI done); needs a TA-Lib run or a StockCharts sheet that is not available offline.
- TA-Lib/pandas-ta reference CSVs (owner or tester with the tools); F4b indicators.
- CR-36-3: a test that loads `fixtures/indicators/synthetic_60.csv` and builds `Candle`s (validity); add in the first slice that consumes it.
- QUAN-42-1 (PR 42): TA-Lib/pandas-ta reference CSVs for Bollinger, Donchian, Keltner, Stochastic, OBV, volume SMA; assert at 1e-8 from the first `Some`; do not invent values.
- CR-42-3 (PR 42): bands/volume windows are O(N) per bar (MAX_PERIOD 10_000); documented on `Window`, no algorithm change.
- CR-42-4 (PR 42): merge `ParamError`/`TrendParamError` and shared helpers into `util.rs` after PRs 39, 41 and 42 land (also CR-41-1).
- Product question for the owner: weekly/monthly bars (not Kite intervals; out of scope).

- Merge `TrendParamError` and the shared `ParamError` into one util error once PRs 39/41/42 land (CR-41-1).

## History
- Slice 0: candle type, indicator trait scaffold, this record.
