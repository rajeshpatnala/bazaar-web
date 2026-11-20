//! The base run and its 2x slippage stress re-run, with the charges evidence of BOTH.
//!
//! One `CostModel` (the spec's) runs the engine and builds the [`ChargesEvidence`], so the legs
//! that were charged and the policy they were charged under cannot disagree (QUAN-94-8).

use super::{EntryPolicy, RegimeProvider, RunOutput, RunSpec, run};
use crate::provenance::ChargesEvidence;
use crate::types::BacktestError;
use bazaar_strategies::Strategy;
use rust_decimal::Decimal;

/// A fresh strategy, regime provider and policy for one run (policies are stateful, so each run
/// gets its own).
pub struct RunParts<'a> {
    pub strategy: Box<dyn Strategy + 'a>,
    pub regimes: Box<dyn RegimeProvider + 'a>,
    pub policy: Box<dyn EntryPolicy + 'a>,
}

/// The base run, the stress run and what both were charged with.
#[derive(Debug, Clone, PartialEq)]
pub struct StressedRun {
    /// Slippage multiplier 1.
    pub base: RunOutput,
    /// `fills.stress_slippage_multiplier` (2x).
    pub stress: RunOutput,
    /// The range over EVERY leg of both runs, and the cost model's history policy.
    pub charges: ChargesEvidence,
}

impl StressedRun {
    /// Net P&L of the stress run minus the base run (not above zero: stress only costs more).
    ///
    /// # Errors
    /// `Overflow` on the sum.
    pub fn stress_pnl_change(&self) -> Result<Decimal, BacktestError> {
        let sum = |o: &RunOutput| {
            o.trades.iter().try_fold(Decimal::ZERO, |a, t| {
                a.checked_add(t.net_pnl)
                    .ok_or(BacktestError::Overflow("net pnl sum"))
            })
        };
        sum(&self.stress)?
            .checked_sub(sum(&self.base)?)
            .ok_or(BacktestError::Overflow("stress pnl change"))
    }
}

/// Run `make()`'s parts at multiplier 1, then again at the stress multiplier. The spec's own
/// `slippage_multiplier` is ignored: this runner owns both.
///
/// # Errors
/// As [`run`].
pub fn run_with_stress<'a>(
    spec: &RunSpec<'a>,
    make: &mut dyn FnMut() -> RunParts<'a>,
) -> Result<StressedRun, BacktestError> {
    let at = |m: Decimal| RunSpec {
        slippage_multiplier: m,
        ..*spec
    };
    let mut once = |m: Decimal| {
        let mut p = make();
        run(&at(m), &mut *p.strategy, &*p.regimes, &mut *p.policy)
    };
    let base = once(Decimal::ONE)?;
    let stress = once(spec.config.stress_multiplier())?;
    let legs = base
        .trades
        .iter()
        .chain(&stress.trades)
        .flat_map(|t| [&t.entry.leg, &t.exit.leg]);
    let charges = ChargesEvidence::from_legs(&spec.cost, legs)?;
    Ok(StressedRun {
        base,
        stress,
        charges,
    })
}

impl std::fmt::Debug for RunParts<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunParts").finish_non_exhaustive()
    }
}
