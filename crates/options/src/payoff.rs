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

/// Maximum number of legs accepted (bounds the O(n^2) work).
pub const MAX_LEGS: usize = 64;

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
    /// Best P&L (negative for a structure that always loses); `None` when `unbounded_profit`.
    pub max_profit: Option<Decimal>,
    /// Worst loss as a positive magnitude (0 if the structure never loses); `None` when
    /// `unbounded_loss`.
    pub max_loss: Option<Decimal>,
    /// Profit grows without limit as the underlying rises (net long calls).
    pub unbounded_profit: bool,
    /// Loss grows without limit (net short calls). A naked short put is bounded at `K - premium`.
    pub unbounded_loss: bool,
    /// Informational: for some option kind, total short quantity exceeds total long quantity of
    /// the same kind (a short leg not covered by a long). It does not check strike ordering; the
    /// risk crate and scanner own the defined-risk classification and margin.
    pub has_naked_short: bool,
    /// Underlying prices (>= 0) where P&L is zero, ascending. Unrounded (the display layer rounds
    /// to the tick). Interior points of a zero-P&L plateau are dropped; its two ends are kept.
    pub breakevens: Vec<Decimal>,
}

/// Payoff failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoffError {
    /// No legs with non-zero quantity.
    NoLegs,
    /// A strike is not positive, or a premium is negative.
    InvalidLeg,
    /// More than [`MAX_LEGS`] legs.
    TooManyLegs,
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
    if legs.len() > MAX_LEGS {
        return Err(PayoffError::TooManyLegs);
    }
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
    let side = |kind: OptionKind, short: bool| -> Option<i64> {
        legs.iter()
            .filter(|l| l.kind == kind && (l.qty < 0) == short)
            .try_fold(0_i64, |a, l| a.checked_add(i64::from(l.qty)))
    };
    let (call_long, call_short) = (
        side(OptionKind::Call, false).ok_or(PayoffError::Overflow)?,
        side(OptionKind::Call, true).ok_or(PayoffError::Overflow)?,
    );
    let (put_long, put_short) = (
        side(OptionKind::Put, false).ok_or(PayoffError::Overflow)?,
        side(OptionKind::Put, true).ok_or(PayoffError::Overflow)?,
    );
    let slope = call_long + call_short;
    let has_naked_short = -call_short > call_long || -put_short > put_long;
    let best = ys.iter().copied().max().unwrap_or(Decimal::ZERO);
    let worst = ys.iter().copied().min().unwrap_or(Decimal::ZERO);
    let unbounded_profit = slope > 0;
    let unbounded_loss = slope < 0;

    let mut breakevens = Vec::new();
    for (i, y) in ys.iter().enumerate() {
        let zero_before = i > 0 && ys[i - 1].is_zero();
        let zero_after = ys.get(i + 1).is_some_and(Decimal::is_zero);
        if y.is_zero() && !(zero_before && zero_after) {
            breakevens.push(xs[i]);
        }
        if let (Some(x1), Some(y1)) = (xs.get(i + 1), ys.get(i + 1)) {
            let crosses = (*y < Decimal::ZERO && *y1 > Decimal::ZERO)
                || (*y > Decimal::ZERO && *y1 < Decimal::ZERO);
            if crosses {
                // multiply first, divide once: x_i + (-y * w) / (y1 - y)
                let x = x1
                    .checked_sub(xs[i])
                    .and_then(|w| (-*y).checked_mul(w))
                    .and_then(|num| num.checked_div(y1.checked_sub(*y)?))
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
        max_profit: (!unbounded_profit).then_some(best),
        max_loss: (!unbounded_loss).then_some((-worst).max(Decimal::ZERO)),
        unbounded_profit,
        unbounded_loss,
        has_naked_short,
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

    #[test]
    fn naked_flag() {
        let sp = expiry_payoff(&[leg(Put, "100", "4", -1)]).unwrap();
        assert!(sp.has_naked_short);
        let ic = expiry_payoff(&[
            leg(Put, "90", "1", 1),
            leg(Put, "95", "3", -1),
            leg(Call, "105", "3", -1),
            leg(Call, "110", "1", 1),
        ])
        .unwrap();
        assert!(!ic.has_naked_short);
        let lc = expiry_payoff(&[leg(Call, "100", "5", 1)]).unwrap();
        assert!(!lc.has_naked_short);
        let ratio = expiry_payoff(&[leg(Call, "100", "5", 1), leg(Call, "110", "1", -2)]).unwrap();
        assert!(ratio.has_naked_short && ratio.unbounded_loss);
    }

    #[test]
    fn breakeven_is_multiplied_before_divided() {
        // long 3 calls @100 (premium 1 each) and long 1 call @101 (premium 2):
        // pnl(100) = -3, pnl(101) = 3*(1-1) + (0-2) = -2 ... slope beyond 101 is 4: be = 101 + 2/4.
        // For a repeating fraction: pnl(100) = -2, pnl(101) = -1, ray slope 3 -> 101 + 1/3.
        let p = expiry_payoff(&[leg(Call, "100", "2", 1), leg(Call, "101", "0", 2)]).unwrap();
        let third = d("101") + d("1") / d("3");
        assert_eq!(p.breakevens.len(), 1);
        assert!((p.breakevens[0] - third).abs() < d("0.000000000000000000000001"));
        // interior crossing, exact: pnl(100) = -1, pnl(103) = +2 -> be = 100 + (1 * 3) / 3 = 101
        let q = expiry_payoff(&[leg(Call, "100", "2", 1), leg(Call, "103", "1", -1)]).unwrap();
        assert_eq!(q.breakevens, vec![d("101")]);
    }

    #[test]
    fn zero_plateau_keeps_only_its_ends() {
        let p = expiry_payoff(&[leg(Call, "100", "2", 1), leg(Call, "100", "2", -1)]).unwrap();
        assert_eq!(p.max_profit, Some(Decimal::ZERO));
        assert_eq!(p.max_loss, Some(Decimal::ZERO));
        assert_eq!(p.breakevens, vec![d("0"), d("100")]);
        let q = expiry_payoff(&[
            leg(Put, "90", "0", 1),
            leg(Put, "90", "0", -1),
            leg(Call, "100", "0", 1),
            leg(Call, "100", "0", -1),
            leg(Call, "110", "0", 1),
            leg(Call, "110", "0", -1),
        ])
        .unwrap();
        assert_eq!(q.breakevens, vec![d("0"), d("110")]);
    }

    #[test]
    fn leg_cap_and_always_losing() {
        let many = vec![leg(Call, "100", "1", 1); MAX_LEGS + 1];
        assert_eq!(expiry_payoff(&many), Err(PayoffError::TooManyLegs));
        let ok = vec![leg(Call, "100", "1", 1); MAX_LEGS];
        assert!(expiry_payoff(&ok).is_ok());
        // long 110 call @6, short 100 call @2 always loses: best P&L is negative, not clamped.
        let l = expiry_payoff(&[leg(Call, "110", "6", 1), leg(Call, "100", "2", -1)]).unwrap();
        assert!(l.max_profit.unwrap() < Decimal::ZERO);
    }

    fn sample_pnl(legs: &[Leg], s: Decimal) -> Decimal {
        legs.iter()
            .map(|l| {
                let intr = match l.kind {
                    Call => (s - l.strike).max(Decimal::ZERO),
                    Put => (l.strike - s).max(Decimal::ZERO),
                };
                Decimal::from(l.qty) * (intr - l.premium)
            })
            .sum()
    }

    proptest::proptest! {
        #[test]
        fn kink_method_matches_sampled_grid(
            raw in proptest::collection::vec(
                (proptest::bool::ANY, 50i64..150, 0i64..60, -3i32..=3), 1..5)
        ) {
            let legs: Vec<Leg> = raw
                .iter()
                .map(|(c, k, pr, q)| Leg {
                    kind: if *c { Call } else { Put },
                    strike: Decimal::from(*k),
                    premium: Decimal::from(*pr) / Decimal::from(2),
                    qty: *q,
                })
                .collect();
            let Ok(p) = expiry_payoff(&legs) else { return Ok(()); };
            // the integer grid 0..=1000 contains every kink
            let ys: Vec<Decimal> = (0..=1000).map(|s| sample_pnl(&legs, Decimal::from(s))).collect();
            let best = ys.iter().copied().max().unwrap();
            let worst = ys.iter().copied().min().unwrap();
            if !p.unbounded_profit {
                proptest::prop_assert_eq!(p.max_profit, Some(best));
            }
            if !p.unbounded_loss {
                proptest::prop_assert_eq!(p.max_loss, Some((-worst).max(Decimal::ZERO)));
            }
            let tiny = Decimal::new(1, 15);
            for be in &p.breakevens {
                proptest::prop_assert!(sample_pnl(&legs, *be).abs() < tiny, "be {} not zero", be);
            }
            for (w, pair) in ys.windows(2).enumerate() {
                let crosses = (pair[0] < Decimal::ZERO && pair[1] > Decimal::ZERO)
                    || (pair[0] > Decimal::ZERO && pair[1] < Decimal::ZERO);
                if crosses {
                    let lo = Decimal::from(w);
                    let hi = Decimal::from(w + 1);
                    proptest::prop_assert!(
                        p.breakevens.iter().any(|b| *b > lo && *b < hi),
                        "missing crossing in ({}, {})", lo, hi
                    );
                }
            }
        }
    }
}
