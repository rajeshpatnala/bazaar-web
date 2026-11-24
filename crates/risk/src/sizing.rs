//! Fixed-fractional position sizing in `Decimal`: lots floored, never rounded up, risk at the stop
//! never above budget, all maths checked. One lot over budget is "no trade", not one lot anyway.
//!
//! `qty = floor((equity x risk% - buffer) / per-unit risk)`, floored to lots, then capped by the
//! position-value cap (G14), then (overridden trades) by `size_factor`, floored to lots again.
//! Finally the size steps down a lot at a time until `risk at stop + allowance(qty) <= budget`,
//! where the allowance is `max(cost(qty), min_charges_slippage_buffer)`: the configured floor means
//! "assume at least this much cost", it never refuses a cheap order (`size_with_cost`).
//!
//! Scope: `size` is for stop-defined linear instruments (equity, futures) only. Options sizing is
//! out of F6 scope: a short option leg has unbounded loss and a bought option risks its premium,
//! so rule G13 must keep those entries away from `size`.
//!
//! The G16 per-order notional cap (`max_notional_per_order_inr`) also limits the size, at
//! `max(entry, reference)` where `reference` is the gate's reference price (`size_with_reference`).
//! G16 in the gate stays the authority. Not applied here: the cash/margin cap (X03 rejects an
//! order above it); G14 counts an existing same-symbol position, the sizer does not.

use crate::config::EffectiveLimits;
use crate::snapshot::PortfolioSnapshot;
use crate::types::{Side, Tri};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::fmt;

const HUNDRED: Decimal = Decimal::ONE_HUNDRED;

/// Why no size could be produced. Every variant is a Reject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SizingError {
    /// Lot size is 0.
    ZeroLot,
    /// Tick size is 0 or negative.
    ZeroTick,
    /// Entry or stop is not a multiple of the tick size.
    PriceOffTick,
    /// Entry price is 0 or negative.
    BadPrice,
    EntryEqualsStop,
    /// Long stop at or above entry, or short stop at or below entry.
    StopOnWrongSide,
    /// Equity is 0 or negative.
    ZeroEquity,
    /// A required snapshot input is unknown (named).
    InputUnknown(&'static str),
    /// The charges + slippage cost is negative (a bad input, never a discount).
    NegativeCost,
    /// One lot risks more than the budget: no trade.
    BudgetBelowOneLot,
    /// One lot is worth more than the position-value cap: no trade.
    CapBelowOneLot,
    /// One lot is worth more than the per-order cap (G16), or that cap is 0: no trade.
    OrderCapBelowOneLot,
    /// Overridden size (`size_factor`) floors to zero lots.
    OverrideBelowOneLot,
    /// `size_factor` outside (0, 1].
    BadSizeFactor,
    /// Overflow or division failure.
    Arithmetic,
}

impl fmt::Display for SizingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::ZeroLot => "the lot size is 0, so the size cannot be worked out",
            Self::ZeroTick => "the tick size is 0, so the size cannot be worked out",
            Self::PriceOffTick => "the entry or stop price is not on a valid price step",
            Self::BadPrice => "the entry price must be above 0",
            Self::EntryEqualsStop => "the stop is at the entry price, so there is no risk to size",
            Self::StopOnWrongSide => "the stop is on the wrong side of the entry price",
            Self::ZeroEquity => "there is no capital left to size from",
            Self::InputUnknown(what) => return write!(f, "cannot confirm {what}; no size given"),
            Self::NegativeCost => "the charges and slippage estimate is negative, so no size given",
            Self::BudgetBelowOneLot => {
                "This setup needs more money than your risk limit allows, so there is no trade here."
            }
            Self::CapBelowOneLot => {
                "one lot is bigger than your single-position cap, so there is no trade here"
            }
            Self::OrderCapBelowOneLot => {
                "one lot is bigger than your per-order cap, so there is no trade here"
            }
            Self::OverrideBelowOneLot => {
                "the reduced override size is below one lot, so there is no trade"
            }
            Self::BadSizeFactor => "the override size factor must be above 0 and at most 1",
            Self::Arithmetic => "the size calculation overflowed",
        };
        f.write_str(text)
    }
}

impl std::error::Error for SizingError {}

/// What to size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizingInput {
    /// From [`sizing_equity`].
    pub equity: Decimal,
    pub side: Side,
    pub entry: Decimal,
    pub stop: Decimal,
    pub lot_size: u32,
    pub tick_size: Decimal,
    /// Total estimated charges + slippage, rupees, used by [`size`] as a flat figure (compute it
    /// for the largest candidate quantity); the allowance is `max(buffer, floor)`; `Unknown` and
    /// negative values reject. [`size_with_cost`] ignores it.
    pub buffer: Tri<Decimal>,
    /// Apply `override.size_factor` (an overridden trade).
    pub overridden: bool,
}

/// A size the gate may use. Rupee figures are derived here, never taken from the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sized {
    pub qty: u32,
    pub lots: u32,
    /// `qty x |entry - stop|` (rupees at the stop, before the buffer).
    pub risk_at_stop: Decimal,
    /// Risk budget for the trade (`equity x risk%`).
    pub budget: Decimal,
    pub notional: Decimal,
    /// True when the position-value cap (G14) reduced the size.
    pub capped_by_position_cap: bool,
    /// True when the per-order notional cap (G16) reduced the size further.
    pub capped_by_order_cap: bool,
    /// True when `size_factor` reduced the size.
    pub reduced_by_override: bool,
}

/// Equity to size from: the lower of (starting cash + realised) and mark-to-market, and, when
/// `limits.cap_equity_at_starting_cash`, never above starting cash (size never rises after a win).
pub fn sizing_equity(
    snapshot: &PortfolioSnapshot,
    limits: &EffectiveLimits,
) -> Result<Decimal, SizingError> {
    let start = *snapshot
        .starting_cash
        .known()
        .ok_or(SizingError::InputUnknown("your starting cash"))?;
    let realised = *snapshot
        .realised_pnl
        .known()
        .ok_or(SizingError::InputUnknown("your realised profit and loss"))?;
    let mtm = *snapshot
        .equity_mtm
        .known()
        .ok_or(SizingError::InputUnknown("your current account value"))?;
    let base = start.checked_add(realised).ok_or(SizingError::Arithmetic)?;
    let mut equity = base.min(mtm);
    if limits.cap_equity_at_starting_cash {
        equity = equity.min(start);
    }
    if equity <= Decimal::ZERO {
        return Err(SizingError::ZeroEquity);
    }
    Ok(equity)
}

fn pct_of(amount: Decimal, pct: Decimal) -> Result<Decimal, SizingError> {
    amount
        .checked_mul(pct)
        .and_then(|v| v.checked_div(HUNDRED))
        .ok_or(SizingError::Arithmetic)
}

/// Whole lots in `qty`, floored.
fn lots_in(qty: Decimal, lot: u32) -> Result<u32, SizingError> {
    qty.checked_div(Decimal::from(lot))
        .map(|v| v.floor())
        .and_then(|v| v.to_u32())
        .ok_or(SizingError::Arithmetic)
}

/// Input checks: lot, tick, prices on the tick grid, stop side, equity.
fn validate(input: &SizingInput) -> Result<(), SizingError> {
    if input.lot_size == 0 {
        return Err(SizingError::ZeroLot);
    }
    if input.tick_size <= Decimal::ZERO {
        return Err(SizingError::ZeroTick);
    }
    if input.entry <= Decimal::ZERO || input.stop <= Decimal::ZERO {
        return Err(SizingError::BadPrice);
    }
    if input.entry.checked_rem(input.tick_size) != Some(Decimal::ZERO)
        || input.stop.checked_rem(input.tick_size) != Some(Decimal::ZERO)
    {
        return Err(SizingError::PriceOffTick);
    }
    if input.entry == input.stop {
        return Err(SizingError::EntryEqualsStop);
    }
    let wrong_side = match input.side {
        Side::Buy => input.stop > input.entry,
        Side::Sell => input.stop < input.entry,
    };
    if wrong_side {
        return Err(SizingError::StopOnWrongSide);
    }
    if input.equity <= Decimal::ZERO {
        return Err(SizingError::ZeroEquity);
    }
    Ok(())
}

/// Size one trade with a flat `input.buffer` as the cost at every quantity. See [`size_with_cost`].
pub fn size(limits: &EffectiveLimits, input: &SizingInput) -> Result<Sized, SizingError> {
    size_with_cost(limits, input, &|_| input.buffer)
}

/// Size one trade; `cost(qty)` is the total charges + slippage for that quantity, rupees. The
/// allowance used is `max(cost(qty), min_charges_slippage_buffer)` at every quantity, including
/// the final one after the step-down.
/// Rejects: lot or tick 0, entry == stop, stop on the wrong side, zero equity,
/// unknown or negative cost, budget below one lot, bad `size_factor`.
pub fn size_with_cost(
    limits: &EffectiveLimits,
    input: &SizingInput,
    cost: &dyn Fn(u32) -> Tri<Decimal>,
) -> Result<Sized, SizingError> {
    size_with_reference(limits, input, cost, input.entry)
}

/// As [`size_with_cost`], and the quantity is also limited so `max(entry, reference) x qty` stays
/// within `max_notional_per_order_inr` (the G16 per-order cap, using the gate's reference price:
/// `max(intent.price, live price)`). `reference` 0 or negative is `BadPrice`; one lot above the
/// cap (or a cap of 0) is `OrderCapBelowOneLot`. Never a larger size than `size_with_cost` gave.
pub fn size_with_reference(
    limits: &EffectiveLimits,
    input: &SizingInput,
    cost: &dyn Fn(u32) -> Tri<Decimal>,
    reference: Decimal,
) -> Result<Sized, SizingError> {
    validate(input)?;
    // A hand-built limits struct with a floor <= 0 must not weaken "never below the floor".
    if limits.min_charges_slippage_buffer <= Decimal::ZERO {
        return Err(SizingError::InputUnknown("the charges and slippage floor"));
    }
    // Allowance for a quantity: `max(cost, floor)`. Unknown or negative cost fails; the floor is
    // the configured `min_charges_slippage_buffer` (assume at least that much cost), so the
    // allowance is never below it. Same function for backtest, paper and live.
    let cost_at = |qty: Decimal| -> Result<Decimal, SizingError> {
        let q = qty.to_u32().ok_or(SizingError::Arithmetic)?;
        let c = *cost(q).known().ok_or(SizingError::InputUnknown(
            "the charges and slippage allowance",
        ))?;
        if c < Decimal::ZERO {
            return Err(SizingError::NegativeCost);
        }
        Ok(c.max(limits.min_charges_slippage_buffer))
    };
    // First estimate from the cost of one lot; the step-down below fixes it for the final size.
    let buffer = cost_at(Decimal::from(input.lot_size))?;

    let per_unit = input
        .entry
        .checked_sub(input.stop)
        .map(|d| d.abs())
        .ok_or(SizingError::Arithmetic)?;
    let budget = pct_of(input.equity, limits.max_risk_per_trade_pct)?;
    let spendable = budget.checked_sub(buffer).ok_or(SizingError::Arithmetic)?;
    if spendable <= Decimal::ZERO {
        return Err(SizingError::BudgetBelowOneLot);
    }
    let risk_qty = spendable
        .checked_div(per_unit)
        .ok_or(SizingError::Arithmetic)?
        .floor();
    let mut lots = lots_in(risk_qty, input.lot_size)?;
    if lots == 0 {
        return Err(SizingError::BudgetBelowOneLot);
    }

    // G14 cap on notional.
    let cap_value = pct_of(input.equity, limits.max_position_value_pct)?;
    let cap_qty = cap_value
        .checked_div(input.entry)
        .ok_or(SizingError::Arithmetic)?
        .floor();
    let cap_lots = lots_in(cap_qty, input.lot_size)?;
    if cap_lots == 0 {
        return Err(SizingError::CapBelowOneLot);
    }
    let capped_by_position_cap = cap_lots < lots;
    lots = lots.min(cap_lots);

    // G16 cap on notional per order, at the gate's reference price (never below the entry).
    let order_cap_lots = order_cap_in_lots(limits, input, reference)?;
    let capped_by_order_cap = order_cap_lots < lots;
    lots = lots.min(order_cap_lots);

    // Overridden trade: size_factor x capped size, floored to lots.
    let factor = limits.override_size_factor;
    let mut reduced_by_override = false;
    if input.overridden {
        if factor <= Decimal::ZERO || factor > Decimal::ONE {
            return Err(SizingError::BadSizeFactor);
        }
        let scaled = Decimal::from(lots)
            .checked_mul(factor)
            .ok_or(SizingError::Arithmetic)?
            .floor()
            .to_u32()
            .ok_or(SizingError::Arithmetic)?;
        if scaled == 0 {
            return Err(SizingError::OverrideBelowOneLot);
        }
        reduced_by_override = scaled < lots;
        lots = scaled;
    }

    // Step down a lot at a time until risk at the stop plus the cost of that quantity fits.
    let lot_dec = Decimal::from(input.lot_size);
    let (qty_dec, risk_at_stop) = loop {
        if lots == 0 {
            return Err(SizingError::BudgetBelowOneLot);
        }
        let qty_dec = Decimal::from(lots)
            .checked_mul(lot_dec)
            .ok_or(SizingError::Arithmetic)?;
        let risk = qty_dec
            .checked_mul(per_unit)
            .ok_or(SizingError::Arithmetic)?;
        let total = risk
            .checked_add(cost_at(qty_dec)?)
            .ok_or(SizingError::Arithmetic)?;
        if total <= budget {
            break (qty_dec, risk);
        }
        lots = lots.saturating_sub(1);
    };
    let qty = qty_dec.to_u32().ok_or(SizingError::Arithmetic)?;
    let notional = qty_dec
        .checked_mul(input.entry)
        .ok_or(SizingError::Arithmetic)?;
    Ok(Sized {
        qty,
        lots,
        risk_at_stop,
        budget,
        notional,
        capped_by_position_cap,
        capped_by_order_cap,
        reduced_by_override,
    })
}

/// Whole lots allowed by the per-order notional cap at `max(entry, reference)`.
fn order_cap_in_lots(
    limits: &EffectiveLimits,
    input: &SizingInput,
    reference: Decimal,
) -> Result<u32, SizingError> {
    if reference <= Decimal::ZERO {
        return Err(SizingError::BadPrice);
    }
    let order_ref = reference.max(input.entry);
    let qty = limits
        .max_notional_per_order_inr
        .checked_div(order_ref)
        .ok_or(SizingError::Arithmetic)?
        .floor();
    let lots = lots_in(qty, input.lot_size)?;
    if lots == 0 {
        return Err(SizingError::OrderCapBelowOneLot);
    }
    Ok(lots)
}
