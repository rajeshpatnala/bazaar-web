# F9 backtest policy (strategist decisions, 2026-11-20)

Status: owner-policy defaults, UNVERIFIED conventions, not market facts. They go in `config/backtest.toml` with `source = "owner policy"`, never in code.
Tightening is always allowed; loosening needs a `DECISIONS.md` entry and the owner. A backtest is evidence, not a promise: no result here means a trade will make money.

## Decisions on the plan's open questions (one line each)
- **Q1 costs before 2026-11-20: CONFIRM.** `latest_for_all` is per-run opt-in, stamps `costs_approximate = true`, and such runs can never reach the evidence bar (label "exploratory").
- **Q2 slippage: AMEND** (table below): add a 1-tick floor, a harsher stop-exit figure, and a 2x stress test.
- **Q3 walk-forward: AMEND.** Rolling 24m in-sample / 6m out-of-sample, step 6m; trade minimum is pooled, not per fold (section 3).
- **Q4 lot/tick history: CONFIRM.** Latest snapshot at or before the trade date; none = typed error. Equity-only runs are unaffected (lot = 1); F&O runs outside the snapshot range need owner-supplied snapshots.

## 1. Slippage (adverse to the trader, always, then rounded against the trader to the tick)
| Class | Entry / target / time-exit | Stop exit | Why |
|---|---|---|---|
| Equity (liquid watchlist) | max(0.05% of price, 1 tick) | max(0.10% of price, 1 tick) | stops fill as market orders in a fast move |
| Index futures | 1 tick | 2 ticks | tight spreads, but stops still slip |
| Index options (bought, defined risk) | max(2 ticks, 0.5% of premium) | max(3 ticks, 1% of premium) | option spreads are wider than the underlying's |
- Keys: `[slippage.<class>] entry_pct, entry_min_ticks, stop_pct, stop_min_ticks`. Unknown class = typed error, never a zero default.
- Rupee check at the owner's limits (Rs 1,00,000, 1% = Rs 1,000 risk, position cap 25% = Rs 25,000): entry Rs 12.50 + stop exit Rs 25 = about Rs 37.50, roughly 4% of one trade's risk budget, before charges. Real slippage can be larger in thin names or on result days.
- Stress rule: the evidence bar is evaluated again at 2x slippage and must keep expectancy above zero (a cost-misestimation test). Report both.

## 2. Fill rules (modelling requirements)
- Entry: signal on bar `i` close, fill at bar `i+1` open plus entry slippage. If the open is above the entry zone top (G10 tolerance is 0), no fill and the signal expires; never chase.
- **Stop fills (QUAN-77-8): long stop fill = `min(stop, open)`; short stop = `max(stop, open)`.** A gap through the stop fills at the open (never at the stop price), then stop slippage is applied. A stop with no gap fills at the stop, then stop slippage.
- Stop and target both inside one bar: the stop wins. A target (limit) fills only if the bar trades through it by at least 1 tick, and fills at the target, never at a better gap open (no free credit).
- Charges are taken on every leg from `CostModel`; P&L, R-multiples and equity are always net.

## 3. Walk-forward and the minimum-trades gate
- Style: **rolling** (not anchored), IS 24 months, OOS 6 months, step 6 months, folds never overlap OOS windows. Reason: F8 strategies are daily-bar swing systems with a handful of trades per instrument per quarter, so the plan's 12m/3m folds would be mostly empty; rolling keeps old regimes from dominating. Indicator warm-up (about 250 bars) is excluded from IS.
- Minimum folds: 4 (about 5 years of daily data including warm-up). Fewer = "low evidence", never "pass".
- Trades gate: **>= 30 pooled OOS trades across all folds and instruments** (matches `min_oos_trades` in risk.toml; the hard floor, not lowerable here). The plan's "30 per OOS fold" is dropped: unreachable for swing systems and would label every fold insufficient. Per fold: fewer than 5 trades = fold "thin", shown but excluded from the profitable-folds vote.
- Pass requires: pooled gate met; OOS/IS performance ratio >= 0.5; >= 60% of non-thin folds profitable; no single fold supplies all the profit; profit factor >= 1.3 and expectancy >= 0.1R after costs (the risk.toml `[gate.evidence]` values). Labels: Low evidence < 30, Provisional 30-99, Trusted >= 100 across at least 2 regimes.
- F7/F8 parameters are fixed proposals, not tuned on P&L: with no grid the run is a rolling out-of-sample stability test and trials = 1.

## 4. Trial counting (stops "try seven variants, keep the best")
- A trial = one distinct (strategy id, full parameter set) evaluated on in-sample data; counted across all folds of a run. Result carries `trials` and the list of parameter sets.
- A parameter choice made after looking at an OOS window burns that window: re-running means a new run, trials restart at the cumulative count the owner declares in `prior_trials` (config key, default 0; the strategy spec also records variants tried).
- Penalty (raises the bar, tightening only): trials 1 = none; 2-10 = profit factor bar +0.1; 11-50 = +0.2 and the label is capped at Provisional; > 50 = +0.3, capped at Provisional. A grid search result is never "Trusted" without a fresh, untouched OOS period.
- Reported, not hidden: trials and penalty appear in provenance and the Why panel.

## 5. Evidence bar for the backtest engine (all after costs and slippage)
The bz-market-strategy table applies unchanged. Extra F9 conditions: `costs_approximate` runs fail; parameter stability (+-20% keeps PF > 1.1) and drawdown <= 10% of capital are reported from slice 7 onward, but not blocking until a Monte Carlo exists (out of scope). Survivorship and corporate-action adjustment are noted as UNVERIFIED unless the data source says otherwise.

## Open for the owner
Slippage and fold lengths are conventions; confirm or change. Older verified rate sets and lot snapshots remain an owner data task.
