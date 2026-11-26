//! Pure replay of paper ledger lines into weighted-average cost pools (BZ-F10-S10 slice 0a;
//! unused until the P&L source is wired).
//!
//! One pool per (paper session, symbol, product): quantity, share cost without charges, and the
//! buy charges of the shares not yet sold. No average is stored or fed back. A sale's result is
//! the money received after its charge, less the sold shares' cost and their share of the buy
//! charges. These are weighted-average figures, not tax figures.
//!
//! Lines are replayed in the order given, which must be ledger id order; they are never sorted,
//! least of all by timestamp. `account.rs` appends a fill as its charge line then its buy or sell
//! line, so a charge belongs to the very next line. Anything else fails closed with a
//! [`PoolError`]; no error is ever turned into a zero figure.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use bazaar_core::ports::LedgerEntry;
use bazaar_execution::types::Product;
use chrono::{DateTime, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};
use thiserror::Error;

use crate::account::LedgerKind;

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

/// Why ledger lines or a mark cannot be turned into pools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PoolError {
    #[error("unknown ledger line kind")]
    UnknownKind,
    #[error("ledger line has no valid paper session")]
    BadSession,
    #[error("ledger line time is not an IST (+05:30) RFC 3339 time")]
    BadDate,
    #[error("ledger line is dated after today")]
    FutureDate,
    #[error("buy or sell line without a symbol")]
    MissingSymbol,
    #[error("buy or sell line without a quantity")]
    MissingQty,
    #[error("buy or sell line without a known product")]
    MissingProduct,
    #[error("sell above the quantity held in its pool")]
    SellAboveHeld,
    #[error("charge line not followed by its buy or sell line")]
    OrphanCharge,
    #[error("two charge lines in a row")]
    DoubleCharge,
    #[error("buy or sell line without a charge line before it")]
    MissingCharge,
    #[error("ledger amount has the wrong sign")]
    WrongSign,
    #[error("charge and trade line do not share a ref_id")]
    RefIdMismatch,
    #[error("charge and trade line are in different paper sessions")]
    SessionMismatch,
    #[error("overflow in pool arithmetic")]
    Overflow,
    #[error("mark price is not positive")]
    BadMark,
}

/// One ledger line to replay. The fill fields are beside the entry until the ledger stores them;
/// they are read on buy and sell lines only.
#[derive(Debug, Clone, Copy)]
pub struct PoolLine<'a> {
    pub entry: &'a LedgerEntry,
    pub symbol: Option<&'a str>,
    pub qty: Option<u32>,
    /// `Product::as_str` label.
    pub product: Option<&'a str>,
}

/// What separates one pool from another.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PoolKey {
    pub session: PaperSessionId,
    pub symbol: String,
    /// `Product::as_str` label.
    pub product: &'static str,
}

/// The shares held under one [`PoolKey`].
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

/// One sale, dated by its ledger line (IST).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sale {
    pub key: PoolKey,
    pub date: NaiveDate,
    pub split: SaleSplit,
}

/// The outcome of a replay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replay {
    /// Every pool ever bought into; a fully sold one stays, all zero.
    pub pools: BTreeMap<PoolKey, Pool>,
    /// Sales in ledger order.
    pub sales: Vec<Sale>,
}

fn add(a: Decimal, b: Decimal) -> Result<Decimal, PoolError> {
    a.checked_add(b).ok_or(PoolError::Overflow)
}

fn sub(a: Decimal, b: Decimal) -> Result<Decimal, PoolError> {
    a.checked_sub(b).ok_or(PoolError::Overflow)
}

/// The part of `total` that goes with `sold` of `held` shares, rounded up to a paisa and never
/// above `total`. The only rounding here; rounding up never overstates a sale result.
fn share_of(total: Decimal, sold: u32, held: u32) -> Result<Decimal, PoolError> {
    let part = total
        .checked_mul(Decimal::from(sold))
        .and_then(|v| v.checked_div(Decimal::from(held)))
        .ok_or(PoolError::Overflow)?;
    Ok(part
        .round_dp_with_strategy(2, RoundingStrategy::AwayFromZero)
        .min(total))
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

    /// Adds a buy; `cost` and `charge` are positive magnitudes. Unchanged on error.
    fn buy(&mut self, qty: u32, cost: Decimal, charge: Decimal) -> Result<(), PoolError> {
        *self = Self {
            qty: self.qty.checked_add(qty).ok_or(PoolError::Overflow)?,
            cost: add(self.cost, cost)?,
            charges: add(self.charges, charge)?,
        };
        Ok(())
    }

    /// Takes a sale out. A full exit takes everything left without dividing. Unchanged on error.
    fn sell(
        &mut self,
        qty: u32,
        proceeds: Decimal,
        charge: Decimal,
    ) -> Result<SaleSplit, PoolError> {
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

/// A charge line waiting for the trade line that must follow it.
struct PendingCharge {
    session: PaperSessionId,
    ref_id: u128,
    /// Positive magnitude.
    charge: Decimal,
}

fn ist_date(ts: &str, today: NaiveDate) -> Result<NaiveDate, PoolError> {
    let date = DateTime::parse_from_rfc3339(ts)
        .ok()
        .filter(|_| ts.ends_with("+05:30"))
        .ok_or(PoolError::BadDate)?
        .date_naive();
    if date > today {
        return Err(PoolError::FutureDate);
    }
    Ok(date)
}

impl Replay {
    /// Books the buy or sell `line` with the charge paired to it.
    fn trade(
        &mut self,
        line: &PoolLine<'_>,
        charge: &PendingCharge,
        is_buy: bool,
        date: NaiveDate,
    ) -> Result<(), PoolError> {
        let amount = line.entry.amount;
        if amount.is_zero() || amount.is_sign_negative() != is_buy {
            return Err(PoolError::WrongSign);
        }
        let symbol = line
            .symbol
            .filter(|s| !s.is_empty())
            .ok_or(PoolError::MissingSymbol)?;
        let qty = line.qty.filter(|q| *q > 0).ok_or(PoolError::MissingQty)?;
        let product = line
            .product
            .and_then(Product::parse)
            .ok_or(PoolError::MissingProduct)?;
        let key = PoolKey {
            session: charge.session,
            symbol: symbol.to_owned(),
            product: product.as_str(),
        };
        if is_buy {
            let pool = self.pools.entry(key).or_default();
            pool.buy(qty, -amount, charge.charge)
        } else {
            let pool = self.pools.get_mut(&key).ok_or(PoolError::SellAboveHeld)?;
            let split = pool.sell(qty, amount, charge.charge)?;
            self.sales.push(Sale { key, date, split });
            Ok(())
        }
    }
}

/// Replays `lines`, which must be in ledger id order and may span paper sessions. `today` is
/// the IST date from the `Clock`. Opening lines are cash, not trades: they touch no pool.
///
/// # Errors
/// The first [`PoolError`] met; there is no partial result.
pub fn replay(lines: &[PoolLine<'_>], today: NaiveDate) -> Result<Replay, PoolError> {
    let mut out = Replay::default();
    let mut pending: Option<PendingCharge> = None;
    for line in lines {
        let e = line.entry;
        let session = PaperSessionId::new(e.epoch).ok_or(PoolError::BadSession)?;
        let kind = LedgerKind::parse(&e.kind).ok_or(PoolError::UnknownKind)?;
        let date = ist_date(&e.ts_ist_rfc3339, today)?;
        match kind {
            LedgerKind::Opening => {
                if pending.is_some() {
                    return Err(PoolError::OrphanCharge);
                }
            }
            LedgerKind::Charge => {
                if pending.is_some() {
                    return Err(PoolError::DoubleCharge);
                }
                if e.amount.is_zero() || !e.amount.is_sign_negative() {
                    return Err(PoolError::WrongSign);
                }
                pending = Some(PendingCharge {
                    session,
                    ref_id: e.ref_id.ok_or(PoolError::RefIdMismatch)?,
                    charge: -e.amount,
                });
            }
            LedgerKind::Buy | LedgerKind::Sell => {
                let charge = pending.take().ok_or(PoolError::MissingCharge)?;
                if charge.session != session {
                    return Err(PoolError::SessionMismatch);
                }
                if e.ref_id != Some(charge.ref_id) {
                    return Err(PoolError::RefIdMismatch);
                }
                out.trade(line, &charge, kind == LedgerKind::Buy, date)?;
            }
        }
    }
    match pending {
        Some(_) => Err(PoolError::OrphanCharge),
        None => Ok(out),
    }
}

#[cfg(test)]
mod tests;
