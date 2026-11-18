# Indicators: formulas, warm-up, reference values

Open this when adding or changing an indicator or its test. Notation: N = period, C/H/L/O/V = close/high/low/open/volume, `t` = current bar, "prev" = bar t-1. Output is `None` until the warm-up bar count is reached. Formulas are written for the streaming update; the batch API is a fold (see SKILL.md).

## 1. Warm-up and smoothing conventions

| Indicator | First `Some` at 0-based bar index | Seed |
|---|---|---|
| SMA(N), Bollinger(N), Donchian(N), highest/lowest | N-1 | first N values |
| EMA(N) | N-1 | SMA of first N closes (TA-Lib, StockCharts). Some platforms start from bar 1 with the first close; they converge to the same values after a few multiples of N |
| WMA(N) | N-1 | linear weights 1..N, newest heaviest |
| RSI(N), ATR(N) | N | SMA of first N changes / true ranges (TR needs a previous close, hence N not N-1) |
| ADX(N), +DI/-DI | +DI/-DI at N, ADX at 2N-1 | sum of first N DM/TR, then ADX = average of first N DX |
| MACD(f,s,sig) | s-1+sig-1 (33 for 12/26/9) | EMAs SMA-seeded, signal seeded from MACD values |
| ROC(N) | N | none |
| Stochastic(N, smoothK, smoothD) | N-1 + smoothing lags | none |
| Supertrend(N, m) | N (first ATR) | ATR seed, initial direction from first close vs band |
| Ichimoku (9/26/52) | Tenkan 8, Kijun 25, Span B 51; cloud for bar t is the span computed at t-26 | none |

- **Wilder smoothing** is an exponential average with alpha = 1/N: `new = prev + (x - prev)/N`, equivalent to `(prev*(N-1) + x)/N`. Ordinary EMA uses alpha = 2/(N+1). Wilder(N) is the same recursion as EMA(2N-1) but seeded differently; do not mix them.
- RSI, ATR, ADX and their derivatives (Supertrend, Chandelier) depend on all history through the recursion. Values from two sources agree only after the seed influence decays (the weight of the seed shrinks by (1 - 1/N) per bar). When comparing against a source that seeds differently, skip roughly the first 10N bars; when the seed is identical (TA-Lib, ChartSchool) compare from the first `Some`.
- ChartSchool says ADX needs on the order of 150 bars and RSI around 250 to match its published numbers; give strategies `min_history_bars` accordingly (200+ for anything using ADX/RSI/ATR levels).

## 2. Formulas (own-words summary)

- **True range:** max of (H-L, |H - prev C|, |L - prev C|). **ATR:** Wilder average of TR. **ATR%:** ATR / C, used for volatility regime and cross-instrument stop sizing.
- **RSI:** average gain and average loss (Wilder) of close-to-close changes; `RSI = 100 - 100/(1 + avgGain/avgLoss)`. Define the degenerate cases explicitly: avgLoss = 0 and avgGain > 0 gives 100; both 0 (flat) gives 50. Libraries differ on the flat case, so keep flat windows out of the TA-Lib comparison fixture and test the convention separately. RSI(2) (mean-reversion) is the same code with N = 2.
- **MACD:** `EMA(fast) - EMA(slow)`; signal = EMA of MACD; histogram = MACD - signal. TA-Lib starts both EMAs at the same bar so its first MACD equals ours from bar s-1.
- **ROC:** `100 * (C - C[t-N]) / C[t-N]`; guard a zero denominator.
- **ADX/DMI:** up = H - prev H, down = prev L - L. +DM = up if up > down and up > 0 else 0; -DM = down if down > up and down > 0 else 0. Wilder-smooth +DM, -DM and TR; `+DI = 100 * smooth(+DM)/smooth(TR)`, same for -DI; `DX = 100 * |+DI - -DI| / (+DI + -DI)` (0 when both are 0); ADX = Wilder average of DX. Sum-seeding versus average-seeding of DM/TR cancels in the ratios.
- **Bollinger:** middle = SMA(N); bands = middle +/- k * standard deviation of the same N closes. Use the **population** standard deviation (divide by N), as TA-Lib and ChartSchool do. Defaults 20 and 2. Also expose %B = (C - lower)/(upper - lower) and bandwidth = (upper - lower)/middle.
- **Stochastic:** `%K = 100 * (C - LL_N)/(HH_N - LL_N)` (define as 50 when the range is 0), then smooth for slow %K and %D.
- **Donchian:** highest high / lowest low over the N bars. For a breakout test compare today's close to the channel computed through the **previous** bar, otherwise the bar compares to itself.
- **Keltner:** EMA(N) +/- m * ATR(M).
- **Supertrend:** `mid = (H+L)/2`; basic bands `mid +/- m*ATR`. Final upper band keeps the lower of (new basic upper, previous final upper) unless the previous close broke above the previous final upper; final lower band keeps the higher of (new basic lower, previous final lower) unless the previous close broke below it. Direction flips to down when close drops below the active final lower band, to up when close rises above the active final upper band. Output = active band and direction. Defaults N = 10, m = 3 (the common retail convention, widely used on Indian platforms). Sources differ in small condition details and in whether ATR is Wilder or SMA; pick Wilder ATR, document it, and fixture against a TradingView or pandas-ta export recorded with the tool version.
- **Ichimoku (9/26/52):** Tenkan = midpoint of highest high and lowest low over 9; Kijun = same over 26; Span A = (Tenkan + Kijun)/2 shifted forward 26; Span B = midpoint over 52 shifted forward 26; Chikou = close shifted back 26. Look-ahead trap: at bar t the cloud is the span value computed at t-26 (known now), and the Chikou test is "C[t] versus C[t-26]", never a lookup of the future-plotted point.
- **OBV:** running sum adding V when C > prev C, subtracting when C < prev C, unchanged on equal.
- **VWAP (intraday):** cumulative `(H+L+C)/3 * V` divided by cumulative V from the session start (the calendar's `normal` window start for that instrument, `bz-india-markets`); returns `None` while cumulative V is 0. Indices have no volume, so use the index future. **Anchored VWAP:** same with a caller-chosen anchor bar (swing low, results day, gap).
- **Volume spike ratio:** `V / SMA(V, N)` over the previous N bars (exclude the current bar). **Delivery %:** from the EOD bhavcopy per day; EOD-only, never mixed into intraday bars.
- **Pivots (from the previous period's H, L, C):** classic `P = (H+L+C)/3`, `R1 = 2P - L`, `S1 = 2P - H`, `R2 = P + (H-L)`, `S2 = P - (H-L)`, `R3 = H + 2(P-L)`, `S3 = L - 2(H-P)`. Camarilla: with range = H-L, `R1..R4 = C + range * {1.1/12, 1.1/6, 1.1/4, 1.1/2}`, `S1..S4 = C - the same`. Pivot inputs use the prior *completed* session (daily for intraday use, weekly or monthly for swing).
- **Swing high/low with confirmation:** bar i is a swing high if its high is the maximum of bars i-k..i+k. It becomes known at bar i+k; the indicator emits it at bar i+k carrying `pivot_index = i`. Higher-high/higher-low structure uses only confirmed swings. Support/resistance zones are clusters of confirmed swings within a tolerance of ATR, with touch count and recency.
- **52-week high/low:** rolling max/min over a configured number of trading days (default about 250); breakout test uses the value through the previous bar.
- **Relative strength vs Nifty:** `C_stock / C_nifty` aligned on timestamps (skip bars missing on either side, never forward-fill silently), then its ROC or a new-N-day-high test. Use corporate-action-adjusted prices (`bz-market-data`).
- **Gap %:** `(O - prev C)/prev C` using the adjusted previous close; classify full/partial gap fill against prev C.
- **Candlestick patterns:** boolean detectors (engulfing, hammer, doji, morning/evening star, inside bar) with tolerances as params. A pattern alone never emits a signal: the strategy waits for a confirming close beyond the pattern extreme.
- **Pair/cointegration (pair trading):** hedge ratio from rolling OLS on log prices, spread z-score; the Engle-Granger style stationarity test runs offline in research (`backtest`), the live module only monitors the z-score of a pair that passed.

## 3. Implementation notes

- Streaming structs are O(1) per update: ring buffer (`VecDeque` or fixed array) for windows, monotonic deque for rolling max/min. No allocation in `update`.
- Running sums drift over long series: use a compensated sum or recompute the window sum every few thousand updates. For standard deviation recompute from the window (N is small) instead of the E[x^2] - E[x]^2 shortcut, which loses precision.
- Inputs must be finite; reject NaN/inf at the `Candle` to `Ohlcv` (f64) adapter (`TryFrom`, return an error, do not propagate). Candles arrive in strictly increasing `close_time` order; the driver checks it.
- `reset()` must restore the exact initial state so a struct can be reused across instruments in tests.
- Composite indicators own their sub-indicators (ADX owns Wilder smoothers) and expose the intermediate outputs (`+DI`, `-DI`, `ADX`) as a struct.

## 4. Reference-value sources and fixture recipe

| Indicator group | Cross-check source |
|---|---|
| SMA, EMA, RSI, ATR, ADX, MACD, Bollinger, Stochastic, ROC, OBV | TA-Lib function reference at ta-lib.org (compute offline with the TA-Lib Python wrapper) and the worked examples and downloadable spreadsheets in StockCharts ChartSchool (RSI, ATR, ADX pages) |
| Supertrend, Ichimoku, Keltner, pivots | pandas-ta and TradingView built-ins exported to CSV, plus hand-computed micro cases |
| VWAP, anchored VWAP, volume ratio, swing points | hand-computed micro cases (10 to 20 bars) written in the test file |

Fixture recipe: take 300+ completed daily bars of a liquid stock from the official bhavcopy fixtures (adjusted), compute the indicator offline with the named tool, store `fixtures/reference/indicators/<name>.csv` (input columns plus expected columns at full precision) and a `README` with tool, version, parameters and date (tree owned by `bz-testing`). The Rust test loads the CSV, runs the streaming indicator and asserts `abs(a-b) <= 1e-8` (the closed-form tolerance from `bz-testing`) from the first `Some`; when the reference tool seeds differently, skip the window from section 1 first, then apply the same tolerance. Tests never call Python or the network. If you cannot obtain a reference value for a new indicator, say so in the PR and add micro cases by hand; do not invent "expected" numbers from your own implementation.
