//! Expiry payoff of an options structure, exact from the piecewise-linear shape.
//!
//! P&L per unit of underlying at expiry price `S >= 0`:
//! `sum_i qty_i * (intrinsic_i(S) - premium_i)`, where `qty > 0` is long and `qty < 0` short
//! (premium paid by longs, received by shorts), call intrinsic `max(S-K,0)`, put `max(K-S,0)`.
//! The curve is linear between the sorted strikes (and `0`), so extremes and breakevens are
//! evaluated at those kinks plus the slope of the final ray; no sampling. Money is `Decimal`
//! (lot size and charges are applied by the caller). Gross of charges.

use crate::OptionKind;
use rust_decimal::Decimal;

/// One option leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leg {
    /// Call or put.
    pub kind: OptionKind,
    /// Strike (> 0).
    pub strike: Decimal,
    /// Premium per unit (>= 0).
    pub premium: Decimal,
    /// Signed quantity: positive long, negative short, zero ignored.
    pub qty: i32,
}

/// Payoff summary at expiry (per unit of underlying, gross of charges).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payoff {
    /// Best P&L; `None` when `unbounded_profit`.
    pub max_profit: Option<Decimal>,
    /// Worst loss as a positive magnitude (0 if the structure never loses); `None` when
    /// `unbounded_loss`.
    pub max_loss: Option<Decimal>,
    /// Profit grows without limit as the underlying rises (net long calls).
    pub unbounded_profit: bool,
    /// Loss grows without limit (net short calls). A naked short put is bounded at `K - premium`.
    pub unbounded_loss: bool,
    /// Underlying prices (>= 0) where P&L is zero, ascending.
    pub breakevens: Vec<Decimal>,
}

/// Payoff failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoffError {
    /// No legs with non-zero quantity.
    NoLegs,
    /// A strike is not positive, or a premium is negative.
    InvalidLeg,
    /// Decimal arithmetic overflowed.
    Overflow,
}

fn leg_pnl(l: &Leg, s: Decimal) -> Option<Decimal> {
    let intrinsic = match l.kind {
        OptionKind::Call => s.checked_sub(l.strike)?.max(Decimal::ZERO),
        OptionKind::Put => l.strike.checked_sub(s)?.max(Decimal::ZERO),
    };
    Decimal::from(l.qty).checked_mul(intrinsic.checked_sub(l.premium)?)
}

fn pnl_at(legs: &[Leg], s: Decimal) -> Option<Decimal> {
    legs.iter()
        .try_fold(Decimal::ZERO, |acc, l| acc.checked_add(leg_pnl(l, s)?))
}

/// Compute the expiry payoff of `legs`.
///
/// # Errors
/// See [`PayoffError`].
pub fn expiry_payoff(legs: &[Leg]) -> Result<Payoff, PayoffError> {
    let legs: Vec<Leg> = legs.iter().copied().filter(|l| l.qty != 0).collect();
    if legs.is_empty() {
        return Err(PayoffError::NoLegs);
    }
    if legs
        .iter()
        .any(|l| l.strike <= Decimal::ZERO || l.premium < Decimal::ZERO)
    {
        return Err(PayoffError::InvalidLeg);
    }
    let mut xs: Vec<Decimal> = std::iter::once(Decimal::ZERO)
        .chain(legs.iter().map(|l| l.strike))
        .collect();
    xs.sort();
    xs.dedup();
    let ys: Vec<Decimal> = xs
        .iter()
        .map(|x| pnl_at(&legs, *x))
        .collect::<Option<_>>()
        .ok_or(PayoffError::Overflow)?;
    // Slope beyond the last strike: net long calls.
    let slope: i64 = legs
        .iter()
        .filter(|l| l.kind == OptionKind::Call)
        .map(|l| i64::from(l.qty))
        .sum();
    let best = ys.iter().copied().max().unwrap_or(Decimal::ZERO);
    let worst = ys.iter().copied().min().unwrap_or(Decimal::ZERO);
    let unbounded_profit = slope > 0;
    let unbounded_loss = slope < 0;

    let mut breakevens = Vec::new();
    for (i, y) in ys.iter().enumerate() {
        if y.is_zero() {
            breakevens.push(xs[i]);
        }
        if let (Some(x1), Some(y1)) = (xs.get(i + 1), ys.get(i + 1)) {
            let crosses = (*y < Decimal::ZERO && *y1 > Decimal::ZERO)
                || (*y > Decimal::ZERO && *y1 < Decimal::ZERO);
            if crosses {
                let frac = (-*y)
                    .checked_div(y1.checked_sub(*y).ok_or(PayoffError::Overflow)?)
                    .ok_or(PayoffError::Overflow)?;
                let x = x1
                    .checked_sub(xs[i])
                    .and_then(|w| frac.checked_mul(w))
                    .and_then(|dx| xs[i].checked_add(dx))
                    .ok_or(PayoffError::Overflow)?;
                breakevens.push(x);
            }
        }
    }
    if let (Some(last), Some(x_last)) = (ys.last(), xs.last())
        && slope != 0
        && !last.is_zero()
        && (*last < Decimal::ZERO) == (slope > 0)
    {
        let x = (-*last)
            .checked_div(Decimal::from(slope))
            .and_then(|dx| x_last.checked_add(dx))
            .ok_or(PayoffError::Overflow)?;
        breakevens.push(x);
    }
    breakevens.sort();
    breakevens.dedup();
    Ok(Payoff {
        max_profit: (!unbounded_profit).then_some(best.max(Decimal::ZERO)),
        max_loss: (!unbounded_loss).then_some((-worst).max(Decimal::ZERO)),
        unbounded_profit,
        unbounded_loss,
        breakevens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use OptionKind::{Call, Put};
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn leg(kind: OptionKind, strike: &str, premium: &str, qty: i32) -> Leg {
        Leg {
            kind,
            strike: d(strike),
            premium: d(premium),
            qty,
        }
    }

    #[test]
    fn long_call() {
        let p = expiry_payoff(&[leg(Call, "100", "5", 1)]).unwrap();
        assert!(p.unbounded_profit && !p.unbounded_loss && p.max_profit.is_none());
        assert_eq!(p.max_loss, Some(d("5")));
        assert_eq!(p.breakevens, vec![d("105")]);
    }

    #[test]
    fn short_put_is_bounded() {
        let p = expiry_payoff(&[leg(Put, "100", "4", -1)]).unwrap();
        assert!(!p.unbounded_profit && !p.unbounded_loss);
        assert_eq!(p.max_profit, Some(d("4")));
        assert_eq!(p.max_loss, Some(d("96")));
        assert_eq!(p.breakevens, vec![d("96")]);
    }

    #[test]
    fn bull_call_spread() {
        let p = expiry_payoff(&[leg(Call, "100", "6", 1), leg(Call, "110", "2", -1)]).unwrap();
        assert!(!p.unbounded_profit && !p.unbounded_loss);
        assert_eq!(p.max_profit, Some(d("6")));
        assert_eq!(p.max_loss, Some(d("4")));
        assert_eq!(p.breakevens, vec![d("104")]);
    }

    #[test]
    fn iron_condor() {
        let p = expiry_payoff(&[
            leg(Put, "90", "1", 1),
            leg(Put, "95", "3", -1),
            leg(Call, "105", "3", -1),
            leg(Call, "110", "1", 1),
        ])
        .unwrap();
        assert_eq!(p.max_profit, Some(d("4")));
        assert_eq!(p.max_loss, Some(d("1")));
        assert!(!p.unbounded_loss && !p.unbounded_profit);
        assert_eq!(p.breakevens, vec![d("91"), d("109")]);
    }

    #[test]
    fn short_strangle_flags_unbounded_loss() {
        let p = expiry_payoff(&[leg(Put, "95", "3", -1), leg(Call, "105", "3", -1)]).unwrap();
        assert!(p.unbounded_loss && p.max_loss.is_none());
        assert_eq!(p.max_profit, Some(d("6")));
        assert_eq!(p.breakevens, vec![d("89"), d("111")]);
    }

    #[test]
    fn non_integer_breakeven_and_scaling() {
        let p = expiry_payoff(&[leg(Call, "100", "5", 1), leg(Call, "110", "1.5", -1)]).unwrap();
        assert_eq!(p.breakevens, vec![d("103.5")]);
        let q = expiry_payoff(&[leg(Call, "100", "5", 2), leg(Call, "110", "1.5", -2)]).unwrap();
        assert_eq!(q.max_loss, Some(d("7")));
        assert_eq!(q.breakevens, vec![d("103.5")]);
    }

    #[test]
    fn errors() {
        assert_eq!(expiry_payoff(&[]), Err(PayoffError::NoLegs));
        assert_eq!(
            expiry_payoff(&[leg(Call, "100", "1", 0)]),
            Err(PayoffError::NoLegs)
        );
        assert_eq!(
            expiry_payoff(&[leg(Call, "0", "1", 1)]),
            Err(PayoffError::InvalidLeg)
        );
        assert_eq!(
            expiry_payoff(&[leg(Call, "100", "-1", 1)]),
            Err(PayoffError::InvalidLeg)
        );
        let big = Leg {
            kind: Put,
            strike: Decimal::MAX,
            premium: Decimal::MAX,
            qty: i32::MAX,
        };
        assert_eq!(expiry_payoff(&[big]), Err(PayoffError::Overflow));
    }
}
