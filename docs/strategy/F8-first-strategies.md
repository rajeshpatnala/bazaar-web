# F8 Strategy trait, Signal and the first three strategies (market-strategist pre-plan, 2026-11-20)

Status: specified (all three strategies: draft until F9 evidence). Not investment advice; no setup here is certain. Owner: Rs 1,00,000, 1% risk/trade
(Rs 1,000), 3% daily loss, paper mode. Scope: NSE cash equity, **long only, daily bars, swing horizon**, no options, no intraday, no shorts.
Reason for long-only: short exposure needs futures (later phase per `project.md`), and F8's job is correct, testable plumbing on fixtures.
Catalogue rows (idea/weakness text) are in `bz-quant/references/strategy-catalog.md`; this file states only what is specific to this account.
Regime states come from `docs/strategy/F7-regime-definitions.md`. Every default is **UNVERIFIED** (convention or proposal, not evidence), lives in
`config/strategies.toml` under `[strategy.<id>.params]`, and all three ship `enabled = false`. Variants tried so far: **0** (any later variant adds to the count).

## 1. Why these three
They cover three different regime cells and three families, use few knobs (about 3 tuned each, so about 90 in-sample trades each is the target), and are explainable to a beginner.
| Id | Family | Works in | Role |
|---|---|---|---|
| `donchian_breakout` | breakout | stock Up | Catches the start of strong moves; many small losses |
| `trend_pullback` | trend | stock Up, calm or normal | Buys a dip inside an uptrend with a close stop |
| `bollinger_reversion` | reversion | stock Range, calm or normal | Buys a stretch below the average in a sideways stock |
Two of the same family on one stock count once for the scanner. Market gate for all three: market trend in {Up, Range} and not Breakdown, else no signal.

## 2. Trait and Signal contract (what dev-quant must build)
Base shape: `bz-quant/references/strategy-trait-and-signal.md`. Needs beyond it (tech-lead to confirm; each is a deliberate change to that skill text):
1. **Closed bars only, no look-ahead.** `on_bar` sees a `DataView` ending at the closed decision bar; the signal time is that bar's `close_time` (IST); no `Clock` or wall-clock call in a strategy. Channel and band levels use bars **before** the decision bar where stated. The regime passed in has `as_of <= view.last().close_time` (assert in a test). Index and stock bars are aligned by session date, never by "latest available".
2. **Fill later.** The signal is raised at bar t close; the backtest fills at t+1 open plus slippage (never the signal bar close).
3. **Exit hook.** The trait as written only enters. Add `fn manage(&mut self, view, ctx, position: &OpenPosition) -> ExitAction` with `Hold | Exit(reason) | TightenStop(new_stop)`; `TightenStop` is a type-checked "only upward for a long" (returns Err otherwise; matches G09). F9 needs it for trailing and time exits.
4. **Extra Signal fields:** `signal_bar_time`; `params_hash` and strategy version (provenance); `exit_plan { time_stop_bars, rule_text }`; `reference_rr` (reward to first target over risk); `risk_per_share = entry_high - stop` (Decimal; the gate re-derives it, never trusts it); `facts` (below). Prices are Decimal, rounded to the instrument-master tick: stop rounded **down**, the top of the entry zone rounded **up** (both enlarge the risk; manager decision, slice 1 review), entry low and targets to the nearest tick (ties away from zero), never toward a smaller risk.
5. **Entry zone.** `[close_t, close_t + entry_zone_atr_frac x ATR]`, valid until the close of the next session (from the calendar). Risk is measured from `entry_high` to the stop (worst case inside the zone). G10 allows no tolerance beyond the zone.
6. **Self-protection, return `None` (with a `debug` reason) when:** fewer than `min_history_bars`; any needed indicator is `None`; candles are not `adjusted` (corporate-action risk); regime Unknown/Mixed or outside `regime_fit`; the instrument is already in `ctx.portfolio` (no adding to a position, G11); stop on the wrong side or at/above entry; stop distance above the strategy's cap; reference reward:risk below `min_rr`. Cool-off, re-entry lockout and daily caps stay in the gate (G08/G17/G18), not in strategies.
7. At most one signal per instrument per bar; deterministic; no randomness; no thresholds in code.
8. Tests: `lookahead_` (append or drop later bars, earlier outputs identical), hand-computed fixture per strategy ("fires exactly on bar k with this entry, stop, target"), a "fires on no other bar" case, a wrong-regime case, an unadjusted-candle case, a tick-rounding case.

## 3. `donchian_breakout` (swing, long)
Idea: a close above the highest high of the last N days shows buyers pushing past recent sellers; strong trends often keep going. Other side: late sellers and fade traders.
Universe/filters (scanner, not strategy): liquid names only (min median 20-day traded value from config), not on the ban list, no results/policy event inside the hold window, not circuit-locked.
Regime fit: Works: stock Up x Low/Normal; Up x High allowed with the scanner's reduced-size penalty. Suppress: Range, Down, Mixed, Unknown, market Down or Breakdown.
Entry (closed bar t): close_t > max(high over t-N..t-1); volume_t / SMA20(volume over t-20..t-1) >= `vol_ratio_min`; (close_t - channel_high) <= `max_extension_atr` x ATR (skip gap-chases).
Stop: entry_high - `stop_atr_mult` x ATR(14). May only be raised: the manage hook lifts it to the M-day low (`exit_n`) once that is above it.
Exit: close_t < min(low over the last `exit_n` bars before t) exits at the next open; or time stop `max_hold_bars`. Target: none is a limit order; `targets[0]` = entry_low + 2 x risk is a **reference** shown for reward:risk only.
Sizing note (illustration, not data): price Rs 500, ATR Rs 10: stop about Rs 27 away; 37 shares is about Rs 18,600 and about Rs 1,000 risk before the Rs 60 buffer. Capped by G16 (Rs 10,000 per order now) it becomes 19 shares and about Rs 475 risk.
| Param | Default | Range | Meaning |
|---|---|---|---|
| `entry_n` | 20 | 15-30 | Days in the breakout channel (tuned) |
| `exit_n` | 10 | 7-14 | Days in the exit channel (tuned) |
| `stop_atr_mult` | 2.5 | 2.0-3.0 | Initial stop distance (tuned) |
| `vol_ratio_min` | 1.2 | 1.0-1.5 | Volume confirmation |
| `max_extension_atr` | 1.0 | 0.5-1.5 | Skip if already stretched |
| `max_hold_bars` | 60 | 40-80 | Time stop |
| `min_rr` | 1.5 | 1.5-2.0 | Reference reward:risk floor |
Weaknesses: many small losses and false breakouts in ranges (regime filter reduces, does not remove); gap-up entries fill above the zone and the setup is then "missed";
gives back open profit before the exit channel triggers; results-day gaps are blind spots; crowded breakouts reverse in market stress.
Why text (fields in braces): "Risk first: you could lose about {risk_rupees} on this trade. Setup: {symbol} closed above its {entry_n}-day high on {volume_ratio}x normal volume. Market today: {regime_label}; this setup likes uptrends. Proven wrong if: price closes below {stop}. Exit plan: we leave if it closes below its {exit_n}-day low or after {max_hold_bars} days. Watch out: breakouts fail often; many small losses are normal for this idea."
Facts: `channel_high`, `breakout_close`, `volume_ratio`, `atr`, `stop_basis`, `exit_rule`, `time_stop_bars`, stock and market regime labels.

## 4. `trend_pullback` (swing, long)
Idea: in an established uptrend, a short dip toward the 20-day average is often bought by trend followers; entering when price turns back up gives a close stop. Other side: early profit-takers.
Regime fit: Works: stock Up x Low/Normal. Suppress: Up x High (dips are deeper, stops wide, size tiny), Range, Down, Mixed, Unknown, market Down or Breakdown.
Entry (closed bar t): some close in t-`pullback_lookback`..t-1 was below EMA20 at its own bar; close_t > EMA20_t; close_t > high_{t-1}; close_t > EMA50_t.
Stop: (lowest low over t-`pullback_lookback`..t) - `stop_buffer_atr` x ATR; drop the signal if entry_high - stop > `max_stop_atr` x ATR.
Targets/exit: target1 = entry_low + `target_r` x risk (limit); exit at the next open if close < EMA50; time stop `max_hold_bars`. v1: fixed stop (no trailing); trailing stops belong to donchian_breakout.
Sizing note: typical stop 1.5 to 2.5 ATR; price Rs 500, ATR Rs 10 gives about Rs 15-25 risk per share: 40-60 shares, about Rs 20,000-30,000, so G14 (Rs 25,000) and G16 will often shrink the size. Say so in the panel.
| Param | Default | Range | Meaning |
|---|---|---|---|
| `pullback_lookback` | 5 | 3-8 | Window to look for the dip (tuned) |
| `stop_buffer_atr` | 0.5 | 0.25-1.0 | Room under the pullback low (tuned) |
| `target_r` | 2.0 | 1.5-3.0 | Reward in R (tuned) |
| `max_stop_atr` | 3.0 | 2.0-3.5 | Skip if the stop is too far |
| `max_hold_bars` | 15 | 10-25 | Time stop |
| `min_rr` | 1.5 | 1.5-2.0 | Reward:risk floor |
Weaknesses: a dip can be the start of a trend change (the stop, not hope, handles it); gap-downs through the stop fill at the open, worse than planned; overlaps with `donchian_breakout` in strong trends (same trade, counted once);
EMA20 touch is arbitrary and shallow dips never trigger; 15-day time stop can cut slow winners.
Why text: "Risk first: you could lose about {risk_rupees}. Setup: {symbol} is in an uptrend and dipped for {days} days, then closed back above its 20-day average. Why now: today's close is above yesterday's high. Market today: {regime_label}. Proven wrong if: price closes below {stop}. Target: {target1} (about {rr} times the risk), or we leave if it falls under its 50-day average."
Facts: `ema20`, `pullback_low`, `days_in_pullback`, `atr`, `stop_basis`, `target_r`, `exit_rule`, regime labels.

## 5. `bollinger_reversion` (swing, long)
Idea: in a sideways stock, a close below the lower band that comes back inside often drifts to the average; short-term overreaction fades. Other side: panicked sellers. This is the one that loses most in a real breakdown, so the regime and market gates are strict.
Regime fit: Works: stock Range x Low/Normal. Suppress: Range x High (stand aside), Up, Down, Mixed, Unknown, market Down or Breakdown.
Entry (closed bar t): close_{t-1} < lower_{t-1}; close_t >= lower_t; close_t > open_t; band = SMA(`bb_n`) -/+ `bb_k` x stdev.
Stop: min(low_t, low_{t-1}) - `stop_buffer_atr` x ATR; drop if entry_high - stop > `max_stop_atr` x ATR.
Targets/exit: target1 = SMA(`bb_n`) at t (limit); reference reward:risk must be >= `min_rr`; exit at the next open if the stock's regime is confirmed Down; time stop `max_hold_bars`.
Sizing note: stop about 1 to 1.5 ATR; price Rs 500, ATR Rs 10: stop Rs 10-15 away, 60-90 shares by risk alone (Rs 30,000-45,000), so G14 or G16 decide the size, and small quantities make the Rs 60 buffer a large share of the planned gain. Many setups will be dropped by `min_rr`; that is intended.
| Param | Default | Range | Meaning |
|---|---|---|---|
| `bb_n` | 20 | 15-30 | Average length (tuned) |
| `bb_k` | 2.0 | 1.8-2.5 | Band width in standard deviations (tuned) |
| `stop_buffer_atr` | 0.25 | 0.1-0.5 | Room under the lows (tuned) |
| `max_stop_atr` | 2.0 | 1.5-2.5 | Skip if the stop is too far |
| `max_hold_bars` | 8 | 5-12 | Time stop |
| `min_rr` | 1.2 | 1.2-2.0 | Reward:risk floor (low by design; mean reversion wins small; costs bite hardest here) |
Weaknesses: falling knives when a range breaks (gap down through the stop); bands widen in a real trend so "cheap" keeps getting cheaper; targets are small so costs and slippage eat a large share of the edge; few signals if Range is rare (low evidence for a long time); results/news gaps.
Why text: "Risk first: you could lose about {risk_rupees}. Setup: {symbol} has been moving sideways, fell below its usual range yesterday and closed back inside today. Why now: prices like this have tended to drift back to the average ({target1}). Market today: {regime_label}; this setup works in sideways markets and is skipped in downtrends. Proven wrong if: price falls below {stop}. Watch out: if the stock starts a real downtrend this idea loses; we leave if it does."
Facts: `lower_band`, `prev_close`, `middle_band`, `atr`, `stop_basis`, `exit_rule`, `time_stop_bars`, regime labels.

## 6. Evidence bar and what F8 can and cannot prove
F8 on fixtures proves **correct rules and no look-ahead only**. It proves nothing about profit. Until F9 + F3 real data, every strategy is "no evidence"; labelled **Low evidence** in the UI; G01 allows approval only as paper, tagged `exploratory`.
Before "Provisional/Trusted" (house defaults from `bz-market-strategy` section 5, config keys in `risk.toml [gate.evidence]`): >= 30 out-of-sample trades (Trusted at >= 100 across regimes), profit factor >= 1.3 and expectancy >= 0.1 R after real costs and slippage, walk-forward out-of-sample/in-sample >= 0.5 with most windows profitable,
each tuned parameter +-20% keeps PF > 1.1 on a plateau, max drawdown (and Monte Carlo 95th) <= 10% of capital, positive across several stocks and sectors (not one), trial count recorded, corporate-action adjusted data. Backtest universe proposal: Nifty 50 plus Nifty Next 50 constituents from the instrument master, about five years of daily bars, regime and strategy params frozen **before** the test window is opened. Costs from `charges.toml` (verified_on recorded in results); slippage policy from F9.
Per-strategy results table (trades, win rate, PF, avg R, expectancy, max DD, walk-forward windows) is filled by dev-backtest in F9, not here.

## 7. Required Recommendation fields (F11 builds it; strategies supply)
Risk in rupees first, entry zone, stop, targets, horizon, `risk_rupees` incl. charges buffer, `reward_risk`, regime (market and stock) and fit sentence, `facts`, exit plan text, backtest stats and the `low_evidence` flag, event warning, "proven wrong if" line, disclaimer constant. Panel order per `bz-market-strategy` section 7. If the regime is outside fit: no trade panel stating what is missing ("market is sideways; this idea needs an uptrend").

## 8. Acceptance criteria for dev-quant
- [ ] Trait, extended `Signal`, `ExitAction` and `StrategyFactory`; params structs with `Default`, `validate()`, ranges; unknown config key is a load error; all three `enabled = false`
- [ ] Doc comment of each strategy copies idea/entry/exit/stop/regime/weaknesses from this file; catalog rows added by the manager or dev-quant
- [ ] Hand-computed fixtures per section 2 item 8; `lookahead_` test; tick-rounding and unadjusted-candle tests
- [ ] Zero threshold literals in strategy code; money in Decimal; indicator math in f64 converted at one boundary
- [ ] A signal never has stop >= entry, never widens a stop, never fires while the instrument is already held

## 9. Reject list (do not build in F8)
Candlestick-pattern-only or single-indicator entries (RSI cross, MACD cross alone); averaging down or adding to winners (pyramiding); martingale or bigger size after a loss; indicator ensembles or optimiser sweeps to find the "best" parameters; machine learning; fixed-percentage stops with no volatility basis;
no-stop mean reversion; trading Range x High or any Mixed/Unknown regime; short selling; options; intraday/ORB (ships disabled later; cost bar highest); entries on unadjusted candles; recommending any of these before G01 evidence; wording like "sure", "safe", "guaranteed".

## 10. Open questions
See the report to the manager (owner questions: G16 Rs 10,000 per-order cap vs 1% risk; scope confirmation; evidence-run universe and history; acceptance of UNVERIFIED regime defaults).
