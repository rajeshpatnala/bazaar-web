//! Rolling walk-forward (policy `docs/strategy/F9-backtest-policy.md` sections 3 and 4).
//!
//! Folds are calendar-month windows over the bars, starting after `warmup_bars`: in-sample (IS)
//! then out-of-sample (OOS), stepping by `step_months` (never below the OOS length), so OOS windows
//! never overlap and no OOS bar is ever inside an IS slice. Per fold, each parameter set runs over
//! the IS slice only; only a set that clears the IS bar can be chosen and run over the OOS slice.
//! The caller's closure builds a fresh strategy, `RunSpec` and `GatePolicy` for every window (a
//! policy's counters are stale on reuse). Each window is a fresh run, so indicators are rebuilt per
//! window: a 6-month OOS window is about 125 daily bars, shorter than the 250-bar `warmup_bars`, so
//! a strategy that needs more history than that trades nothing in OOS (a known limit; the run shows
//! it as a thin fold).
//!
//! G01 evidence for an OOS fold is `measurement_evidence` of the chosen set's IS metrics: same
//! costs and slippage, window strictly before the first OOS decision bar. IS runs get `None`
//! evidence: they use a selection-only policy the caller supplies, never the OOS gate; IS metrics
//! are only for selection and for the OOS evidence figures, and IS trades are never pooled. A
//! looser IS policy inflates those figures (slice 7 defines it as the real `GatePolicy`).
//!
//! Selection: max IS net P&L among sets clearing the IS bar (profit factor with the trial penalty,
//! and expectancy); ties go to the lowest index. A thin IS pick cannot pass the G01 trade-count
//! evidence in OOS, and no extra minimum-trades rule exists beyond it. A fold where no set clears
//! the bar is skipped (`skipped_in_is`) and does not count toward `min_folds`. The trial count is
//! the distinct sets declared in `WfParams::sets`.

use std::ops::Range;

use bazaar_core::candle::Candle;
use bazaar_risk::rules::inputs::EvidenceInputs;
use bazaar_risk::types::Tri;
use chrono::{Days, Months, NaiveDate};
use rust_decimal::Decimal;

use crate::config::BacktestConfig;
use crate::cost::CostModel;
use crate::engine::RunOutput;
use crate::fill::trade_date;
use crate::metrics::{Assessment, Metrics, RunFacts, assess};
use crate::types::{BacktestError, Trade};

/// What the provenance records about the gate's evidence in a walk-forward run.
pub const GATE_EVIDENCE: &str = "chosen-candidate IS window; walk_forward_pass assumed true \
    (measurement context only; the verdict comes from pooled OOS trades)";

/// Which slice a run is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// The in-sample slice (candidate selection; no gate evidence).
    InSample,
    /// The out-of-sample slice (only the chosen set; gate evidence from IS).
    OutOfSample,
}

/// One fold as bar index ranges: `is` ends where `oos` starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    /// In-sample bar indices.
    pub is: Range<usize>,
    /// Out-of-sample bar indices (never overlaps another fold's OOS).
    pub oos: Range<usize>,
}

/// One parameter set tried in IS: a strategy id and its parameters in a stable text form.
/// Two sets with the same `(id, params)` are the same trial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSet {
    /// Strategy id.
    pub id: String,
    /// Canonical parameter text.
    pub params: String,
}

/// The distinct sets in first-seen order. The closure's candidate index and
/// `WalkForwardResult::param_sets` both refer to this list.
#[must_use]
pub fn distinct_sets(sets: &[ParamSet]) -> Vec<ParamSet> {
    let mut out: Vec<ParamSet> = Vec::new();
    for s in sets {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

/// Builds and runs one window: `(window, index into the distinct sets, that window's bars only,
/// evidence)`. `evidence` is `Some` for OOS only. The closure must read nothing but the bars it is
/// given (it can capture the full series; a caller-side leak test is the guard, see slice 7).
pub type RunWindow<'a> = dyn FnMut(Window, usize, &[Candle], Option<EvidenceInputs>) -> Result<RunOutput, BacktestError>
    + 'a;

/// Inputs outside the bars and config.
#[derive(Debug, Clone, Copy)]
pub struct WfParams<'a> {
    /// Starting capital of each window's run.
    pub capital: Decimal,
    /// Every parameter set the closure may run in IS. Duplicates count once; the distinct count is
    /// the trial count (the closure must run exactly these sets, nothing hidden behind an index).
    pub sets: &'a [ParamSet],
    /// Distinct regimes the OOS trades cover. Caller input until slice 7 derives it from
    /// per-trade regime tags; it can lift the label, so treat it as unverified.
    pub regimes: u32,
    /// The cost model of the runs; `costs_approximate` comes from it, never the caller.
    pub cost: CostModel<'a>,
}

/// One fold's outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct FoldResult {
    /// Index of the chosen set (best IS net P&L among sets clearing the IS bar; if none clears,
    /// the best IS net P&L overall, shown but not run OOS).
    pub chosen: usize,
    /// The chosen set's IS metrics.
    pub is: Metrics,
    /// `None` when no set cleared the IS bar and no OOS run was made.
    pub oos: Option<Metrics>,
    /// No set cleared the IS bar, so there is no OOS run. Slice 7's profitable-fold vote must count
    /// these as failures, never drop them from the denominator.
    pub skipped_in_is: bool,
    /// Meaningful only when `oos` is `Some`: fewer R trades than `thin_fold_trades` (shown, left
    /// out of the profitable-folds vote). Always false for a skipped fold.
    pub thin: bool,
}

/// Pooled OOS metrics. No drawdown: fold equity curves restart at capital, so a chained curve
/// (slice 7) is needed and a zero would be a fabricated number.
#[derive(Debug, Clone, PartialEq)]
pub struct PooledMetrics {
    /// Pooled OOS trades.
    pub trades: u32,
    /// Trades with an R multiple.
    pub r_trades: u32,
    /// Net P&L after costs and slippage.
    pub net_pnl: Decimal,
    /// Pooled profit factor (`None` with no losing trade).
    pub profit_factor: Option<Decimal>,
    /// Pooled mean R (`None` with no R trades).
    pub expectancy_r: Option<Decimal>,
}

/// The whole walk-forward run.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkForwardResult {
    /// One entry per fold, including those skipped in IS.
    pub folds: Vec<FoldResult>,
    /// Pooled OOS trades (only folds with an OOS run).
    pub oos_trades: Vec<Trade>,
    /// Pooled OOS metrics, without drawdown.
    pub pooled: PooledMetrics,
    /// Label and the partial bar (`folds` counts only folds with an OOS run).
    pub assessment: Assessment,
    /// Declared prior trials plus the distinct sets of this run.
    pub trials: u32,
    /// The distinct parameter sets tried (the closure's candidate index points into this).
    pub param_sets: Vec<ParamSet>,
    /// Provenance text on the gate evidence assumption.
    pub gate_evidence: &'static str,
}

/// G01 evidence for measuring a candidate in the OOS window from its IS metrics.
///
/// `walk_forward_pass` is set `Known(true)` HERE and nowhere else: it is measurement context only
/// (an OOS fold cannot pass a walk-forward it is part of). The real verdict is computed after the
/// fact from pooled OOS trades (policy section 3), never from this flag. Crate-private on purpose:
/// never reuse this for a live or scanner gate. Unmeasurable figures stay `Unknown`, so the gate
/// fails closed. The IS trade count is passed as `oos_trades`, so a G01 rejection line reading
/// "N out-of-sample trades" counts IS trades here.
#[must_use]
pub(crate) fn measurement_evidence(is: &Metrics) -> EvidenceInputs {
    EvidenceInputs {
        oos_trades: Tri::Known(is.r_trades),
        profit_factor: is.profit_factor.map_or(Tri::Unknown, Tri::Known),
        expectancy_r: is.expectancy_r.map_or(Tri::Unknown, Tri::Known),
        walk_forward_pass: Tri::Known(true),
    }
}

fn first_at_or_after(bars: &[Candle], day: NaiveDate) -> usize {
    bars.partition_point(|b| trade_date(b.close_time()) < day)
}

/// The rolling folds over `bars`, starting at `bars[warmup_bars]`. Only whole windows count: a
/// fold whose OOS calendar window runs past the last bar's date is dropped (so a window ending on
/// a weekend or holiday after the last bar loses its fold: conservative).
///
/// # Errors
/// `Invalid` when the step is below the OOS length (OOS windows would overlap) or the bars are not
/// strictly increasing in time.
pub fn split(bars: &[Candle], cfg: &BacktestConfig) -> Result<Vec<Fold>, BacktestError> {
    let w = &cfg.walk_forward;
    if w.step_months < w.out_of_sample_months || w.step_months == 0 {
        return Err(BacktestError::Invalid(
            "walk-forward step must be at least the OOS length".into(),
        ));
    }
    if bars
        .windows(2)
        .any(|p| p[0].close_time() >= p[1].close_time())
    {
        return Err(BacktestError::Invalid(
            "bars must be strictly increasing in time".into(),
        ));
    }
    let warm = usize::try_from(w.warmup_bars).unwrap_or(usize::MAX);
    let (Some(first), Some(last)) = (bars.get(warm), bars.last()) else {
        return Ok(Vec::new());
    };
    let (start, end) = (
        trade_date(first.close_time()),
        trade_date(last.close_time()),
    );
    let mut folds = Vec::new();
    for k in 0_u32.. {
        let at =
            |months: Option<u32>| months.and_then(|m| start.checked_add_months(Months::new(m)));
        let off = k.checked_mul(w.step_months);
        let is_start = at(off);
        let is_end = at(off.and_then(|o| o.checked_add(w.in_sample_months)));
        let oos_end = at(off
            .and_then(|o| o.checked_add(w.in_sample_months))
            .and_then(|o| o.checked_add(w.out_of_sample_months)));
        // The OOS window must lie fully inside the data (its last day is the day before `oos_end`).
        let (Some(a), Some(b), Some(c)) = (is_start, is_end, oos_end) else {
            break;
        };
        if c.checked_sub_days(Days::new(1)).is_none_or(|d| d > end) {
            break;
        }
        let (i0, i1, i2) = (
            first_at_or_after(bars, a),
            first_at_or_after(bars, b),
            first_at_or_after(bars, c),
        );
        folds.push(Fold {
            is: i0..i1,
            oos: i1..i2,
        });
    }
    Ok(folds)
}

fn window<'a>(bars: &'a [Candle], r: &Range<usize>) -> Result<&'a [Candle], BacktestError> {
    bars.get(r.clone())
        .ok_or_else(|| BacktestError::Invalid("fold range outside the bars".into()))
}

fn bar_met(cfg: &BacktestConfig, m: &Metrics, required_pf: Decimal) -> bool {
    m.profit_factor.is_some_and(|pf| pf >= required_pf)
        && m.expectancy_r
            .is_some_and(|e| e >= cfg.evidence.min_expectancy_r)
}

/// Max net P&L among the entries (optionally only those clearing the bar); ties: lowest index.
fn pick(all: &[(usize, Metrics)], keep: impl Fn(&Metrics) -> bool) -> Option<&(usize, Metrics)> {
    let mut best: Option<&(usize, Metrics)> = None;
    for e in all {
        if keep(&e.1) && best.is_none_or(|b| e.1.net_pnl > b.1.net_pnl) {
            best = Some(e);
        }
    }
    best
}

/// Run every fold through `run`. See the module doc for the flow.
///
/// # Errors
/// `Invalid` for no parameter sets, a bad step or unsorted bars; any error from `run`; `Overflow`.
pub fn walk_forward(
    cfg: &BacktestConfig,
    bars: &[Candle],
    p: &WfParams<'_>,
    run: &mut RunWindow<'_>,
) -> Result<WalkForwardResult, BacktestError> {
    let sets = distinct_sets(p.sets);
    if sets.is_empty() {
        return Err(BacktestError::Invalid("at least one parameter set".into()));
    }
    let trials =
        u32::try_from(sets.len()).map_err(|_| BacktestError::Overflow("parameter set count"))?;
    let required_pf = cfg
        .evidence
        .min_profit_factor
        .checked_add(cfg.trials.profit_factor_penalty(trials))
        .ok_or(BacktestError::Overflow("required profit factor"))?;
    let folds = split(bars, cfg)?;
    let (mut results, mut pooled) = (Vec::new(), Vec::new());
    for fold in &folds {
        let is_bars = window(bars, &fold.is)?;
        let oos_bars = window(bars, &fold.oos)?;
        let mut all: Vec<(usize, Metrics)> = Vec::new();
        for c in 0..sets.len() {
            let out = run(Window::InSample, c, is_bars, None)?;
            all.push((
                c,
                Metrics::from_trades(&out.trades, p.capital, &out.equity)?,
            ));
        }
        let met = pick(&all, |m| bar_met(cfg, m, required_pf));
        let Some((chosen, is)) = met.or_else(|| pick(&all, |_| true)).cloned() else {
            continue;
        };
        let oos = if met.is_some() {
            let ev = Some(measurement_evidence(&is));
            let out = run(Window::OutOfSample, chosen, oos_bars, ev)?;
            let m = Metrics::from_trades(&out.trades, p.capital, &out.equity)?;
            pooled.extend(out.trades);
            Some(m)
        } else {
            None
        };
        let thin = oos
            .as_ref()
            .is_some_and(|m| m.r_trades < cfg.walk_forward.thin_fold_trades);
        results.push(FoldResult {
            chosen,
            is,
            skipped_in_is: oos.is_none(),
            oos,
            thin,
        });
    }
    let all_m = Metrics::from_trades(&pooled, p.capital, &[])?;
    let ran = results.iter().filter(|f| f.oos.is_some()).count();
    let facts = RunFacts {
        folds: u32::try_from(ran).map_err(|_| BacktestError::Overflow("fold count"))?,
        regimes: p.regimes,
        costs_approximate: p.cost.costs_approximate(),
        trials_in_run: trials,
    };
    Ok(WalkForwardResult {
        folds: results,
        oos_trades: pooled,
        assessment: assess(cfg, &all_m, &facts)?,
        pooled: PooledMetrics {
            trades: all_m.trades,
            r_trades: all_m.r_trades,
            net_pnl: all_m.net_pnl,
            profit_factor: all_m.profit_factor,
            expectancy_r: all_m.expectancy_r,
        },
        trials: cfg.trials.total_trials(trials),
        param_sets: sets,
        gate_evidence: GATE_EVIDENCE,
    })
}

#[cfg(test)]
mod tests;
