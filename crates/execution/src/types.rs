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

/// Longest symbol a stored payload may carry.
const MAX_SYMBOL: usize = 500;

impl Product {
    /// Parse a stored label.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cnc" => Some(Self::Cnc),
            "mis" => Some(Self::Mis),
            "nrml" => Some(Self::Nrml),
            _ => None,
        }
    }
}

impl OrderIntent {
    /// The full intent as the stored order payload (restart reload decodes it). Rupee amounts
    /// are strings, never floats.
    #[must_use]
    pub fn to_payload(&self) -> String {
        let (kind, price) = match self.kind {
            OrderKind::Market => ("market", None),
            OrderKind::Limit(p) => ("limit", Some(p.to_string())),
        };
        serde_json::json!({
            "symbol": self.symbol,
            "side": match self.side { Side::Buy => "buy", Side::Sell => "sell" },
            "qty": self.qty,
            "kind": kind,
            "price": price,
            "product": self.product.as_str(),
            "reduce_only": self.reduce_only,
            "created_at": self.created_at.to_rfc3339(),
        })
        .to_string()
    }

    /// Decode a stored payload; `None` for anything missing, malformed, from an older row that
    /// did not carry the full intent, or semantically invalid (qty 0, empty or over-long symbol,
    /// limit price not above 0, `created_at` after `now`, which would stretch the TTL). The
    /// caller expires or blocks such a row, audited.
    #[must_use]
    pub fn from_payload(id: IntentId, payload: &str, now: DateTime<Tz>) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(payload).ok()?;
        let side = match v.get("side")?.as_str()? {
            "buy" => Side::Buy,
            "sell" => Side::Sell,
            _ => return None,
        };
        let kind = match v.get("kind")?.as_str()? {
            "market" => OrderKind::Market,
            "limit" => OrderKind::Limit(v.get("price")?.as_str()?.parse().ok()?),
            _ => return None,
        };
        let qty = u32::try_from(v.get("qty")?.as_u64()?).ok()?;
        let created_at = DateTime::parse_from_rfc3339(v.get("created_at")?.as_str()?)
            .ok()?
            .with_timezone(&chrono_tz::Asia::Kolkata);
        let symbol = v.get("symbol")?.as_str()?.to_owned();
        let bad_price = matches!(kind, OrderKind::Limit(p) if p <= Decimal::ZERO);
        if qty == 0
            || symbol.is_empty()
            || symbol.len() > MAX_SYMBOL
            || bad_price
            || created_at > now
        {
            return None;
        }
        Some(Self {
            id,
            symbol,
            side,
            qty,
            kind,
            product: Product::parse(v.get("product")?.as_str()?)?,
            reduce_only: v.get("reduce_only")?.as_bool()?,
            created_at,
        })
    }
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

    #[test]
    fn payload_round_trips_and_rejects_incomplete_rows() {
        use chrono::TimeZone;
        let t = chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 23, 9, 15, 0)
            .unwrap();
        for kind in [OrderKind::Market, OrderKind::Limit(Decimal::new(10125, 2))] {
            let i = OrderIntent {
                id: IntentId(9),
                symbol: "ABC".into(),
                side: Side::Sell,
                qty: 3,
                kind,
                product: Product::Nrml,
                reduce_only: true,
                created_at: t,
            };
            assert_eq!(
                OrderIntent::from_payload(IntentId(9), &i.to_payload(), t),
                Some(i)
            );
        }
        // An old-style row (no side, kind, created_at) and garbage do not decode.
        let old = r#"{"symbol":"ABC","qty":2,"reduce_only":false,"product":"mis"}"#;
        assert_eq!(OrderIntent::from_payload(IntentId(1), old, t), None);
        assert_eq!(OrderIntent::from_payload(IntentId(1), "nope", t), None);
        assert_eq!(OrderIntent::from_payload(IntentId(1), "", t), None);
    }

    #[test]
    fn payload_with_invalid_values_is_rejected() {
        use chrono::TimeZone;
        let t = chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 23, 9, 15, 0)
            .unwrap();
        let base = OrderIntent {
            id: IntentId(1),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 1,
            kind: OrderKind::Limit(Decimal::ONE),
            product: Product::Mis,
            reduce_only: false,
            created_at: t,
        };
        let ok = |i: &OrderIntent| OrderIntent::from_payload(IntentId(1), &i.to_payload(), t);
        assert!(ok(&base).is_some());
        let mut i = base.clone();
        i.qty = 0;
        assert!(ok(&i).is_none());
        let mut i = base.clone();
        i.symbol = String::new();
        assert!(ok(&i).is_none());
        let mut i = base.clone();
        i.symbol = "X".repeat(501);
        assert!(ok(&i).is_none());
        for price in [Decimal::ZERO, Decimal::NEGATIVE_ONE] {
            let mut i = base.clone();
            i.kind = OrderKind::Limit(price);
            assert!(ok(&i).is_none());
        }
        // Created in the future: would never reach its TTL.
        let mut i = base;
        i.created_at = t + chrono::Duration::seconds(1);
        assert!(ok(&i).is_none());
    }
}
