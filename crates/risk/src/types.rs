//! Order-side contracts: `Side`, `OrderIntent` and the tri-state `Tri`.

use bazaar_core::money::Qty;
use rust_decimal::Decimal;

/// Order direction. A position's side is the side that opened it (`Buy` = long).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    /// The other side.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }
}

/// A value that is either known or not known. `Unknown` is never defaulted: rules reject
/// entries when a required input is `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri<T> {
    Known(T),
    Unknown,
}

impl<T> Tri<T> {
    /// The known value, if any.
    #[must_use]
    pub const fn known(&self) -> Option<&T> {
        match self {
            Self::Known(v) => Some(v),
            Self::Unknown => None,
        }
    }

    /// True when the value is `Unknown`.
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// What the caller asks for. It carries no "exit" or "reduce-only" flag: the gate classifies.
/// No `Default`: every field is a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderIntent {
    pub symbol: String,
    pub side: Side,
    /// Quantity in shares/units. Zero is valid only together with `new_stop` (a stop move).
    pub qty: Qty,
    /// Limit or expected entry price.
    pub price: Decimal,
    /// Stop for the new/added quantity. An entry without a stop is rejected by the gate.
    pub stop: Option<Decimal>,
    /// Requested replacement for the stop of the held position (stop move).
    pub new_stop: Option<Decimal>,
    pub lot_size: u32,
    pub tick_size: Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_flips() {
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);
    }

    #[test]
    fn tri_known_and_unknown() {
        assert_eq!(Tri::Known(3).known(), Some(&3));
        assert!(!Tri::Known(3).is_unknown());
        assert_eq!(Tri::<u8>::Unknown.known(), None);
        assert!(Tri::<u8>::Unknown.is_unknown());
    }
}
