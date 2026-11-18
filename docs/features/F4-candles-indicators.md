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
- `Candle` fields are private; `close_time` serialises as RFC 3339 and is read back in `Asia/Kolkata`.
- 60-minute buckets align to the session open (odd first bucket possible); no owner preference stated.

## Known limits
- No Ichimoku, 52-week, relative strength, anchored VWAP or delivery %; no TA-Lib cross-check; no persistence.

## Deferred
- TA-Lib/pandas-ta reference CSVs (owner or tester with the tools); F4b indicators.
- Product question for the owner: weekly/monthly bars (not Kite intervals; out of scope).

## History
- Slice 0: candle type, indicator trait scaffold, this record.
