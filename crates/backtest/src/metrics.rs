//! Net metrics, R statistics and the evidence label (policy `docs/strategy/F9-backtest-policy.md`).
//!
//! Everything is `Decimal`; no float is needed (sums, ratios and means only). Every figure uses
//! net P&L (after slippage and all charges). A trade with `r_multiple = None` (no stop, zero risk)
//! is left out of the R statistics and counted in `no_r_trades`; it still counts in P&L, win rate
//! and profit factor. A backtest is evidence, not a promise.

use rust_decimal::{Decimal, RoundingStrategy};

use crate::config::BacktestConfig;
use crate::ledger::{Drawdown, max_drawdown};
use crate::types::{BacktestError, EquityPoint, EvidenceLabel, Trade};

fn ov(what: &'static str) -> BacktestError {
    BacktestError::Overflow(what)
}

fn round4(d: Decimal) -> Decimal {
    d.round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero)
}

/// Summary of a set of closed trades.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metrics {
    pub trades: u32,
    /// Trades with net P&L above zero (a flat trade is neither a win nor a loss).
    pub wins: u32,
    pub losses: u32,
    /// `wins / trades`, 4 dp; zero with no trades.
    pub win_rate: Decimal,
    pub net_pnl: Decimal,
    /// Sum of winning nets (>= 0).
    pub gross_profit: Decimal,
    /// Absolute sum of losing nets (>= 0).
    pub gross_loss: Decimal,
    /// `gross_profit / gross_loss`, 4 dp; `None` with no losing trade (undefined, never treated
    /// as passing: `assess` fails the bar on `None`).
    pub profit_factor: Option<Decimal>,
    /// Mean R over trades that have one, 4 dp; `None` when none do.
    pub expectancy_r: Option<Decimal>,
    pub r_trades: u32,
    /// Trades excluded from R stats because `r_multiple` is `None`.
    pub no_r_trades: u32,
    pub drawdown: Drawdown,
}

impl Metrics {
    /// # Errors
    /// `Overflow` on a sum or ratio that does not fit.
    pub fn from_trades(
        trades: &[Trade],
        starting_equity: Decimal,
        curve: &[EquityPoint],
    ) -> Result<Self, BacktestError> {
        let (mut wins, mut losses, mut r_trades) = (0_u32, 0_u32, 0_u32);
        let (mut net, mut profit, mut loss, mut r_sum) =
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
        for t in trades {
            net = net.checked_add(t.net_pnl).ok_or_else(|| ov("net pnl"))?;
            if t.net_pnl > Decimal::ZERO {
                wins += 1;
                profit = profit
                    .checked_add(t.net_pnl)
                    .ok_or_else(|| ov("gross profit"))?;
            } else if t.net_pnl < Decimal::ZERO {
                losses += 1;
                loss = loss
                    .checked_sub(t.net_pnl)
                    .ok_or_else(|| ov("gross loss"))?;
            }
            if let Some(r) = t.r_multiple {
                r_trades += 1;
                r_sum = r_sum.checked_add(r).ok_or_else(|| ov("r sum"))?;
            }
        }
        let count = u32::try_from(trades.len()).map_err(|_| ov("trade count"))?;
        let win_rate = Decimal::from(wins)
            .checked_div(Decimal::from(count))
            .map_or(Decimal::ZERO, round4);
        let profit_factor = profit.checked_div(loss).map(round4);
        let expectancy_r = r_sum.checked_div(Decimal::from(r_trades)).map(round4);
        Ok(Self {
            trades: count,
            wins,
            losses,
            win_rate,
            net_pnl: net,
            gross_profit: profit,
            gross_loss: loss,
            profit_factor,
            expectancy_r,
            r_trades,
            no_r_trades: count - r_trades,
            drawdown: max_drawdown(starting_equity, curve)?,
        })
    }
}

/// Run-level facts the label depends on (pooled over folds and instruments).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunFacts {
    pub folds: u32,
    pub regimes: u32,
    /// True for a `latest_for_all` run.
    pub costs_approximate: bool,
    /// Distinct parameter sets evaluated in-sample in this run (prior trials are added from config).
    pub trials_in_run: u32,
}

/// The label and whether the profit factor / expectancy bar was met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    pub label: EvidenceLabel,
    /// Profit factor the run had to reach: the floor plus the trial penalty.
    pub required_profit_factor: Decimal,
    /// Trade gate, fold count, profit factor and expectancy all met, and not approximate-cost.
    /// The OOS/IS ratio and profitable-fold vote need per-fold data (slice 7) and are not here.
    pub meets_bar: bool,
}

/// Label and bar check for pooled OOS `metrics` (policy sections 3 to 5).
///
/// Exploratory for an approximate-cost run; Low evidence below `provisional_min_trades` pooled
/// trades or below `min_folds` folds; Trusted at `trusted_min_trades` with `trusted_min_regimes`
/// unless the trial count caps it; otherwise Provisional.
///
/// # Errors
/// `Overflow` if the required profit factor does not fit.
pub fn assess(
    cfg: &BacktestConfig,
    metrics: &Metrics,
    facts: &RunFacts,
) -> Result<Assessment, BacktestError> {
    let ev = &cfg.evidence;
    let required = ev
        .min_profit_factor
        .checked_add(cfg.trials.profit_factor_penalty(facts.trials_in_run))
        .ok_or_else(|| ov("required profit factor"))?;
    let label = if facts.costs_approximate {
        EvidenceLabel::Exploratory
    } else if metrics.trades < ev.provisional_min_trades || facts.folds < cfg.walk_forward.min_folds
    {
        EvidenceLabel::LowEvidence
    } else if metrics.trades >= ev.trusted_min_trades
        && facts.regimes >= ev.trusted_min_regimes
        && !cfg.trials.caps_label(facts.trials_in_run)
    {
        EvidenceLabel::Trusted
    } else {
        EvidenceLabel::Provisional
    };
    let meets_bar = !facts.costs_approximate
        && metrics.trades >= cfg.walk_forward.min_pooled_oos_trades
        && facts.folds >= cfg.walk_forward.min_folds
        && metrics.profit_factor.is_some_and(|pf| pf >= required)
        && metrics
            .expectancy_r
            .is_some_and(|e| e >= ev.min_expectancy_r);
    Ok(Assessment {
        label,
        required_profit_factor: required,
        meets_bar,
    })
}

#[cfg(test)]
mod tests;
