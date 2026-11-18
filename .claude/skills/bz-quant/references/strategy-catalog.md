# Strategy catalog

One row per strategy module in `crates/strategies`. Open this before adding a module (copy its row into the module's doc comment: idea, entry, exit, stop, best regime, weaknesses) and when tuning scanner confluence families. Numbers shown are **starting parameters for the params struct**, not facts and not evidence: each must be validated by the backtester with real costs and walk-forward (`bz-backtest-risk`) before a strategy is enabled, and the trader-side evidence bar is in `bz-market-strategy`. All entries trigger on a completed bar; "next open" means the fill model decides, never the signal bar.

Conventions: stops are always defined at entry (price level or ATR multiple) and never widened; targets are expressed in R; `family` feeds scanner confluence (two strategies of the same family count once); horizon is Intraday (I), Swing (S), Positional (P), Long-term (L, not this crate: fundamentals screens live in a separate module).

## 1. Equity: trend (family `trend`)

| Strategy | Idea | Entry | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| EMA/SMA crossover (S,P) | Fast average crossing slow marks a new trend | Fast crosses above slow (fast 20/slow 50 start); optional close > 200 SMA filter | Opposite cross or trailing ATR stop | 2-3 x ATR(14) below entry | Trending up/down, normal or high vol | Whipsaws in ranges; late entries; gives back open profit |
| Supertrend (S,P) | ATR band that flips with trend | Direction flips up (10, 3); require ADX above a floor | Direction flips down | The Supertrend line itself | Trending | Many false flips in ranges; band depends on ATR seed/version |
| ADX + DMI (S,P) | Trade only when trend strength is real | ADX above floor (25 start) and rising, +DI crosses above -DI | ADX rolls over or DI cross back | Below recent swing low or 2 x ATR | Trending, rising ADX | ADX lags; late after a big move |
| Ichimoku (S,P) | Cloud, Tenkan/Kijun and lagging line agree | Close above cloud, Tenkan above Kijun, Chikou above close 26 bars ago | Close back inside cloud or Tenkan below Kijun | Kijun or cloud bottom | Strong trends | Lots of parameters; slow; flat clouds in ranges |
| HH/HL structure (S,P) | Trend = confirmed higher highs and higher lows | Pullback holds above last confirmed higher low, then close above prior swing high | Lower low confirmed | Below last confirmed higher low | Trending | Swings confirmed late (k bars); subjective if k varies |

## 2. Equity: momentum (family `momentum`)

| Strategy | Idea | Entry | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| RSI regime (S) | In uptrends RSI holds a bull zone; dips to the zone edge are buys | Close above 200 SMA, RSI dips to a band (40-50 start) then turns up | RSI above overbought band then falls, or trailing stop | 2 x ATR or below pullback low | Trending up | Fails in distribution tops; band edges are arbitrary |
| MACD (S,P) | Momentum shift of two EMAs | MACD crosses signal while histogram turns positive, above zero line preferred | Opposite cross | 2-3 x ATR | Trending | Lag; noisy in ranges |
| Rate of change (S) | Persistent strength continues | ROC(N) positive and in the top decile of the universe | ROC falls below zero or rank drops | 2 x ATR | Bull markets | Crowded; reverses sharply in corrections |
| Relative strength vs Nifty (S,P) | Leaders outperform in both directions | RS ratio at a new N-day high while Nifty is not falling | RS ratio breaks its trend line (below N-day average) | Below swing low or 2.5 x ATR | Up or sideways index | Needs aligned adjusted data; sector-wide moves mask stock edge |
| 52-week-high breakout (S,P) | New highs attract demand | Close above the prior 52-week high (through previous bar) on volume ratio above 1.5 | Trailing ATR stop or close below 20 EMA | Below breakout bar low or 2.5 x ATR | Trending up | Fake breakouts on thin volume; gap-ups give bad fills |

## 3. Equity: mean reversion (family `reversion`)

| Strategy | Idea | Entry | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| RSI(2) (S) | Short-term oversold bounce inside an uptrend | Close above 200 SMA and RSI(2) below 10 (start) | RSI(2) above 65-70 or close above 5 SMA, or time exit after N bars | Fixed 2-3 x ATR (a wide stop is a feature) or none-with-time-exit tested explicitly | Range or up-trend, normal vol | Catches falling knives in crashes; needs a hard stop; sizing must assume the stop |
| Bollinger reversion (S) | Price reverts to the mean from the band | Close below lower band then closes back inside, with ADX below ceiling | Middle band or opposite band | Below the extreme bar or 2 x ATR | Range-bound | Bands expand in a real trend, so "cheap" keeps getting cheaper |
| VWAP reversion (I) | Intraday stretch from VWAP reverts | Price more than k x ATR(intraday) from VWAP and a reversal bar, after the opening 15 minutes | VWAP touch or end of session | Beyond the extreme or k2 x ATR | Range day, normal vol | Trend days; no VWAP for indices (use the future); needs square-off before the broker's cut-off |

## 4. Equity: breakout (family `breakout`)

| Strategy | Idea | Entry | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| Opening range breakout (I) | First N minutes set the day's range; a break shows direction | After the opening window (15 or 30 min from config), close beyond the range high/low with volume | Target at 1-2 x range, time exit before square-off, or trailing | Opposite side of range or midpoint | Volatile open, trend days | Fake-outs, gap days, wide ranges give poor R; event days |
| Donchian/turtle (S,P) | Classic channel breakout | Close above prior N-day high (20 or 55) | Close below prior M-day low (10 or 20) | Channel low or 2 x ATR | Strong trends | Many small losses; high drawdown in ranges |
| Consolidation / volume-contraction breakout (S) | Tight, quiet base releases energy | ATR% and range contracted vs its history for N bars, then close above base high on volume expansion | Trailing stop or measured move | Base low | Post-consolidation up-trends | Subjective base definition; fake breakouts |
| Cup-and-handle (P) | Rounded base then shallow pullback then breakout | Pattern detector (depth, symmetry, handle depth bounded by params) then close above handle high | Measured move or trailing | Handle low | Bull markets, leaders | Pattern detection is fuzzy and overfit-prone; few samples, so usually low evidence |

## 5. Volume and price action

| Strategy | Idea | Entry | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| Volume spike (S) family `volume` | Unusual participation precedes follow-through | Volume ratio above threshold with a strong close in the bar's range | Trailing or time exit | Spike bar low/high | Any, needs liquidity | News-driven spikes are one-offs; confirms but rarely leads (use as confluence) |
| OBV divergence/trend (S) family `volume` | Accumulation shows in OBV before price | OBV breaks to a new high before price does | OBV trend breaks | Swing low | Base building | Noisy; indices have no volume |
| Delivery % (EOD, S) family `volume` | High delivery share suggests real buying | Delivery % well above its own average with price up and volume ratio above 1 | Time or trailing | Swing low | After pullbacks | Data is EOD and per-day; no intraday use; stock-specific baseline needed |
| VWAP / anchored VWAP (I,S) family `volume` | Where the average participant stands | Reclaim of anchored VWAP from a swing low/results day with volume | Loss of anchored VWAP | Below anchored VWAP | Trending | Anchor choice is discretionary |
| Support/resistance zones (S,P) family `price_action` | Reaction at repeated levels | Rejection candle at a zone confirmed by next close; zones from confirmed swing clusters | Next zone or trailing | Beyond the zone by 0.5-1 x ATR | Range or pullbacks in trend | Zones are subjective; levels break |
| Pivots classic/Camarilla (I) family `price_action` | Prior-day levels act as intraday magnets | Camarilla S3/R3 reversion with confirmation, or R4/S4 breakout | Opposite pivot level or session end | Beyond S4/R4 or ATR | Range day (reversion), trend day (breakout) | Crowded levels; works on average only with regime filter |
| Candlestick patterns, confirmed only (S) family `price_action` | Pattern plus confirming close at a level | Pattern at S/R zone and next bar closes beyond its extreme | Next zone | Pattern extreme | At zones only | Alone has near-zero edge; never emitted without a zone |

## 6. Gap, rotation, pairs

| Strategy | Idea | Entry | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| Gap (I,S) family `gap` | Small gaps tend to fill; large gaps on news continue | Gap fill setup: gap within a band, first 15-minute reversal; Gap-and-go: large gap holding the opening range | Previous close (fill) or range target | Opening range extreme | Gap-fill in range markets; gap-and-go in trends | Needs adjusted previous close (splits/bonus); results-day gaps are events |
| Sector rotation (P) family `rotation` | Money flows to leading sectors | Rank sector indices by relative strength and momentum; hold top N via leaders in them | Rank drops out of top N at rebalance | Per-position ATR stop | Trending broad market | Whipsaws at turning points; index constituents change |
| Pair trading (S) family `stat_arb` | Related stocks' spread reverts | Spread z-score beyond +/-2 (start) for a pair that passed cointegration offline | Z-score back to 0 or time exit | Z-score beyond 3.5 or max holding period | Any, pair-specific | Cointegration breaks (corporate events); needs short leg, so F&O-eligible names only; two legs double costs |

## 7. Futures and F&O analytics (signals, family `derivs`)

| Strategy | Idea | Entry | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| OI build-up confirmation (S) | Fresh positions behind a move | Long build-up above a trend filter (or short build-up below) as confirmation of another family | Build-up turns to unwinding/covering | Price-based (swing/ATR) | Trending | Rollover distortion; cannot separate hedgers; use only as a score input unless backtested alone |
| Futures basis/rollover (S) | Rollover % and basis extremes show positioning | High rollover with rising OI and stable basis agrees with trend | Basis collapse | Price-based | Series transitions | Weak alone; expiry-week noise |

## 8. Options structures (module `options_builder`, horizon S/P)

Defined-risk by default. Entry/exit values below are parameter starting points; all are config.

| Structure | Idea | Entry conditions | Exit | Stop | Best regime | Weaknesses |
|---|---|---|---|---|---|---|
| Long call / long put | Directional, limited loss | Low-to-mid IV rank, clear directional signal, enough DTE for the move | Profit target % of premium, or underlying target, time exit with DTE left | Premium down X% or underlying level | Trending, IV low/rising | Theta decay; IV crush after events; needs both direction and timing |
| Covered call | Income on held stock | Own the lots; sell OTM call, high IV rank, flat/mild up view | Buy back at 50-80% decay, roll before ex-date/expiry | Stock stop on the holding | Range/mild bull, high IV | Caps upside; stock downside remains; lot-size mismatch with holdings |
| Cash-secured put | Get paid to buy at a lower price | Sell OTM put on a stock you accept owning, high IV rank, cash/margin set aside | Buy back at 50-80% decay or accept assignment per settlement rules | Close if support breaks | Range/up, high IV | Undefined-ish downside (to zero), assignment/physical settlement on stock options; show worst case in rupees |
| Bull call / bear put spread (debit) | Directional with capped cost and gain | Directional signal with low-mid IV | Target 50-75% of max profit, time exit | Loss capped at debit; exit earlier at set % | Trending, IV low | Theta works against until in the money; capped upside |
| Bull put / bear call spread (credit) | Sell premium with a hedge | Range or mild directional view, high IV rank, short strike beyond expected move | Buy back at 50% of credit, DTE exit | Close at loss of 1-2 x credit | Range, IV high/falling | Small gain vs large loss; gap risk; pin risk at expiry |
| Iron condor | Sell both wings, profit if range holds | High IV rank, range regime, strikes outside 1 SD expected move | 50% of credit, DTE exit | Close if either short strike is breached or loss 1-2 x credit | Range-bound, IV high | One-sided trends and events; four legs mean higher charges and slippage |
| Iron butterfly | Sell ATM, hedged wings | Very range-bound view, high IV | 25-50% of credit | Underlying leaves breakeven zone | Pinning/low-move | Narrow profit zone; high gamma near expiry |
| Long straddle / strangle | Bet on a big move | Low IV rank before a known catalyst, T long enough | Profit target %, exit after the event | Premium down X%, time exit | Low IV, catalyst ahead | IV crush; needs a move larger than implied |
| Short straddle / strangle | Sell volatility (naked) | Shown only with a strong warning, margin from broker, worst-case loss | 25-50% of credit | Hard stop on premium or underlying | High IV, range | Unlimited-style loss; the default blocks it unless risk rules and user acknowledgement allow (`bz-trading-discipline`) |
| Calendar / diagonal | Profit from time and IV term structure | Flat view, near IV rich vs far; same strike (calendar) or shifted (diagonal) | Near-leg expiry or target % | Underlying leaves range | Range, stable IV | Needs term-structure data; model-estimated payoff; vega exposure |
| Ratio spreads | Skew/cost trade with a naked-leg component | Specific skew view | Target %, strict DTE exit | Mandatory hard stop | Mild directional | HIGH RISK flag always: net short quantity beyond the hedge; excluded from default scans |

Selection heuristic (feeds `bz-market-strategy` section 6, which owns the IV rank bands; IV percentile is shown as context only; not a rule engine): high IV rank with a range regime favours credit spreads and condors; low IV with an expected move favours debit spreads or long options; high VIX with a trending-down regime favours caution and smaller size; event ahead (results, RBI, Budget) penalises short premium into the event unless that is the explicit thesis. Exits: profit-target % of max profit, stop on premium or underlying level, and a time exit before expiry (avoid pin risk and expiry-day gamma). Check the lot size and expiry from the instrument master, margin from the broker API, charges from `charges.toml` via the `CostModel`.

## 9. Adding a strategy

1. Write the spec row (here and, with evidence, in `docs/strategy/<name>.md` via the strategist).
2. Create `strategies/src/<name>.rs` with params struct (`Default`, `validate`, ranges), the doc comment, `StrategyFactory`, and `on_bar`.
3. Add reference/micro-case tests, the no-look-ahead test and a fixture-based "fires exactly on these bars" test.
4. Register it in the registry and `strategies.toml` (disabled by default until backtest evidence exists).
5. Run the walk-forward and cost-adjusted backtest (`bz-backtest-risk`); record the stats in the PR.
