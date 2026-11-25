//! Pure weighted-average cost pools for paper trading (BZ-F10-S10 slice 0a; unused until the
//! P&L source is wired).
//!
//! One [`Pool`] holds the shares of one (paper session, symbol, product): quantity, share cost
//! without charges, and the buy charges of the shares not yet sold. No average is stored or fed
//! back. A sale's result is the money received after its charge, less the sold shares' cost and
//! their share of the buy charges. These are weighted-average figures, not tax figures.
//! Every failure is a [`PoolError`]; none is ever turned into a zero figure.

use std::fmt;
use std::num::NonZeroU32;

use rust_decimal::{Decimal, RoundingStrategy};
use thiserror::Error;

/// A numbered paper session (P#1, P#2, ...), stored in the ledger's `epoch` column. Never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaperSessionId(NonZeroU32);

impl PaperSessionId {
    /// `None` for 0, which is no session.
    #[must_use]
    pub fn new(n: u32) -> Option<Self> {
        NonZeroU32::new(n).map(Self)
    }

    /// The stored number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for PaperSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P#{}", self.0)
    }
}

/// Why a trade or a mark cannot be applied to a pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PoolError {
    #[error("buy or sell without a quantity")]
    MissingQty,
    #[error("sell above the quantity held in its pool")]
    SellAboveHeld,
    #[error("negative amount or charge")]
    WrongSign,
    #[error("buy cost or sale proceeds of zero")]
    ZeroAmount,
    #[error("overflow in pool arithmetic")]
    Overflow,
    #[error("mark price is not positive")]
    BadMark,
}

/// The shares held in one pool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pool {
    qty: u32,
    cost: Decimal,
    charges: Decimal,
}

/// What one sale took out of its pool, and what it made or lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaleSplit {
    pub cost_out: Decimal,
    pub charge_out: Decimal,
    /// `(proceeds - sale charge) - cost_out - charge_out`.
    pub result: Decimal,
}

fn add(a: Decimal, b: Decimal) -> Result<Decimal, PoolError> {
    a.checked_add(b).ok_or(PoolError::Overflow)
}

fn sub(a: Decimal, b: Decimal) -> Result<Decimal, PoolError> {
    a.checked_sub(b).ok_or(PoolError::Overflow)
}

/// The part of `total` that goes with `sold` of `held` shares, rounded up to a paisa and never
/// above `total`. The only rounding here. A partial sale never overstates its result against what
/// the pool holds then; later results and open loss can read better by the paise taken early. A
/// full exit makes totals exact.
fn share_of(total: Decimal, sold: u32, held: u32) -> Result<Decimal, PoolError> {
    let part = total
        .checked_mul(Decimal::from(sold))
        .and_then(|v| v.checked_div(Decimal::from(held)))
        .ok_or(PoolError::Overflow)?;
    Ok(part
        .round_dp_with_strategy(2, RoundingStrategy::AwayFromZero)
        .min(total))
}

/// A trade needs a quantity, no negative figure and an amount above zero. A charge may be zero.
fn check(qty: u32, amount: Decimal, charge: Decimal) -> Result<(), PoolError> {
    if qty == 0 {
        return Err(PoolError::MissingQty);
    }
    if amount < Decimal::ZERO || charge < Decimal::ZERO {
        return Err(PoolError::WrongSign);
    }
    if amount.is_zero() {
        return Err(PoolError::ZeroAmount);
    }
    Ok(())
}

impl Pool {
    /// Shares held.
    #[must_use]
    pub const fn qty(&self) -> u32 {
        self.qty
    }

    /// Cost of the shares held, without charges.
    #[must_use]
    pub const fn cost(&self) -> Decimal {
        self.cost
    }

    /// Buy charges belonging to the shares held.
    #[must_use]
    pub const fn unsold_charges(&self) -> Decimal {
        self.charges
    }

    /// `min(0, mark x qty - (cost + unsold charges))`: a gain counts as 0, so it offsets
    /// nothing outside this pool.
    ///
    /// # Errors
    /// `BadMark` for a mark that is not positive; `Overflow`.
    pub fn open_loss(&self, mark: Decimal) -> Result<Decimal, PoolError> {
        if mark <= Decimal::ZERO {
            return Err(PoolError::BadMark);
        }
        let value = mark
            .checked_mul(Decimal::from(self.qty))
            .ok_or(PoolError::Overflow)?;
        Ok(sub(value, add(self.cost, self.charges)?)?.min(Decimal::ZERO))
    }

    /// Adds a buy of `qty` shares; `cost` (without charges) is a positive magnitude and
    /// `charge` is zero or more. The pool is unchanged on error.
    ///
    /// # Errors
    /// `MissingQty` for 0 shares; `WrongSign` for a negative figure; `ZeroAmount` for a zero
    /// cost; `Overflow`.
    pub fn buy(&mut self, qty: u32, cost: Decimal, charge: Decimal) -> Result<(), PoolError> {
        check(qty, cost, charge)?;
        *self = Self {
            qty: self.qty.checked_add(qty).ok_or(PoolError::Overflow)?,
            cost: add(self.cost, cost)?,
            charges: add(self.charges, charge)?,
        };
        Ok(())
    }

    /// Takes a sale of `qty` shares out; `proceeds` is a positive magnitude and `charge` is
    /// zero or more. A full exit takes everything left without dividing. The pool is unchanged
    /// on error.
    ///
    /// # Errors
    /// `MissingQty` for 0 shares; `WrongSign` for a negative figure; `ZeroAmount` for zero
    /// proceeds; `SellAboveHeld`; `Overflow`.
    pub fn sell(
        &mut self,
        qty: u32,
        proceeds: Decimal,
        charge: Decimal,
    ) -> Result<SaleSplit, PoolError> {
        check(qty, proceeds, charge)?;
        let left = self.qty.checked_sub(qty).ok_or(PoolError::SellAboveHeld)?;
        let (cost_out, charge_out) = if left == 0 {
            (self.cost, self.charges)
        } else {
            (
                share_of(self.cost, qty, self.qty)?,
                share_of(self.charges, qty, self.qty)?,
            )
        };
        let result = sub(sub(sub(proceeds, charge)?, cost_out)?, charge_out)?;
        *self = Self {
            qty: left,
            cost: sub(self.cost, cost_out)?,
            charges: sub(self.charges, charge_out)?,
        };
        Ok(SaleSplit {
            cost_out,
            charge_out,
            result,
        })
    }
}

#[cfg(test)]
mod tests;
