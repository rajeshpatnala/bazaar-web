# F8b Candidate strategies after Indian-market research (market-strategist, 2026-11-20)

Status: research shortlist, specs are drafts. Not investment advice. Nothing here is shown to make money for this account: published edges decay, mostly pre-date costs and tax, and were tested on index-sized portfolios, not Rs 1,00,000 with 3 positions. Every strategy ships `enabled = false`, "Low evidence" until F9 runs it net of costs. Every default is UNVERIFIED. Variants tried so far: 0. Account: Rs 1,00,000, 1% risk = Rs 1,000/trade, 3 open positions, Rs 25,000 position cap (`config/risk.toml`).
Evidence grades: **G1** index-provider documented or regulator data (not independent proof of a strategy); **G2** academic paper, venue/peer review UNVERIFIED; **G3** backtest blog; **G4** influencer/promo. Sources S1-S14 at the end. Search-summary figures were not all checked in the primary PDFs (SEBI and NSE PDFs would not parse; S&P returned 403): re-verify before quoting in the UI.

## Table (build order: 1 first; "-" = not built)
| Name | Family | Horizon | Evidence | Regime fit | Cost sensitivity | Data | Order |
|---|---|---|---|---|---|---|---|
| `momentum_rank` | momentum | positional (monthly) | G1 S1,S2; G2 S3 (momentum paid in down markets per summary); drawdowns deep (fund max DD about -30%, S2b) | market Up/Range; suppress Down/Breakdown | low-medium: about 6-12 round trips a year; STT + flat DP fee per sell, tax (S14) | daily OHLCV, universe ranks | 1 |
| `high52_leader` | momentum | swing/positional | G2 S7 (India, venue UNVERIFIED) | stock Up | low-medium | daily OHLCV | 2 |
| `low_vol_trend` | defensive | positional (monthly) | G1 S5 factsheets (method only); G2 conflicting S4 (one study finds the effect, one finds none) | market Range/High vol; lags strong Up | low | daily OHLCV | 3 |
| `results_drift` | event drift | swing (20-40 days) | G2 S6 (Nifty 500, 2002-2017; positive side only); G3 blog | stock Up after good results | medium: needs wider stop | daily OHLCV + **results-date feed (new)** | 4 |
| `delivery_confirm` | volume (confirm only) | n/a | G2 weak S13 (2 of 13 stocks) | confluence only | none (score input) | bhavcopy delivery % | 5 |
| `orb_index` (stretch) | breakout | intraday | G3 conflicting S11 ("all 9 versions lost money" vs +91%) | trend day | **high**: SEBI S10: loss-makers spent 57% of losses on costs | **5-min candles (new)** | - |
| `credit_spread_ivrank` (stretch) | options, defined risk | swing | G1 only for outcomes (S9); none for the structure | Range, high IV | **high**: 4 legs, lot sizes | **option chain + IV history (new)** | - |
| Donchian-250 preset | breakout | swing | G2 S7 | Up | low | have | variant of F8 `donchian_breakout` (counts as a trial) |
| Supertrend, MA crossover, RSI(2), sector rotation, pairs, ORB-as-default | see rejects | | G3/G4 | | | | no |

## Shared facts behind the shortlist
- SEBI (S9, press-reported): about 88% of individual F&O traders lost money in FY26 (91% in FY25); options were about 92% of losses; majorly-options-sellers had the lowest loser share (44%) but about 11x larger average loss per loser than buyers; sellers are about 2% of the sample. Defined-risk structures are not separated out. Intraday cash (S10, FY23): 71% lost; costs hurt most. This is why the first picks are daily, long-only delivery.
- Cross-sectional rules (`momentum_rank`, `low_vol_trend`, `high52_leader`) need ranks over the whole liquid universe. `Strategy::on_bar` sees one instrument: **ranks must come from a scanner-built context** (tech-lead question). Without it these three cannot be built.
- Delivery costs for a Rs 25,000 position: STT both sides and a flat DP fee per sell (see `config/charges.toml`) are a larger share of a small position, so short holds are punished. Tax (S14, secondary, UNVERIFIED): short-term gains on listed shares 20%; long-term 12.5% above Rs 1.25 lakh a year. Most holds here are short-term, so test the net-of-tax result as well.
- Concentration: NSE momentum/low-vol indices hold 30 stocks; we hold at most 3, so the real variance and drawdown are far larger than the index's. Index results are context, not a forecast.

## 1. `momentum_rank` (positional, long, family momentum)
Idea: stocks that rose most over 6-12 months, adjusted for their volatility, tend to keep outperforming for a few months (NSE builds Nifty200 Momentum 30 this way, rebalanced half-yearly; exact score UNVERIFIED). Other side: late sellers and profit-takers; it breaks in sharp reversals.
Entry (month-end close t, closed bars only): score = mean(R126, R252)/sigma252; stock in the top `top_pct` of the liquid universe; close > SMA200; R252 > 0; market trend not Down/Breakdown; max 3 signals (best scores first). Fill next open.
Stop: entry_high - `stop_atr_mult` x ATR14, raised only. Exit at a month-end close if rank leaves the top `exit_pct` or close < SMA200; time stop `max_hold_bars`.
Params: lookbacks 126/252 (100-150 / 200-260); `top_pct` 10 (5-20); `exit_pct` 30 (20-50); `stop_atr_mult` 3.0 (2.5-4.0); `skip_recent_bars` 0 (0-21); `max_hold_bars` 126 (60-252). Tuned count: 2 (lookback, `top_pct`); hold the others fixed.
Sizing: price Rs 500, ATR Rs 12: stop Rs 36; 27 shares = Rs 13,500 (about Rs 1,000 risk) before the Rs 25,000 cap and G16.
Weaknesses: momentum crashes at market turns; crowded; 3 names is noisy; gap risk; rank churn raises costs.
Evidence needed: bar in `bz-market-strategy` section 5 net of costs and tax; add 2x slippage test, rank-universe survivorship note, at least 100 trades for Trusted (needs a wide universe and 5+ years).
Why text: "Risk first: about {risk_rupees} could be lost. {symbol} is in the top {top_pct}% of stocks by 6-12 month strength. Market: {regime_label}. Wrong if price falls to {stop} or it drops out of the leaders at month-end. Strong stocks can also reverse fast; this is a probability, not a promise."

## 2. `high52_leader` (swing/positional, long, family momentum)
Idea: stocks near their 52-week high keep drifting up because holders anchor on the high (George-Hwang idea; India paper S7). Differs from Donchian by buying proximity, not the break.
Entry: close >= (1 - `near_pct`) x highest high of t-252..t-1; SMA50 > SMA200; R126 > Nifty R126; volume ratio >= `vol_ratio_min`; no add to holdings. Stop: entry_high - 2.5 x ATR14. Exit: close < EMA50 or `max_hold_bars` 60.
Params: `near_pct` 3% (2-8); `stop_atr_mult` 2.5 (2.0-3.5); `vol_ratio_min` 1.0 (1.0-1.5); `max_hold_bars` 60 (40-100).
Weaknesses: overlaps heavily with `momentum_rank` and `donchian_breakout`; buying near highs gets stopped on normal pullbacks. Retire if over 70% of its trades overlap another strategy's with no incremental expectancy (scanner counts the family once).

## 3. `low_vol_trend` (positional, long, family defensive)
Idea: the least volatile stocks have, in some Indian samples, matched or beaten the market with smaller swings, and held up in falling markets (S3 summary; the studies disagree, S4). Role: diversify away from momentum, not a return booster.
Entry (month-end): sigma252 in the lowest `low_vol_pct` of the universe; close > SMA200 and R252 > 0 (avoid value traps); market not Breakdown. Stop: 2.5 x ATR14 (tight in rupees: small ATR means many shares; the Rs 25,000 cap will bind). Exit: month-end sigma rank leaves the lowest 40% or close < SMA200.
Params: `low_vol_pct` 20 (10-30); `exit_pct` 40 (30-50); `stop_atr_mult` 2.5 (2.0-3.5); `sigma_bars` 252 (120-252).
Weaknesses: underperforms in strong rallies; defensive sectors crowd in (sector cap G05); modest gains can be eaten by costs. Honest status: the evidence is mixed, so it may be rejected by F9.

## 4. `results_drift` (swing, long, event)
Idea: prices keep adjusting for weeks after good results (post-earnings drift; Nifty 500 study S6). We have no earnings-surprise number (paid data), so this is a **price-reaction proxy**, weaker than the studied effect.
Entry (signal on the first close at least 1 session after the results-day reaction bar; G07 stays in force on results day): reaction-day return >= `min_reaction_pct`; volume >= 2x SMA20; reaction close in the top 30% of its range; today's close above the reaction-day close and above SMA50. Stop: reaction-day low - 0.5 x ATR14 (drop if more than 3 ATR away). Exit: close < EMA20 or `max_hold_bars` 40.
Params: `min_reaction_pct` 4 (3-8); `vol_mult` 2.0 (1.5-3.0); `stop_buffer_atr` 0.5 (0.25-1.0); `max_hold_bars` 40 (20-60).
Weaknesses: results gaps are large (stop far, small size); drift reported only on the positive side (negative needs shorts); few trades per year, so Low evidence for long. Needs an official results-date feed (NSE announcements): out of F8's data today.

## 5. `delivery_confirm` (scanner input, not a strategy)
Idea: high share of delivery volume suggests investors, not day traders, are buying. Evidence is weak (S13), so it only nudges a score on other strategies' signals: delivery % >= 1.3x its own 20-day average on the signal bar. Never standalone; weight stays 0 until F9 shows added expectancy over the same signals without it. Data: bhavcopy (official).

## Stretch (need new data; do not start before F9 passes and the owner decides)
- `orb_index`: first-15/30-minute range break on index futures. Blog evidence conflicts (S11) and intraday retail outcomes are poor (S10). Build only as a **falsification test** on 5-min data, expect a reject. Needs intraday candles, square-off time from config.
- `credit_spread_ivrank`: bull put / bear call spread when IV rank is high and regime is Range, strikes beyond about 1 expected move, exit at 50% of credit. Needs option chain, IV history, margin from the broker; the worst case in rupees must be shown first and one lot may not fit the budget (UNVERIFIED margin). SEBI shows sellers lose less often but far more per loss (S9); defined risk removes the tail but not the costs. Verdict: not now.

## Reject list
| Idea | Reason |
|---|---|
| Naked short straddle/strangle, any undefined risk | Tail losses (S9) vs a Rs 1,000 risk budget; blocked by G13 |
| Expiry-day scalping, zero-DTE option buying | Option buyers are the worst-hit group in SEBI data (S9); costs; no evidence |
| Short-window MA crossovers, 50/200 as-is | Gains vanished after costs; breakeven cost about 0.16% for a 3-day SMA (S8); 50/200 failed out-of-sample in a GitHub test (G3) |
| Supertrend standalone | Only blogs, win rate 40-43%, costs unclear (S12); keep as an indicator |
| RSI(2) / Connors dip-buy | No India evidence found; duplicates `bollinger_reversion` and `trend_pullback`; each extra variant adds trials |
| Sector rotation, pairs trading, cup-and-handle | No evidence gathered; pairs need shorts; pattern detection is fuzzy |
| Candlestick-only, "90% win rate", Telegram/YouTube setups | G4; no stops or costs shown; cannot be tested |
| Martingale, averaging down, stop widening | Hard rules G08/G09/G11 |

## Decisions for the owner
1. Approve a scanner-built universe rank context (needed by 3 of the 5)? 2. Approve an official results-date feed for `results_drift`? 3. Accept that only daily long-only ideas are built and both stretch items wait for F9 results? 4. Test net of tax (20% short-term) as well as before tax?

## Sources
S1 https://www.niftyindices.com/docs/default-source/indices/nifty200-momentum-30/momentum-strategy-whitepaper_2026.pdf (19.19% vs 15.08% since Apr 2005, pre-cost, via search summary) | S2 https://www.niftyindices.com/Factsheet/Factsheet_Nifty200_Momentum30.pdf ; S2b fund-level max DD -30.11% https://arthgyaan.com/fund/bandhan-nifty200-momentum-30-index-fund-direct-growth/150591
S3 https://www.spglobal.com/spdji/en/documents/research/factor%20risk%20premia%20in%20the%20indian%20market.pdf (403 here) | S4 https://www.scirp.org/html/28-1501934_94780.htm and https://www.vbu.ac.in/ftpwebapps/vbu/resources/vbu_web/dept/mba/Dr.%20Sitaram%20Pandey%20Journal%208.pdf (opposite findings; which is which not mapped)
S5 https://www.niftyindices.com/Factsheet/Factsheet_NIFTY_Alpha_Quality_Low-Volatility_30.pdf | S6 https://www.scirp.org/journal/paperinformation?paperid=88060 ; blog https://blog.tradingstudio.finance/pead-india/ | S7 https://www.researchgate.net/publication/374825583_The_52-Week_High_Effect_and_Momentum_Investing_Evidence_from_India
S8 https://www.researchgate.net/publication/267254777_Usefulness_of_Moving_Average_Based_Trading_Rules_in_India ; https://github.com/ambatkarabhi/ema-crossover-backtest | S9 SEBI Aug 2026 studies https://www.sebi.gov.in/sebi_data/attachdocs/aug-2026/1787233506209.pdf ; press https://www.moneylife.in/article/92-percentage-of-aggregate-losses-incurred-by-individuals-are-from-options-trading-sebi-study/81429.html
S10 SEBI PR 13/2024 via https://taxguru.in/sebi/sebi-study-finds-7-10-individual-intraday-traders-equity-cash-segment-losses.html | S11 https://strikeatm.com/backtest-lab/nifty-opening-range-breakout ; https://intradaylab.com/blog/nifty-orb-breakout-strategy-backtest | S12 https://www.share.market/buzz/insights/i-backtested-200000-trades-using-the-supertrend-indicator-here-is-what-actually-works
S13 https://www.aminer.org/pub/61c754915244ab9dcb0da8bc/study-on-relationship-between-delivery-quantity-to-total-quantity-traded-and-stock | S14 https://www.bajajbroking.in/share-market-news/stt-ltcg-and-stcg-on-stocks-in-india-2026-tax-guide (secondary, UNVERIFIED against the Finance Act)
