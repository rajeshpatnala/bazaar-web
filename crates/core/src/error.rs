//! Domain error enum shared by the pure domain types.

use thiserror::Error;

/// Violations of domain invariants (money, prices, quantities, sessions).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    /// Tick size was zero or negative.
    #[error("tick size must be positive")]
    InvalidTick,
    /// Lot size was zero.
    #[error("lot size must be positive")]
    InvalidLotSize,
    /// A price was negative.
    #[error("negative price")]
    NegativePrice,
    /// A quantity was negative.
    #[error("negative quantity or price")]
    NegativeQuantity,
    /// A timestamp fell outside the trading session.
    #[error("outside trading session")]
    OutsideSession,
    /// A decimal string could not be parsed.
    #[error("invalid decimal amount: {0}")]
    InvalidAmount(String),
    /// Arithmetic overflowed.
    #[error("arithmetic overflow")]
    Overflow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_stable() {
        assert_eq!(
            DomainError::InvalidTick.to_string(),
            "tick size must be positive"
        );
        assert_eq!(
            DomainError::InvalidAmount("x".into()).to_string(),
            "invalid decimal amount: x"
        );
        assert_eq!(DomainError::Overflow.to_string(), "arithmetic overflow");
    }
}
