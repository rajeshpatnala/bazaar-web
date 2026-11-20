# F9: Backtest engine + CostModel

Status: building (slice 0 of 8: types, config, record). Plan: `.agents/plans/F9-backtest-costmodel.md`. Policy: `docs/strategy/F9-backtest-policy.md`. Not investment advice: a backtest is evidence, not a promise.

Summary: run a strategy over fixture candles net of real charges and slippage, with a rolling walk-forward and a trial count. Owner area: `crates/backtest`, `config/backtest.toml`, `fixtures/{charges,backtest}`.

## Decisions
- GST base = brokerage + exchange txn + SEBI (settles QUAN-65-3/66-1); DP once per scrip per day on the first delivery sell, GST-inclusive (QUAN-66-2); STT and stamp whole rupee half-up; SEBI tested with 0.05 tolerance (rule UNVERIFIED); charges by trade date, no fallback unless `history_policy = latest_for_all` (flagged `costs_approximate`).
- Policy decisions (strategist, 2026-11-20), all in `config/backtest.toml` with `source` and `unverified = true`, never in code:
  - Q1 costs before 2026-11-20: confirm; `latest_for_all` is a per-run opt-in, such runs are "exploratory" and can never reach the evidence bar.
  - Q2 slippage: per class (equity max(0.05%, 1 tick) entry, max(0.10%, 1 tick) stop; index futures 1 / 2 ticks; index options max(2 ticks, 0.5%) / max(3 ticks, 1%)); an unknown class is a typed error; the evidence bar is re-run at 2x slippage and must keep expectancy above zero.
  - Q3 walk-forward: rolling, 24 months in-sample, 6 months out-of-sample, step 6 months, at least 4 folds, warm-up (250 bars) excluded; the trade minimum is pooled: at least 30 OOS trades across folds and instruments (equals risk.toml `min_oos_trades`, a hard floor); a fold under 5 trades is "thin" and left out of the profitable-folds vote. The plan's "30 per fold" is dropped.
  - Q4 lot/tick history: latest snapshot at or before the trade date; none = typed error.
  - Fills: stop long = `min(stop, open)`, short = `max(stop, open)`, then stop slippage (QUAN-77-8); stop wins a tie; a target fills at the target only if the bar trades through it by 1 tick.
  - Trials: a trial is a distinct (strategy id, full parameter set) evaluated in-sample; `prior_trials` is owner-declared; penalty on the profit factor bar: 2-10 trials +0.1, 11-50 +0.2 and label capped at Provisional, over 50 +0.3 and capped. Never loosens.
  - Evidence: pass needs the pooled gate, OOS/IS >= 0.5, >= 60% of non-thin folds profitable, no single fold supplying all profit, profit factor >= 1.3 and expectancy >= 0.1R after costs. Labels: Low evidence < 30 trades, Provisional 30-99, Trusted >= 100 over at least 2 regimes.
- Review round 1 (PR 83): every config value is also a floor (a looser value fails to load, a tighter one loads; floors in `config.rs` `mod floor`, the first three compared with `risk.toml [gate.evidence]` by a parsing test). `policy_date` replaces `verified_on` (policy date, not a source check). `latest_for_all` is refused in the file (per-run opt-in). `Leg` gained `LegInstrument` (cash, future, option call/put) and `Product::Carry`. `EvidenceLabel::Exploratory` for approximate-cost runs. `Trials` methods take the in-run count and add `prior_trials` (saturating); callers must not add it. Walk-forward: a position open at the in-sample end is force-closed or discarded, never carried into OOS (slice 6 tests entry and exit timestamps). Provenance gained universe, parameter_sets, trial_penalty_pf; its date fields are required in a finished result. The plan's AC 7 text ("per fold") still needs aligning with the pooled gate (planner).
- Config loader lives in `crates/backtest` (own typed errors); the floors (30 trades, PF 1.3, 0.1R) are checked against `risk.toml [gate.evidence]` by a test, so tightening is allowed and loosening fails to load.
- `LegSide` is a serde enum local to `backtest`; it is converted from `risk::types::Side` where the gate is called (slice 4). Types hold `Price`/`Qty` from core and `Decimal` rupees (serialised as strings).
- New dependencies for `backtest`: none outside the workspace set (`bazaar-core`, `chrono`, `chrono-tz`, `rust_decimal`, `serde`, `thiserror`, `toml`).

## Owner confirmations
UNVERIFIED conventions: slippage (all classes, the 2x stress), fold lengths (24m/6m/6m, 4 folds, thin-fold 5), the trial penalty tiers and the evidence label thresholds. Owner to confirm or change; tightening is allowed, loosening needs a `DECISIONS.md` entry. Older verified rate sets and lot snapshots remain an owner data task.

## Known limits
Calculator estimate, not a contract note; no delivery minimum; per-unit futures/options samples; history before 2026-11-20 has no verified rate set; survivorship and corporate-action adjustment UNVERIFIED unless the data source says so.

## What was built
Slice 0: `types.rs` (`Leg`, `ChargeBreakdown`, `Fill`, `Trade`, `EquityPoint`, `Provenance`, `HistoryPolicy`, `EvidenceLabel`, `BacktestError`), `config.rs` (`BacktestConfig::load`, validation with `BacktestConfigError`, trial penalty helper), `config/backtest.toml`.
Slice 1 (cost model): `cost/mod.rs` `CostModel::new(cfg, policy, exchange)`; `charges_with_dp(&mut DpLedger, &Leg)` is the engine entry (DP once per scrip per IST day on the first delivery sell); `charges_excl_dp` omits DP and is for tests/line checks only. Rounding: STT and stamp whole rupees, others 2 dp, half-up after the full multiplication; GST on brokerage + txn + SEBI only; IPFT folded into `exchange_txn`. Strict policy gives `NoRateSet`; `latest_for_all` takes the newest set and `leg_is_approximate` counts off-date legs. Zero quantity or price, a basis mismatch (options need `premium`) and a missing row are `Invalid`. Matches the 5 owner calculator samples within Rs 0.05 per line (fixture `fixtures/charges/calculator_2026-11-20.json`).

Slice 2 (fill model, `crates/backtest/src/fill`): next-open entry with optional no-chase zone limit, gap-aware stop (`min`/`max` with open, then stop slippage), target fill only when traded through by `target_through_ticks`, stop wins a same-bar tie, time exit, adverse tick rounding of slippage and of stop/target levels, stop/target side validation, slippage multiplier (2x stress hook), lot floor. Target fills use entry-class slippage with no gap credit: a conservative choice per policy section 1 (decided, keep). Not in this slice (slice 4, engine): AC4 peeking-double and `lookahead_` property tests, limit-entry fills, the no-`Fill`-with-default-charges test (QUAN-84-6), wiring `stress_slippage_multiplier` into the stress re-run (QUAN-84-3 hook exists as `FillParams::slippage_multiplier`).
Slice 3 (ledger): `ledger.rs` `Ledger::new(CostModel, capital)` owns one `DpLedger` (never share across runs); `apply(strategy_id, Fill, stop)` prices every fill via `charges_with_dp`, opens or fully closes a position (no pyramiding or partial exits; errors leave state untouched) and returns the `Trade` (gross after slippage, net after all charges, R to 4 dp, `None` when risk is 0); `square_off_intraday(date, exit_for)`; `mark` records net equity (capital + realised - open entry charges + unrealised); `max_drawdown` gives the largest rupee and percent fall.

Slice 3 fix round (PR 86 review): conservative conventions the engine must respect.
- Drawdown peak is seeded with starting capital (`max_drawdown(starting_equity, curve)`).
- Open positions are valued at gross unrealised minus entry charges; exit charges are NOT reserved (QUAN-86-5), so an open ledger is slightly optimistic. The engine must force-close at run end or report net of estimated exit charges.
- Marks carry forward per token over a missing bar; `mark` rejects time going backwards.
- Trades with no stop have `r_multiple = None` (QUAN-86-6): metrics must exclude them from R statistics and count them separately; the engine should reject stop-less entries where a stop is required.
- Opening validation: stop must be on the loss side; delivery is long only; an exit may not precede its entry; all arithmetic is checked.
- `square_off_intraday` stops at the first error; positions closed before it stay closed (CR-86-5). The slice exceeds the 400-line cap (CR-86-6), flagged only.
- New dev-dependency for the crate: `proptest` (workspace dep).

Slice 5 (metrics, provenance): `metrics.rs` `Metrics::from_trades` (all Decimal, no float: win rate, profit factor, expectancy R, net, drawdown from starting equity); trades with `r_multiple = None` are excluded from R stats and counted in `no_r_trades`. Profit factor with no losing trade is `None` and fails the bar (conservative). `assess` gives the label (Exploratory for approximate costs, Low below 30 pooled trades or `min_folds`, Trusted at 100 trades and 2 regimes unless the trial count caps it, else Provisional) and `meets_bar` (trade gate, folds, PF >= floor + trial penalty, expectancy); OOS/IS ratio and profitable-fold vote need per-fold data and come with slice 7. `provenance.rs`: SHA-256 config hashes, charges `verified_on` range over the whole file, `build` fills `Provenance`. New dependency: `sha2` (workspace dep, already used by bazaar-data).

## Deferred
Contract-note cross-check; QUAN-66-3 (delivery minimum); Monte Carlo (drawdown and stability are reported, not blocking).
Slice 4 must call `charges_with_dp` (one `DpLedger` per run, never shared across rayon runs) and test `dp == 15.34` on a delivery round trip; `[option_exercise]` STT (0.15% of intrinsic on exercised long options, QUAN-85-5) is deferred to the expiry/engine slice; the charges `verified_on` range for provenance is not yet exposed by `CostModel` (slice 4/7).
