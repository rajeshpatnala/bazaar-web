//! Market segment and instrument kind. Variants match the `calendar.toml` schema in
//! `bz-india-markets`; do not add a second set.

use serde::{Deserialize, Serialize};

/// Exchange segment; trading holidays and sessions are per segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Segment {
    Equity,
    EquityDerivatives,
    Currency,
    Commodity,
}

/// Instrument kind, which decides the session end (closing auction vs none, derivatives).
///
/// The `all` value of `applies_to` in `calendar.toml` is a wildcard for the config loader,
/// not a kind of instrument, so it is not a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    CashNonFno,
    CashFno,
    Derivative,
    DerivativeExpiringIndex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Row {
        segment: Segment,
        applies_to: InstrumentKind,
    }

    #[test]
    fn parses_schema_spellings() {
        let row: Row =
            toml::from_str("segment = \"equity_derivatives\"\napplies_to = \"cash_fno\"").unwrap();
        assert_eq!(row.segment, Segment::EquityDerivatives);
        assert_eq!(row.applies_to, InstrumentKind::CashFno);
        let row: Row =
            toml::from_str("segment = \"commodity\"\napplies_to = \"derivative_expiring_index\"")
                .unwrap();
        assert_eq!(row.segment, Segment::Commodity);
        assert_eq!(row.applies_to, InstrumentKind::DerivativeExpiringIndex);
    }

    #[test]
    fn rejects_unknown_and_wildcard_values() {
        assert!(toml::from_str::<Row>("segment = \"slb\"\napplies_to = \"cash_fno\"").is_err());
        assert!(toml::from_str::<Row>("segment = \"equity\"\napplies_to = \"all\"").is_err());
    }

    #[test]
    fn serialises_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&Segment::Currency).unwrap(),
            "\"currency\""
        );
        assert_eq!(
            serde_json::to_string(&InstrumentKind::CashNonFno).unwrap(),
            "\"cash_non_fno\""
        );
    }
}
