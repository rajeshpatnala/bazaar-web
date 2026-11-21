//! Ids and the intent that enters the pipeline.

use std::fmt;

use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

pub use bazaar_risk::types::Side;

/// 128-bit intent id from the injected `IdGen`; displayed as 32 hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IntentId(pub u128);

impl fmt::Display for IntentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// Broker product type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// Delivery.
    Cnc,
    /// Intraday; squared off at the configured IST time in paper.
    Mis,
    /// Carry-forward derivatives.
    Nrml,
}

impl Product {
    /// Stable label used in storage and the margin port.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cnc => "cnc",
            Self::Mis => "mis",
            Self::Nrml => "nrml",
        }
    }
}

/// Order type. A market order has no price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    Market,
    Limit(Decimal),
}

/// One order proposal. Whether it may proceed is decided by the `DecisionGate`, not by this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderIntent {
    pub id: IntentId,
    pub symbol: String,
    pub side: Side,
    pub qty: u32,
    pub kind: OrderKind,
    pub product: Product,
    /// An exit that only shrinks a position; the sole order allowed while the kill switch is on.
    pub reduce_only: bool,
    pub created_at: DateTime<Tz>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_id_displays_as_32_hex_digits() {
        assert_eq!(IntentId(255).to_string(), format!("{}ff", "0".repeat(30)));
        assert_eq!(IntentId(u128::MAX).to_string().len(), 32);
    }

    #[test]
    fn product_labels_are_stable() {
        assert_eq!(Product::Cnc.as_str(), "cnc");
        assert_eq!(Product::Mis.as_str(), "mis");
        assert_eq!(Product::Nrml.as_str(), "nrml");
    }
}
