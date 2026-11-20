//! The per-fold parts of the evidence bar: OOS/IS ratio, profitable-fold vote and "no single fold
//! supplies all the profit" (`docs/features/F9-backtest-costmodel.md`, Q3). `assess` covers the
//! pooled trade gate, profit factor and expectancy; this covers what needs per-fold data.
//!
//! Folds skipped in IS (no OOS run) count as failures in the vote: they stay in the denominator,
//! so a run cannot look good by reporting only the folds that happened to qualify. Thin folds
//! (an OOS run with fewer R trades than `thin_fold_trades`) are shown but left out of the vote, as
//! the config says. The 2x slippage stress is not here.

use rust_decimal::{Decimal, RoundingStrategy};

use crate::config::BacktestConfig;
use crate::types::BacktestError;
use crate::walkforward::FoldResult;

/// The per-fold verdicts. Each `*_ok` is false when its figure is undefined (fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldVote {
    /// Folds in the vote: all folds except those with a thin OOS run.
    pub voting_folds: u32,
    /// Voting folds with an OOS run and positive net P&L.
    pub profitable_folds: u32,
    /// Folds skipped in IS (counted in `voting_folds` as failures).
    pub skipped_folds: u32,
    /// `profitable_folds / voting_folds` in percent, 4 dp toward zero; `None` with no voting fold.
    pub profitable_pct: Option<Decimal>,
    /// `profitable_pct >= min_profitable_fold_pct` (exact, not on the rounded figure) AND
    /// `voting_folds >= min_folds`: thin folds are left out of the vote, so they cannot supply the
    /// fold minimum either.
    pub vote_ok: bool,
    /// Mean OOS R over mean IS R across non-thin folds with an OOS run, weighted by R trades, 4 dp toward
    /// zero. `None` when there is no such fold or the IS mean is not above zero.
    pub oos_is_ratio: Option<Decimal>,
    /// `oos_is_ratio >= min_oos_is_ratio`.
    pub ratio_ok: bool,
    /// More than one voting fold made a profit: no single fold supplies ALL of it. This is the
    /// literal reading; it does not cap one fold's share of the profit (a config share cap would
    /// be stricter and needs an owner decision).
    pub single_fold_ok: bool,
}

impl FoldVote {
    /// True when every per-fold check holds (still needs `assess` and the slippage stress).
    #[must_use]
    pub const fn all_ok(&self) -> bool {
        self.vote_ok && self.ratio_ok && self.single_fold_ok
    }
}

/// R-weighted mean of `(mean R, r trades)` pairs; `None` with no R trades.
fn weighted_mean(parts: &[(Decimal, u32)]) -> Result<Option<Decimal>, BacktestError> {
    let over = || BacktestError::Overflow("fold R mean");
    let (mut sum, mut n) = (Decimal::ZERO, Decimal::ZERO);
    for (m, k) in parts {
        let k = Decimal::from(*k);
        sum = m
            .checked_mul(k)
            .and_then(|x| sum.checked_add(x))
            .ok_or_else(over)?;
        n = n.checked_add(k).ok_or_else(over)?;
    }
    if n.is_zero() {
        return Ok(None);
    }
    Ok(Some(sum.checked_div(n).ok_or_else(over)?))
}

/// Vote over every fold of a walk-forward run.
///
/// # Errors
/// `Overflow` if a sum or ratio does not fit.
pub fn vote(cfg: &BacktestConfig, folds: &[FoldResult]) -> Result<FoldVote, BacktestError> {
    let over = |w: &'static str| BacktestError::Overflow(w);
    let ran = |f: &&FoldResult| f.oos.is_some();
    let voting: Vec<&FoldResult> = folds
        .iter()
        .filter(|f| !(f.oos.is_some() && f.thin))
        .collect();
    let profitable = voting
        .iter()
        .filter(|f| f.oos.as_ref().is_some_and(|m| m.net_pnl > Decimal::ZERO))
        .count();
    let to_u32 = |n: usize| u32::try_from(n).map_err(|_| over("fold count"));
    let (voting_folds, profitable_folds) = (to_u32(voting.len())?, to_u32(profitable)?);
    let skipped_folds = to_u32(folds.iter().filter(|f| f.skipped_in_is).count())?;
    let (den, num) = (Decimal::from(voting_folds), Decimal::from(profitable_folds));
    let profitable_pct = num
        .checked_mul(Decimal::ONE_HUNDRED)
        .and_then(|x| x.checked_div(den))
        .map(|p| p.round_dp_with_strategy(4, RoundingStrategy::ToZero));
    let vote_ok = voting_folds > 0
        && voting_folds >= cfg.walk_forward.min_folds
        && num.checked_mul(Decimal::ONE_HUNDRED).is_some_and(|lhs| {
            cfg.evidence
                .min_profitable_fold_pct
                .checked_mul(den)
                .is_some_and(|rhs| lhs >= rhs)
        });
    let (mut is_parts, mut oos_parts) = (Vec::new(), Vec::new());
    // Thin folds are out of the ratio as well (one rule for the whole per-fold verdict).
    for f in folds.iter().filter(ran).filter(|f| !f.thin) {
        if let (Some(i), Some(Some(o))) =
            (f.is.expectancy_r, f.oos.as_ref().map(|m| m.expectancy_r))
        {
            is_parts.push((i, f.is.r_trades));
            let o_trades = f.oos.as_ref().map_or(0, |m| m.r_trades);
            oos_parts.push((o, o_trades));
        }
    }
    let oos_is_ratio = match (weighted_mean(&is_parts)?, weighted_mean(&oos_parts)?) {
        (Some(i), Some(o)) if i > Decimal::ZERO => o
            .checked_div(i)
            .map(|r| r.round_dp_with_strategy(4, RoundingStrategy::ToZero)),
        _ => None,
    };
    let ratio_ok = oos_is_ratio.is_some_and(|r| r >= cfg.evidence.min_oos_is_ratio);
    Ok(FoldVote {
        voting_folds,
        profitable_folds,
        skipped_folds,
        profitable_pct,
        vote_ok,
        oos_is_ratio,
        ratio_ok,
        // Counted over voting folds only: a thin fold's lone profit is not evidence.
        single_fold_ok: profitable_folds > 1,
    })
}

#[cfg(test)]
mod tests;
