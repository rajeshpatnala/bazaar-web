//! Instrument master types: what the parser produces and the store holds.
//!
//! No market literal lives here; lot and tick sizes, expiries and eligibility all come from
//! the instrument master data.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Exchange an instrument trades on. Unknown codes are kept, never dropped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Exchange {
    Nse,
    Bse,
    Nfo,
    Bfo,
    Cds,
    Mcx,
    /// Any exchange code not listed above, upper-cased.
    Other(String),
}

impl Exchange {
    /// Parse an exchange code (case-insensitive); never fails.
    #[must_use]
    pub fn parse(code: &str) -> Self {
        let code = code.trim().to_ascii_uppercase();
        match code.as_str() {
            "NSE" => Self::Nse,
            "BSE" => Self::Bse,
            "NFO" => Self::Nfo,
            "BFO" => Self::Bfo,
            "CDS" => Self::Cds,
            "MCX" => Self::Mcx,
            _ => Self::Other(code),
        }
    }

    /// Canonical code, as stored.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Nse => "NSE",
            Self::Bse => "BSE",
            Self::Nfo => "NFO",
            Self::Bfo => "BFO",
            Self::Cds => "CDS",
            Self::Mcx => "MCX",
            Self::Other(code) => code,
        }
    }
}

/// Instrument type column of the master.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InstrumentType {
    Equity,
    Future,
    Call,
    Put,
    /// Any type not listed above, upper-cased.
    Other(String),
}

impl InstrumentType {
    /// Parse a type code (`EQ`, `FUT`, `CE`, `PE`; case-insensitive); never fails.
    #[must_use]
    pub fn parse(code: &str) -> Self {
        let code = code.trim().to_ascii_uppercase();
        match code.as_str() {
            "EQ" => Self::Equity,
            "FUT" => Self::Future,
            "CE" => Self::Call,
            "PE" => Self::Put,
            _ => Self::Other(code),
        }
    }

    /// Canonical code, as stored.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Equity => "EQ",
            Self::Future => "FUT",
            Self::Call => "CE",
            Self::Put => "PE",
            Self::Other(code) => code,
        }
    }

    /// True for futures and options.
    #[must_use]
    pub fn is_derivative(&self) -> bool {
        matches!(self, Self::Future | Self::Call | Self::Put)
    }
}

/// One row of the instrument master. Money-like fields are `Decimal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    pub instrument_token: u64,
    pub exchange_token: u64,
    pub tradingsymbol: String,
    /// Display name; for derivatives it is the underlying's name as the source gives it.
    pub name: String,
    pub exchange: Exchange,
    pub instrument_type: InstrumentType,
    /// Segment label as given by the source (kept verbatim).
    pub segment: String,
    pub expiry: Option<NaiveDate>,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub strike: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub tick_size: Decimal,
    pub lot_size: u32,
}

impl Instrument {
    /// Check the invariants the cleaning step enforces: positive tick and lot size.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.tick_size <= Decimal::ZERO {
            return Err(DomainError::InvalidTick);
        }
        if self.lot_size == 0 {
            return Err(DomainError::InvalidLotSize);
        }
        Ok(())
    }
}

/// Why a row was rejected by the cleaning step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    NonPositiveTick,
    NonPositiveLot,
    MalformedNumber,
    MalformedDate,
    MissingField,
    FieldTooLong,
}

/// Counts of rows kept and rejected by one parse; rows are never logged, only counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleaningReport {
    pub accepted: u64,
    pub rejected: u64,
    /// Rejections per reason, sorted by reason.
    pub reasons: Vec<(RejectReason, u64)>,
}

impl CleaningReport {
    /// Count one accepted row.
    pub fn accept(&mut self) {
        self.accepted = self.accepted.saturating_add(1);
    }

    /// Count one rejected row under `reason`.
    pub fn reject(&mut self, reason: RejectReason) {
        self.rejected = self.rejected.saturating_add(1);
        match self.reasons.binary_search_by_key(&reason, |(r, _)| *r) {
            Ok(i) => self.reasons[i].1 = self.reasons[i].1.saturating_add(1),
            Err(i) => self.reasons.insert(i, (reason, 1)),
        }
    }

    /// Total rows seen.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.accepted.saturating_add(self.rejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Instrument {
        Instrument {
            instrument_token: 1,
            exchange_token: 2,
            tradingsymbol: "ABC".into(),
            name: "ABC".into(),
            exchange: Exchange::Nse,
            instrument_type: InstrumentType::Equity,
            segment: "NSE".into(),
            expiry: None,
            strike: None,
            tick_size: Decimal::new(5, 2),
            lot_size: 1,
        }
    }

    #[test]
    fn exchange_parses_known_and_unknown() {
        assert_eq!(Exchange::parse(" nse "), Exchange::Nse);
        assert_eq!(Exchange::parse("xyz"), Exchange::Other("XYZ".into()));
        assert_eq!(Exchange::parse("xyz").as_str(), "XYZ");
        assert_eq!(Exchange::Mcx.as_str(), "MCX");
    }

    #[test]
    fn type_parses_and_falls_back_to_other() {
        assert_eq!(InstrumentType::parse("ce"), InstrumentType::Call);
        assert_eq!(InstrumentType::parse("FUT").as_str(), "FUT");
        let other = InstrumentType::parse("zz");
        assert_eq!(other, InstrumentType::Other("ZZ".into()));
        assert!(!other.is_derivative());
        assert!(InstrumentType::Put.is_derivative());
        assert!(!InstrumentType::Equity.is_derivative());
    }

    #[test]
    fn validate_rejects_bad_tick_and_lot() {
        assert_eq!(sample().validate(), Ok(()));
        let mut i = sample();
        i.tick_size = Decimal::ZERO;
        assert_eq!(i.validate(), Err(DomainError::InvalidTick));
        let mut i = sample();
        i.tick_size = Decimal::new(-5, 2);
        assert_eq!(i.validate(), Err(DomainError::InvalidTick));
        let mut i = sample();
        i.lot_size = 0;
        assert_eq!(i.validate(), Err(DomainError::InvalidLotSize));
    }

    #[test]
    fn report_counts_by_reason_sorted() {
        let mut r = CleaningReport::default();
        r.accept();
        r.reject(RejectReason::MalformedNumber);
        r.reject(RejectReason::NonPositiveTick);
        r.reject(RejectReason::MalformedNumber);
        assert_eq!(r.accepted, 1);
        assert_eq!(r.rejected, 3);
        assert_eq!(r.total(), 4);
        assert_eq!(
            r.reasons,
            vec![
                (RejectReason::NonPositiveTick, 1),
                (RejectReason::MalformedNumber, 2)
            ]
        );
    }

    #[test]
    fn decimal_strike_serialises_as_string() {
        let mut i = sample();
        i.strike = Some(Decimal::new(2_250_050, 2));
        let toml = toml::to_string(&i).unwrap();
        assert!(toml.contains("strike = \"22500.50\""), "{toml}");
        let back: Instrument = toml::from_str(&toml).unwrap();
        assert_eq!(back, i);
    }
}
