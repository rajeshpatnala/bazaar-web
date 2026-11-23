//! Tick rounding and sizing for one long idea. Every rupee figure is `Decimal`; the quantity comes
//! from `risk::sizing` only. Rounding goes against the trader (entry up, stop down, targets down),
//! then the levels are re-checked; a result that breaks order is a `BlockedIdea`, never fixed.
//!
//! `risk_rupees = risk_at_stop + max(cost(qty), min_charges_slippage_buffer)`, the same allowance
//! sizing used (BZ-522), and it must not exceed the trade budget.

use bazaar_core::instrument::Instrument;
use bazaar_risk::config::EffectiveLimits;
use bazaar_risk::sizing::{SizingError, SizingInput, size_with_cost, sizing_equity};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::{Side, Tri};
use rust_decimal::Decimal;

use crate::result::BlockReason;
use crate::types::{PositionSize, reward_risk};

/// The levels a strategy gave (long only).
#[derive(Debug, Clone, PartialEq)]
pub struct Levels {
    pub entry_low: Decimal,
    pub entry_high: Decimal,
    pub stop: Decimal,
    pub targets: Vec<Decimal>,
}

/// A sized idea: rounded levels and every rupee figure, derived here, never copied from a signal.
#[derive(Debug, Clone, PartialEq)]
pub struct SizedIdea {
    pub levels: Levels,
    pub size: PositionSize,
    /// `qty x (entry_high - stop)`.
    pub risk_at_stop: Decimal,
    /// `max(cost(qty), min_charges_slippage_buffer)`.
    pub allowance: Decimal,
    /// `risk_at_stop + allowance`; never above `budget`.
    pub risk_rupees: Decimal,
    pub budget: Decimal,
    pub notional: Decimal,
    /// `(first target - entry_high) / (entry_high - stop)` on the rounded levels.
    pub reward_risk: Decimal,
    pub capped_by_position_cap: bool,
}

fn collapse(why: &str) -> BlockReason {
    BlockReason::TickCollapse(why.to_owned())
}

fn to_tick(x: Decimal, tick: Decimal, up: bool) -> Result<Decimal, BlockReason> {
    let q = x
        .checked_div(tick)
        .ok_or_else(|| collapse("a price overflowed while rounding to the tick"))?;
    let q = if up { q.ceil() } else { q.floor() };
    q.checked_mul(tick)
        .ok_or_else(|| collapse("a price overflowed while rounding to the tick"))
}

/// Round against the trader to `tick` and re-validate. Entry low and high go up, the stop down,
/// every target down (long only). Errors name the broken check.
pub fn round_levels(levels: &Levels, tick: Decimal) -> Result<Levels, BlockReason> {
    if tick <= Decimal::ZERO {
        return Err(BlockReason::Sizing(
            "the tick size is 0 or negative, so levels cannot be rounded".to_owned(),
        ));
    }
    let (lo, hi) = (levels.entry_low, levels.entry_high);
    if lo <= Decimal::ZERO || lo > hi || levels.stop <= Decimal::ZERO || levels.stop >= lo {
        return Err(BlockReason::Sizing(
            "the strategy's entry zone or stop is invalid before rounding".to_owned(),
        ));
    }
    let r = Levels {
        entry_low: to_tick(lo, tick, true)?,
        entry_high: to_tick(hi, tick, true)?,
        stop: to_tick(levels.stop, tick, false)?,
        targets: levels
            .targets
            .iter()
            .map(|t| to_tick(*t, tick, false))
            .collect::<Result<_, _>>()?,
    };
    if lo < hi && r.entry_low == r.entry_high {
        return Err(collapse("the entry zone is narrower than one tick"));
    }
    if r.stop <= Decimal::ZERO {
        return Err(collapse("the stop rounded to zero or below"));
    }
    if r.stop >= r.entry_low {
        return Err(collapse("the stop is not below the entry zone"));
    }
    if r.targets.is_empty() {
        return Err(collapse("there is no target"));
    }
    if r.targets.iter().any(|t| *t <= r.entry_high) {
        return Err(collapse("a target is not above the entry zone"));
    }
    if !r.targets.windows(2).all(|w| w[1] > w[0]) {
        return Err(collapse("the targets are not strictly rising"));
    }
    Ok(r)
}

/// Round, size through `risk::size_with_cost` and compute the charges-inclusive risk.
/// `cost(qty)` is total charges + slippage for that quantity (the context `CostModel`).
pub fn size_idea(
    instrument: &Instrument,
    levels: &Levels,
    snapshot: &PortfolioSnapshot,
    limits: &EffectiveLimits,
    cost: &dyn Fn(u32) -> Tri<Decimal>,
) -> Result<SizedIdea, BlockReason> {
    let sizing = |e: SizingError| BlockReason::Sizing(e.to_string());
    let r = round_levels(levels, instrument.tick_size)?;
    let equity = sizing_equity(snapshot, limits).map_err(sizing)?;
    let input = SizingInput {
        equity,
        side: Side::Buy,
        entry: r.entry_high,
        stop: r.stop,
        lot_size: instrument.lot_size,
        tick_size: instrument.tick_size,
        buffer: Tri::Unknown,
        overridden: false,
    };
    let s = size_with_cost(limits, &input, cost).map_err(sizing)?;
    let spent = *cost(s.qty)
        .known()
        .ok_or_else(|| BlockReason::Sizing("the charges estimate is unknown".to_owned()))?;
    let allowance = spent.max(limits.min_charges_slippage_buffer);
    let risk_rupees = s
        .risk_at_stop
        .checked_add(allowance)
        .ok_or_else(|| BlockReason::Sizing("the risk calculation overflowed".to_owned()))?;
    if risk_rupees > s.budget {
        return Err(BlockReason::RiskAboveBudget);
    }
    let first = r.targets.first().copied().unwrap_or(r.entry_high);
    let rr = reward_risk(r.entry_high, r.stop, first)
        .filter(|v| *v > Decimal::ZERO)
        .ok_or_else(|| collapse("reward:risk is not above zero"))?;
    Ok(SizedIdea {
        size: PositionSize {
            qty: s.qty,
            lots: s.lots,
        },
        risk_at_stop: s.risk_at_stop,
        allowance,
        risk_rupees,
        budget: s.budget,
        notional: s.notional,
        reward_risk: rr,
        capped_by_position_cap: s.capped_by_position_cap,
        levels: r,
    })
}

#[cfg(test)]
mod tests;
