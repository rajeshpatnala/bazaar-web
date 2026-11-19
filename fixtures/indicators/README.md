# fixtures/indicators

Synthetic, deterministic OHLCV for indicator tests. Not a market fact and not a recording.

- `synthetic_60.csv`: 60 bars, `idx,open,high,low,close,volume`. Close = `100 + 10*sin(i/6) + 0.1*i` (2 dp);
  open = previous close (100.00 for bar 0); high/low pad the body by 0.5 plus a small repeating offset;
  volume = `1000 + (37*i mod 500)`. Every row satisfies `low <= open, close <= high`.
- `session_micro.csv` (`session,open,high,low,close,volume`) and `swing_micro.csv` (`idx,high,low`):
  hand-computed micro cases for session VWAP, gap % and swing pivots; the arithmetic is in the
  tests of `session.rs` and `swing.rs` (loaded with `include_str!`). `session` is an opaque id.
