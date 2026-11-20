//! Rolling walk-forward (policy `docs/strategy/F9-backtest-policy.md` sections 3 and 4).
//!
//! Folds are calendar-month windows over the bars, starting after `warmup_bars`: in-sample (IS)
//! then out-of-sample (OOS), stepping by `step_months` (never below the OOS length), so OOS windows
//! never overlap and no OOS bar is ever inside an IS slice. Per fold, each candidate parameter set
//! runs over the IS slice only; the best net P&L (ties: lowest index) is chosen, and only that
//! candidate runs over the OOS slice. Candidates count as trials. The caller's closure builds a
//! fresh strategy, `RunSpec` and `GatePolicy` for every window (a policy's counters are stale on
//! reuse). Each window is a fresh run, so indicators are rebuilt per window: a 6-month OOS window
//! is about 125 daily bars, shorter than the 250-bar `warmup_bars`, so a strategy that needs
//! more history than that trades nothing in OOS (a known limit; the run shows it as a thin fold).
//!
//! G01 evidence for an OOS fold is [`measurement_evidence`] of the chosen candidate's IS metrics:
//! same costs and slippage, window strictly before the first OOS decision bar. A candidate that
//! misses the profit factor or expectancy bar in IS gets no OOS run. IS runs get `None` evidence:
//! they use a selection-only policy the caller supplies, never the OOS gate.

use std::ops::Range;

use bazaar_core::candle::Candle;
use bazaar_risk::rules::inputs::EvidenceInputs;
use bazaar_risk::types::Tri;
use chrono::{Days, Months, NaiveDate};
use rust_decimal::Decimal;

use crate::config::BacktestConfig;
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
    InSample,
    OutOfSample,
}

/// One fold as bar index ranges: `is` ends where `oos` starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    pub is: Range<usize>,
    pub oos: Range<usize>,
}

/// Builds and runs one window: `(window, candidate index, that window's bars only, evidence)`.
/// `evidence` is `Some` for OOS only.
pub type RunWindow<'a> = dyn FnMut(Window, usize, &[Candle], Option<EvidenceInputs>) -> Result<RunOutput, BacktestError>
    + 'a;

/// Inputs outside the bars and config.
#[derive(Debug, Clone, Copy)]
pub struct WfParams {
    /// Starting capital of each window's run.
    pub capital: Decimal,
    /// Distinct parameter sets tried in IS (the trial count; at least 1).
    pub candidates: usize,
    /// Distinct regimes the OOS trades cover (the label needs this).
    pub regimes: u32,
    pub costs_approximate: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FoldResult {
    pub chosen: usize,
    pub is: Metrics,
    /// `None` when the chosen candidate missed the IS bar and no OOS run was made.
    pub oos: Option<Metrics>,
    /// Fewer OOS trades than `thin_fold_trades`: shown, left out of the profitable-folds vote.
    pub thin: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WalkForwardResult {
    pub folds: Vec<FoldResult>,
    pub oos_trades: Vec<Trade>,
    /// Pooled OOS metrics. Drawdown is zero: fold equity curves restart at capital.
    pub pooled: Metrics,
    pub assessment: Assessment,
    pub trials: u32,
    pub gate_evidence: &'static str,
}

/// G01 evidence for measuring a candidate in the OOS window from its IS metrics.
///
/// `walk_forward_pass` is set `Known(true)` HERE and nowhere else: it is measurement context only
/// (an OOS fold cannot pass a walk-forward it is part of). The real verdict is computed after the
/// fact from pooled OOS trades (policy section 3), never from this flag. Never reuse this for a
/// live or scanner gate. Unmeasurable figures stay `Unknown`, so the gate fails closed.
#[must_use]
pub fn measurement_evidence(is: &Metrics) -> EvidenceInputs {
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

/// The rolling folds over `bars` (sorted), starting at `bars[warmup_bars]`. Only whole windows
/// count: a fold whose OOS window runs past the data is dropped.
///
/// # Errors
/// `Invalid` when the step is below the OOS length (OOS windows would overlap).
pub fn split(bars: &[Candle], cfg: &BacktestConfig) -> Result<Vec<Fold>, BacktestError> {
    let w = &cfg.walk_forward;
    if w.step_months < w.out_of_sample_months || w.step_months == 0 {
        return Err(BacktestError::Invalid(
            "walk-forward step must be at least the OOS length".into(),
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

/// Run every fold through `run`. See the module doc for the flow.
///
/// # Errors
/// `Invalid` for no candidates or a bad step; any error from `run`; `Overflow`.
pub fn walk_forward(
    cfg: &BacktestConfig,
    bars: &[Candle],
    p: &WfParams,
    run: &mut RunWindow<'_>,
) -> Result<WalkForwardResult, BacktestError> {
    if p.candidates == 0 {
        return Err(BacktestError::Invalid("at least one candidate".into()));
    }
    let trials =
        u32::try_from(p.candidates).map_err(|_| BacktestError::Overflow("candidate count"))?;
    let required_pf = cfg
        .evidence
        .min_profit_factor
        .checked_add(cfg.trials.profit_factor_penalty(trials))
        .ok_or(BacktestError::Overflow("required profit factor"))?;
    let folds = split(bars, cfg)?;
    let (mut results, mut pooled) = (Vec::new(), Vec::new());
    for fold in &folds {
        let is_bars = bars.get(fold.is.clone()).unwrap_or_default();
        let oos_bars = bars.get(fold.oos.clone()).unwrap_or_default();
        let mut best: Option<(usize, Metrics)> = None;
        for c in 0..p.candidates {
            let out = run(Window::InSample, c, is_bars, None)?;
            let m = Metrics::from_trades(&out.trades, p.capital, &out.equity)?;
            if best.as_ref().is_none_or(|(_, b)| m.net_pnl > b.net_pnl) {
                best = Some((c, m));
            }
        }
        let Some((chosen, is)) = best else { continue };
        let bar_met = is.profit_factor.is_some_and(|pf| pf >= required_pf)
            && is
                .expectancy_r
                .is_some_and(|e| e >= cfg.evidence.min_expectancy_r);
        let oos = if bar_met {
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
            .is_none_or(|m| m.trades < cfg.walk_forward.thin_fold_trades);
        results.push(FoldResult {
            chosen,
            is,
            oos,
            thin,
        });
    }
    let pooled_m = Metrics::from_trades(&pooled, p.capital, &[])?;
    let facts = RunFacts {
        folds: u32::try_from(results.len()).map_err(|_| BacktestError::Overflow("fold count"))?,
        regimes: p.regimes,
        costs_approximate: p.costs_approximate,
        trials_in_run: trials,
    };
    Ok(WalkForwardResult {
        folds: results,
        oos_trades: pooled,
        assessment: assess(cfg, &pooled_m, &facts)?,
        pooled: pooled_m,
        trials: cfg.trials.total_trials(trials),
        gate_evidence: GATE_EVIDENCE,
    })
}

#[cfg(test)]
mod tests;
