//! Pure options maths: pricing, Greeks, IV, payoff; no I/O, no clock, no network.
//!
//! `f64` is used only inside this maths; callers convert money at one boundary.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use bazaar_core as _;

pub mod black76;
pub mod iv;
pub mod norm;
pub mod payoff;
pub mod time;

/// Call or put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    /// Right to buy.
    Call,
    /// Right to sell.
    Put,
}

#[cfg(test)]
mod tests {
    use super::OptionKind;

    #[test]
    fn kinds_are_distinct_and_copy() {
        let k = OptionKind::Call;
        let k2 = k;
        assert_eq!(k, k2);
        assert_ne!(k, OptionKind::Put);
    }
}
