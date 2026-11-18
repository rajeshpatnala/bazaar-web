//! `Money`, `Price`, `Qty` and `Lots` over `Decimal`. No `f64` anywhere.
//!
//! Rounding policy (two modes, one per purpose):
//! - ledger amounts round to paise (2 dp) with banker's rounding (`MidpointNearestEven`), which
//!   is symmetric for profit and loss;
//! - tick snapping rounds to the nearest tick, exact halves away from zero
//!   (`MidpointAwayFromZero`), as a price ladder would.
//!
//! Caveats: banker's rounding can differ by one paisa from broker contract notes, which usually
//! round half up per charge line; reconcile per call site in the cost model if that matters.
//! Nearest-tick snapping is for normalising data only: order limits and stops need directional
//! rounding (floor/ceil to tick), to be added before any order path uses prices.
//! No operator impls except `Neg`: all other arithmetic is checked and returns `DomainError`.

use std::ops::Neg;
use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use crate::error::DomainError;

const PAISE_DP: u32 = 2;
/// Longest input prefix echoed in an `InvalidAmount` error.
const MAX_ECHO: usize = 32;

/// Rupee amount; may be negative (P&L). Serialised as a string. Never built from `f64`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Money(#[serde(with = "rust_decimal::serde::str")] Decimal);

impl Money {
    /// Zero rupees.
    pub const ZERO: Self = Self(Decimal::ZERO);

    /// Wrap an exact decimal amount.
    #[must_use]
    pub const fn new(d: Decimal) -> Self {
        Self(d)
    }

    /// The underlying decimal.
    #[must_use]
    pub const fn amount(self) -> Decimal {
        self.0
    }

    /// Round to paise (banker's rounding).
    #[must_use]
    pub fn round_paise(self) -> Self {
        Self(
            self.0
                .round_dp_with_strategy(PAISE_DP, RoundingStrategy::MidpointNearestEven),
        )
    }

    /// Checked sum.
    ///
    /// # Errors
    /// `Overflow` when the result does not fit.
    pub fn checked_add(self, o: Self) -> Result<Self, DomainError> {
        self.0
            .checked_add(o.0)
            .map(Self)
            .ok_or(DomainError::Overflow)
    }

    /// Checked difference.
    ///
    /// # Errors
    /// `Overflow` when the result does not fit.
    pub fn checked_sub(self, o: Self) -> Result<Self, DomainError> {
        self.0
            .checked_sub(o.0)
            .map(Self)
            .ok_or(DomainError::Overflow)
    }

    /// Checked product with a scalar (rate, fraction, quantity).
    ///
    /// # Errors
    /// `Overflow` when the result does not fit.
    pub fn checked_mul(self, k: Decimal) -> Result<Self, DomainError> {
        self.0.checked_mul(k).map(Self).ok_or(DomainError::Overflow)
    }
}

impl FromStr for Money {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::from_str(s)
            .map(Self)
            .map_err(|_| DomainError::InvalidAmount(s.chars().take(MAX_ECHO).collect()))
    }
}

impl Neg for Money {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// Non-negative price on an instrument, a multiple of that instrument's tick.
///
/// Deliberately has no `Deserialize`: a price must pass `snap_to_tick` (parse, do not validate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Price(#[serde(with = "rust_decimal::serde::str")] Decimal);

impl Price {
    /// Snap `raw` to the nearest multiple of `tick` (the tick comes from the instrument master).
    ///
    /// # Errors
    /// `InvalidTick` if `tick <= 0`; `NegativePrice` if `raw < 0`; `Overflow` on overflow.
    pub fn snap_to_tick(raw: Decimal, tick: Decimal) -> Result<Self, DomainError> {
        if tick <= Decimal::ZERO {
            return Err(DomainError::InvalidTick);
        }
        if raw < Decimal::ZERO {
            return Err(DomainError::NegativePrice);
        }
        let steps = raw
            .checked_div(tick)
            .ok_or(DomainError::Overflow)?
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
        steps
            .checked_mul(tick)
            .map(Self)
            .ok_or(DomainError::Overflow)
    }

    /// The underlying decimal.
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }

    /// Value of `qty` units at this price, rounded to paise.
    ///
    /// # Errors
    /// `Overflow` when the result does not fit.
    pub fn notional(self, qty: Qty) -> Result<Money, DomainError> {
        Money(self.0)
            .checked_mul(Decimal::from(qty.0))
            .map(Money::round_paise)
    }
}

/// Quantity in units.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Qty(pub u32);

/// Quantity in lots; convert to `Qty` with the lot size from the instrument master.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Lots(pub u32);

impl Lots {
    /// Units for this many lots.
    ///
    /// # Errors
    /// `InvalidLotSize` if `lot_size == 0`; `Overflow` if the product exceeds `u32`.
    pub fn to_qty(self, lot_size: u32) -> Result<Qty, DomainError> {
        if lot_size == 0 {
            return Err(DomainError::InvalidLotSize);
        }
        self.0
            .checked_mul(lot_size)
            .map(Qty)
            .ok_or(DomainError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn reference_value_notional_and_paise() {
        // 1234.55 x 75 = 92591.25 exactly
        let p = Price::snap_to_tick(d("1234.55"), d("0.05")).unwrap();
        assert_eq!(p.value(), d("1234.55"));
        assert_eq!(
            p.notional(Qty(75)).unwrap(),
            Money::from_str("92591.25").unwrap()
        );
    }

    #[test]
    fn paise_rounding_is_bankers() {
        let r = |s: &str| Money::from_str(s).unwrap().round_paise().amount();
        assert_eq!(r("0.125"), d("0.12"));
        assert_eq!(r("0.135"), d("0.14"));
        assert_eq!(r("-0.125"), d("-0.12"));
    }

    #[test]
    fn tick_snapping_edges() {
        let t = d("0.05");
        let s = |x: &str| Price::snap_to_tick(d(x), t).unwrap().value();
        assert_eq!(s("100.02"), d("100.00"));
        assert_eq!(s("100.03"), d("100.05"));
        assert_eq!(s("100.025"), d("100.05")); // exact half goes up
        assert_eq!(s("0"), Decimal::ZERO);
    }

    #[test]
    fn tick_snapping_errors() {
        assert_eq!(
            Price::snap_to_tick(d("1"), Decimal::ZERO),
            Err(DomainError::InvalidTick)
        );
        assert_eq!(
            Price::snap_to_tick(d("1"), d("-0.05")),
            Err(DomainError::InvalidTick)
        );
        assert_eq!(
            Price::snap_to_tick(d("-1"), d("0.05")),
            Err(DomainError::NegativePrice)
        );
    }

    #[test]
    fn money_arithmetic_and_negative_pnl() {
        let a = Money::from_str("100.10").unwrap();
        let b = Money::from_str("250.25").unwrap();
        assert_eq!(a.checked_sub(b).unwrap().amount(), d("-150.15"));
        assert_eq!((-a.checked_sub(b).unwrap()).amount(), d("150.15"));
        assert_eq!(a.checked_add(b).unwrap().amount(), d("350.35"));
        assert_eq!(a.checked_mul(d("2")).unwrap().amount(), d("200.20"));
        assert_eq!(Money::ZERO.checked_add(a).unwrap(), a);
    }

    #[test]
    fn overflow_is_an_error_not_a_panic() {
        let big = Money::new(Decimal::MAX);
        assert_eq!(big.checked_add(big), Err(DomainError::Overflow));
        assert_eq!(big.checked_mul(d("2")), Err(DomainError::Overflow));
        assert_eq!(
            big.checked_sub(Money::new(Decimal::MIN)),
            Err(DomainError::Overflow)
        );
        // division overflow in tick snapping
        assert_eq!(
            Price::snap_to_tick(Decimal::MAX, d("0.0001")),
            Err(DomainError::Overflow)
        );
    }

    #[test]
    fn negative_zero_is_accepted() {
        assert!(Price::snap_to_tick(d("-0.00"), d("0.05")).is_ok());
    }

    #[test]
    fn invalid_amount_echo_is_bounded() {
        let long = "x".repeat(10_000);
        let err = Money::from_str(&long).unwrap_err();
        assert_eq!(err, DomainError::InvalidAmount("x".repeat(32)));
    }

    #[test]
    fn money_parses_strings_only() {
        assert!(Money::from_str("12.5").is_ok());
        assert_eq!(
            Money::from_str("abc"),
            Err(DomainError::InvalidAmount("abc".into()))
        );
    }

    #[test]
    fn serde_uses_strings() {
        let m = Money::from_str("1234.50").unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"1234.50\"");
        let back: Money = serde_json::from_str("\"1234.50\"").unwrap();
        assert_eq!(back, m);
        assert!(serde_json::from_str::<Money>("1234.5").is_err());
        let p = Price::snap_to_tick(d("10.05"), d("0.05")).unwrap();
        assert_eq!(serde_json::to_string(&p).unwrap(), "\"10.05\"");
    }

    #[test]
    fn lots_to_qty() {
        assert_eq!(Lots(3).to_qty(75).unwrap(), Qty(225));
        assert_eq!(Lots(0).to_qty(75).unwrap(), Qty(0));
        assert_eq!(Lots(1).to_qty(0), Err(DomainError::InvalidLotSize));
        assert_eq!(Lots(u32::MAX).to_qty(2), Err(DomainError::Overflow));
    }

    proptest! {
        #[test]
        fn snapped_price_is_a_tick_multiple(raw in 0i64..10_000_000, tick_n in 1i64..500) {
            let raw = Decimal::new(raw, 3);
            let tick = Decimal::new(tick_n, 2);
            let p = Price::snap_to_tick(raw, tick).unwrap().value();
            prop_assert_eq!(p % tick, Decimal::ZERO);
            prop_assert!((p - raw).abs() <= tick / Decimal::TWO);
        }
    }
}
