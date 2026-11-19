# F7 regime detection: definitions (market-strategist pre-plan, 2026-11-20)

Status: specified. Not investment advice. A regime label describes recent market behaviour; it does not predict the next bar.
Owner limits for context: Rs 1,00,000, 1% risk/trade (Rs 1,000), 3% daily loss (Rs 3,000), paper mode.
Every default below is **UNVERIFIED**: each is a widely used convention, not source-checked for this app. All live in config
(proposed `[regime]` table in `config/strategies.toml`; tech-lead may prefer a separate file), each with `source` and `verified_on`
left unset until the owner or a later feature confirms. Code holds no threshold literals. The agent does not edit config.

## 1. What F7 outputs
Daily bars only (intraday regime is a later feature). One pure function `classify(candles, params, prev_state) -> RegimeState`, run
twice: on the **index series** (Nifty 50, the "market regime") and on the **instrument's own series** ("stock regime").
`StrategyContext` carries both (amendment to the single `regime` field in `bz-quant` section 5; tech-lead to confirm).

| Axis | States | Meaning for a beginner |
|---|---|---|
| Trend | `Up`, `Down`, `Range`, `Mixed`, `Unknown` | Mixed = signals disagree (treated as no trade). Unknown = not enough bars or missing input (no trade, reason shown) |
| Volatility | `Low`, `Normal`, `High`, `Unknown` | How big recent daily swings are compared with this series' own last year |
| Breakdown flag (market series only) | `true`/`false`/`unknown` | Index weak and swings large: no new long ideas at all |

`RegimeState` also carries: `as_of` (close time of the last bar used, IST), the indicator values used, `reasons: Vec<Fact>`
(plain English, for the Why panel), `breadth_pct_above_50dma: Option`, `india_vix: Option`, `vix_percentile: Option`.
Missing optional input (breadth, VIX) is `None` plus a reason; it is never guessed. India VIX and breadth need F3 data, so in F7
they are inputs on the struct but the classifier works without them and says so.

## 2. Indicators (all from F4, closed bars only)
EMA(fast_mid=50), EMA(slow=200), ADX/+DI/-DI(14), ATR(14) and ATR% = ATR/close, percentile rank of ATR% over a trailing window.

## 3. Rules
**Raw trend at bar t** (before smoothing):
- `Up_raw`: close > EMA50 and EMA50 > EMA200 and EMA50 slope over `slope_bars` > 0 and ADX >= `adx_trend_enter`.
- `Down_raw`: mirror image (close < EMA50 < EMA200, slope < 0, ADX >= `adx_trend_enter`).
- `Range_raw`: ADX < `adx_range_max` and |EMA50 slope %| <= `slope_flat_max_pct`.
- Anything else: `Mixed_raw`.
- Hysteresis on strength: if the confirmed state is already Up or Down, the ADX test uses `adx_trend_stay` (lower) instead of `adx_trend_enter`, so a trend is not dropped the moment ADX dips.
**Confirmation:** the published state changes only after `confirm_bars` consecutive closes agree on the new raw state. Until the first
confirmed state exists the answer is `Unknown`. This stops the badge flapping.
**Volatility:** percentile rank of today's ATR% inside the last `vol_lookback_bars` values of ATR% (including today). `Low` if rank <= `vol_low_pct`,
`High` if >= `vol_high_pct`, else `Normal`; same `confirm_bars` smoothing. Market series: if `vix_percentile` is present (F3) use it instead of ATR%
(config switch `market_vol_source = "vix_if_available"`); otherwise index ATR%.
**Breakdown (market series):** close < EMA200 and vol == High and (trend == Down or breadth < `breadth_breakdown_max_pct` when breadth is present).
Without breadth it uses what exists and the reason says "breadth not available".
**Warm-up:** the indicators warm up in parallel, so `min_history_bars = max(ema_slow, 2*adx_period, ema_mid + slope_bars, atr_period + vol_lookback_bars - 1) + confirm_bars - 1` (about 265 at defaults); fewer bars gives `Unknown`. F4/F7 fixtures need at least about 300 synthetic daily bars.
**Precedence:** `adx_range_max` must not exceed `adx_trend_stay` (config check), so Range and a held trend cannot overlap; Up/Down are tested first, then Range, then Mixed.

## 4. Parameters
| Name | Default | Range for stability check | Meaning |
|---|---|---|---|
| `ema_mid`, `ema_slow` | 50, 200 | 40-60, 150-250 | Trend averages (convention; UNVERIFIED) |
| `adx_period`, `atr_period` | 14, 14 | fixed | Wilder defaults (UNVERIFIED); ATR has its own key so the volatility axis does not move with ADX |
| `adx_trend_enter` / `adx_trend_stay` | 25 / 20 | 22-28 / 17-23 | Trend strength to enter / keep a trend label |
| `adx_range_max` | 20 | 17-23 | Below this, no trend strength |
| `slope_bars`, `slope_flat_max_pct` | 10, 2.0 | 5-15, 1-3 | "Flat average" test |
| `confirm_bars` | 3 | 2-5 | Closes needed to flip a label |
| `vol_lookback_bars` | 250 | 200-300 | About one trading year for the percentile |
| `vol_low_pct` / `vol_high_pct` | 25 / 75 | 20-35 / 65-80 | Percentile cut-offs |
| `breadth_breakdown_max_pct` | 30 | 20-40 | Breadth below this counts as collapsing |
| `market_vol_source` | `vix_if_available` | enum | Where market volatility comes from |

These are classifier definitions, not fitted parameters. **Do not tune them on strategy P&L.** Allowed checks only: the share of days in each
state on a long history looks sensible (no state above about 60% or below about 5% without explanation), and the +-20% stability sweep does not change
labels wildly. A wide `Mixed` share is expected and is accepted: fewer signals is the cost of "unsure means no trade".

## 5. Strategy fit (how F8 and the scanner use it)
Each strategy declares `regime_fit` as a list of (trend, vol) cells plus the market conditions it needs. Helper `RegimeFit::allows(&market, &stock)` lives in
`regime`. Hard suppress (no signal): any `Unknown`, `Mixed`, market Breakdown. Soft penalty (scanner score) for allowed-but-weaker cells such as Up x High.
The Why panel always names the regime and whether it fits.

Labels (plain English, templated, snapshot-tested):
| Cell | Label |
|---|---|
| Up x Low / Normal / High | "Uptrend, calm swings" / "Uptrend, normal swings" / "Uptrend, wild swings" |
| Range x Low / Normal / High | "Sideways, calm" / "Sideways, normal swings" / "Sideways, wild swings" |
| Down x any | "Downtrend" + swing word |
| Mixed | "Unclear: signals disagree" |
| Breakdown | "Market under stress: no new buys" |
| Unknown | "Not enough data to judge the market" |

## 6. Known weaknesses
- Lagging: EMA/ADX confirm trends late, and `confirm_bars` adds more lag. Labels describe the past.
- Sharp V-turns: a crash then rebound flips Breakdown to Up only after several bars, so early-recovery trades are missed. Accepted.
- Percentile of ATR% is relative to the last year only: a long calm stretch makes a normal day look "High".
- Index regime does not equal a stock's regime; hence two classifications.
- Mixed zone (ADX between the range and trend thresholds) can be common; fewer signals.

## 7. Acceptance criteria for dev-quant (F7 evidence bar)
- [ ] Hand-computed micro cases: constructed uptrend, sideways, downtrend, crash fixtures classify as Up / Range / Down / Breakdown with stated indicator values.
- [ ] `lookahead_` prefix-invariance test: output at bar t is identical whether or not bars after t exist (property test over random prefixes).
- [ ] Hysteresis test: noise series flips at most N times per 100 bars; a single contrary close never flips a label.
- [ ] Warm-up: fewer than `min_history_bars` bars gives `Unknown` + reason; zero NaN/inf; flat and zero-volume series handled.
- [ ] Missing VIX/breadth gives `None` + reason, never a default value; `as_of` equals the last closed bar's close time; no wall-clock reads.
- [ ] Deterministic; thresholds only from config; `RegimeFit` unit tests for each strategy cell table in F8.
- [ ] Later (F9, not a tuning loop): on real data, report forward-return and drawdown by regime cell as a sanity picture.

## 8. Reject list
Machine-learning or hidden-Markov regime models (unexplainable, overfit-prone, no data yet); regime chosen by whichever label makes a backtest look best;
regime from a single indicator; changing the label intrabar; guessing VIX or breadth when absent; any "force regime" switch; using future bars to
"clean up" labels afterwards.
