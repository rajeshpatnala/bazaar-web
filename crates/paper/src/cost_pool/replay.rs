//! Ledger replay into cost pools (BZ-F10-S10 slice 0a part 2; unused until the P&L source is
//! wired).
//!
//! Lines are applied in the order given, which must be ledger id order; timestamps never reorder
//! them. A `charge` line pairs with the line right after it, which must be a buy or sell with the
//! same session and `ref_id`. Any bad line or broken pairing fails the whole replay with a
//! [`ReplayError`]: no partial state is returned and nothing is read as zero.
//!
//! Amounts and quantities are bounded here, where lines are parsed, because the checked
//! operations of `rust_decimal` report only magnitude overflow and may round fraction digits
//! away. Within [`MAX_SCALE`], [`MAX_LINE_AMOUNT`], [`MAX_POOL_AMOUNT`] and [`MAX_POOL_QTY`]
//! every sum and product on pool figures is exact, and the one rounding (the paisa share of a
//! partial sale) lands on the right paisa. These are sanity bounds, not trading limits.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate};
use rust_decimal::Decimal;
use thiserror::Error;

use bazaar_execution::types::Product;

use super::{PaperSessionId, Pool, PoolError, SaleSplit};
use crate::account::LedgerKind;

/// Most fraction digits an amount may carry, trailing zeros aside.
pub const MAX_SCALE: u32 = 4;
/// Largest magnitude of one line's amount, in rupees (1e11).
pub const MAX_LINE_AMOUNT: Decimal = Decimal::from_parts(1_215_752_192, 23, 0, false, 0);
/// Largest share cost, and largest unsold charges, one pool may hold, in rupees (1e12).
pub const MAX_POOL_AMOUNT: Decimal = Decimal::from_parts(3_567_587_328, 232, 0, false, 0);
/// Most shares one pool may hold.
pub const MAX_POOL_QTY: u32 = 100_000_000;

/// One ledger line as stored, not yet checked. `session` is the `epoch` column. `symbol`, `qty`
/// and `product` are required on a buy or sell and not read on other lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayLine<'a> {
    pub session: u32,
    pub kind: &'a str,
    /// Signed rupees, credit positive.
    pub amount: Decimal,
    pub ref_id: Option<u128>,
    pub ts_ist_rfc3339: &'a str,
    pub symbol: Option<&'a str>,
    pub qty: Option<u32>,
    /// `Product::as_str` label.
    pub product: Option<&'a str>,
}

/// What makes one pool: all shares of one holding, in one session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub session: PaperSessionId,
    /// The string the position book keys on.
    pub symbol: String,
    pub product: Product,
}

/// One replayed sale; `date` is the IST date of its sell line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sale {
    pub key: PoolKey,
    pub date: NaiveDate,
    pub split: SaleSplit,
}

/// The outcome of a whole replay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replayed {
    /// Every pool that ever traded; one sold out stays here, empty.
    pub pools: HashMap<PoolKey, Pool>,
    /// Sales in ledger order.
    pub sales: Vec<Sale>,
}

/// What is wrong with a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LineFault {
    #[error("unknown line kind")]
    UnknownKind,
    #[error("session 0")]
    BadSession,
    #[error("timestamp is not RFC 3339 at +05:30")]
    BadDate,
    #[error("line dated after today")]
    FutureDate,
    #[error("amount has too many fraction digits")]
    TooPrecise,
    #[error("amount above the line bound")]
    AmountTooLarge,
    #[error("amount is zero or has the wrong sign for its kind")]
    WrongSign,
    #[error("buy or sell without a symbol")]
    MissingSymbol,
    #[error("buy or sell without a quantity")]
    MissingQty,
    #[error("buy or sell without a product")]
    MissingProduct,
    #[error("unknown product")]
    UnknownProduct,
    #[error("charge not followed by its buy or sell")]
    OrphanCharge,
    #[error("two charges in a row")]
    TwoCharges,
    #[error("buy or sell without a charge before it")]
    MissingCharge,
    #[error("charge and trade differ in ref_id, or one has none")]
    PairRefId,
    #[error("charge and trade are in different sessions")]
    PairSession,
    #[error("pool above its quantity or amount bound")]
    PoolTooLarge,
    #[error("pool: {0}")]
    Pool(PoolError),
}

/// Why a replay failed. `line` is the 0-based position of the offending line; no row content is
/// carried. For [`LineFault::OrphanCharge`] it is the charge's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("ledger line {line}: {fault}")]
pub struct ReplayError {
    pub line: usize,
    pub fault: LineFault,
}

/// A checked line; amounts are positive magnitudes.
enum Parsed {
    Opening,
    Charge(Charge),
    Trade(Trade),
}

#[derive(Clone, Copy)]
struct Charge {
    session: PaperSessionId,
    ref_id: Option<u128>,
    amount: Decimal,
}

struct Trade {
    is_buy: bool,
    key: PoolKey,
    ref_id: Option<u128>,
    date: NaiveDate,
    qty: u32,
    amount: Decimal,
}

fn date_of(ts: &str, today: NaiveDate) -> Result<NaiveDate, LineFault> {
    let date = DateTime::parse_from_rfc3339(ts)
        .ok()
        .filter(|_| ts.ends_with("+05:30"))
        .ok_or(LineFault::BadDate)?
        .date_naive();
    if date > today {
        return Err(LineFault::FutureDate);
    }
    Ok(date)
}

/// The positive magnitude of a bounded amount that has the sign its kind needs.
fn magnitude(amount: Decimal, credit: bool) -> Result<Decimal, LineFault> {
    // Trailing zeros carry no value and must not count against the scale.
    let amount = amount.normalize();
    if amount.scale() > MAX_SCALE {
        return Err(LineFault::TooPrecise);
    }
    if amount.abs() > MAX_LINE_AMOUNT {
        return Err(LineFault::AmountTooLarge);
    }
    if amount.is_zero() || amount.is_sign_positive() != credit {
        return Err(LineFault::WrongSign);
    }
    Ok(amount.abs())
}

fn parse(line: &ReplayLine<'_>, today: NaiveDate) -> Result<Parsed, LineFault> {
    let kind = LedgerKind::parse(line.kind).ok_or(LineFault::UnknownKind)?;
    let session = PaperSessionId::new(line.session).ok_or(LineFault::BadSession)?;
    let date = date_of(line.ts_ist_rfc3339, today)?;
    let credit = matches!(kind, LedgerKind::Opening | LedgerKind::Sell);
    let amount = magnitude(line.amount, credit)?;
    Ok(match kind {
        LedgerKind::Opening => Parsed::Opening,
        LedgerKind::Charge => Parsed::Charge(Charge {
            session,
            ref_id: line.ref_id,
            amount,
        }),
        LedgerKind::Buy | LedgerKind::Sell => {
            let symbol = line
                .symbol
                .filter(|s| !s.trim().is_empty())
                .ok_or(LineFault::MissingSymbol)?;
            let qty = line.qty.filter(|q| *q > 0).ok_or(LineFault::MissingQty)?;
            let label = line.product.ok_or(LineFault::MissingProduct)?;
            let product = [Product::Cnc, Product::Mis, Product::Nrml]
                .into_iter()
                .find(|p| p.as_str() == label)
                .ok_or(LineFault::UnknownProduct)?;
            Parsed::Trade(Trade {
                is_buy: kind == LedgerKind::Buy,
                key: PoolKey {
                    session,
                    symbol: symbol.to_owned(),
                    product,
                },
                ref_id: line.ref_id,
                date,
                qty,
                amount,
            })
        }
    })
}

impl Replayed {
    /// Applies one trade with the charge paired to it.
    fn apply(&mut self, charge: Charge, t: Trade) -> Result<(), LineFault> {
        if charge.session != t.key.session {
            return Err(LineFault::PairSession);
        }
        if charge.ref_id.is_none() || charge.ref_id != t.ref_id {
            return Err(LineFault::PairRefId);
        }
        let pool = self.pools.entry(t.key.clone()).or_default();
        if t.is_buy {
            pool.buy(t.qty, t.amount, charge.amount)
                .map_err(LineFault::Pool)?;
            if pool.qty() > MAX_POOL_QTY
                || pool.cost() > MAX_POOL_AMOUNT
                || pool.unsold_charges() > MAX_POOL_AMOUNT
            {
                return Err(LineFault::PoolTooLarge);
            }
        } else {
            let split = pool
                .sell(t.qty, t.amount, charge.amount)
                .map_err(LineFault::Pool)?;
            self.sales.push(Sale {
                key: t.key,
                date: t.date,
                split,
            });
        }
        Ok(())
    }
}

/// Replays `lines`, given in ledger id order, into cost pools and sales. Opening lines are
/// checked and are never P&L. `today` is the IST date from the `Clock`.
///
/// # Errors
/// [`ReplayError`] for the first bad line or broken pairing.
pub fn replay_pools<'a>(
    lines: impl IntoIterator<Item = ReplayLine<'a>>,
    today: NaiveDate,
) -> Result<Replayed, ReplayError> {
    let mut out = Replayed::default();
    // The charge waiting for its trade, with its position.
    let mut waiting: Option<(usize, Charge)> = None;
    for (index, line) in lines.into_iter().enumerate() {
        let at = |fault| ReplayError { line: index, fault };
        let parsed = parse(&line, today).map_err(at)?;
        match (waiting.take(), parsed) {
            (None, Parsed::Charge(c)) => waiting = Some((index, c)),
            (None, Parsed::Opening) => {}
            (None, Parsed::Trade(_)) => return Err(at(LineFault::MissingCharge)),
            (Some((_, c)), Parsed::Trade(t)) => out.apply(c, t).map_err(at)?,
            (Some(_), Parsed::Charge(_)) => return Err(at(LineFault::TwoCharges)),
            (Some((charge_at, _)), Parsed::Opening) => {
                return Err(ReplayError {
                    line: charge_at,
                    fault: LineFault::OrphanCharge,
                });
            }
        }
    }
    match waiting {
        Some((line, _)) => Err(ReplayError {
            line,
            fault: LineFault::OrphanCharge,
        }),
        None => Ok(out),
    }
}

#[cfg(test)]
mod tests;
