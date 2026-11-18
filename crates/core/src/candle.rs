//! `Candle` (closed OHLCV bar) and `Interval` (Kite bar sizes).
//!
//! A `Candle` is always a *closed* bar: `close_time` is when the bar ended, so consumers that only
//! see candles cannot look ahead. Prices are `Decimal`; the constructor and `Deserialize` enforce
//! `low <= open, close <= high` and non-negative prices.

use chrono::{DateTime, FixedOffset, SecondsFormat};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::DomainError;
use crate::money::Qty;

/// Bar size. These are Kite API protocol constants, not market facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interval {
    /// 1 minute.
    Min1,
    /// 3 minutes.
    Min3,
    /// 5 minutes.
    Min5,
    /// 10 minutes.
    Min10,
    /// 15 minutes.
    Min15,
    /// 30 minutes.
    Min30,
    /// 60 minutes.
    Min60,
    /// One trading session.
    Day,
}

impl Interval {
    /// Length in minutes for intraday intervals; `None` for `Day` (its length is the session's).
    #[must_use]
    pub const fn minutes(self) -> Option<u32> {
        match self {
            Self::Min1 => Some(1),
            Self::Min3 => Some(3),
            Self::Min5 => Some(5),
            Self::Min10 => Some(10),
            Self::Min15 => Some(15),
            Self::Min30 => Some(30),
            Self::Min60 => Some(60),
            Self::Day => None,
        }
    }
}

/// A closed OHLCV bar. Fields are private so every value passed validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candle {
    #[serde(serialize_with = "ist_time::serialize")]
    close_time: DateTime<Tz>,
    interval: Interval,
    #[serde(with = "rust_decimal::serde::str")]
    open: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    high: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    low: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    close: Decimal,
    volume: Qty,
    adjusted: bool,
}

/// RFC 3339 on the wire, always read back in `Asia/Kolkata`.
mod ist_time {
    use super::{DateTime, Deserialize, Deserializer, FixedOffset, Kolkata, SecondsFormat, Tz};
    use serde::Serializer;

    pub(super) fn serialize<S: Serializer>(t: &DateTime<Tz>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Tz>, D::Error> {
        let raw = String::deserialize(d)?;
        let t: DateTime<FixedOffset> =
            DateTime::parse_from_rfc3339(&raw).map_err(serde::de::Error::custom)?;
        Ok(t.with_timezone(&Kolkata))
    }
}

#[derive(Deserialize)]
struct RawCandle {
    #[serde(deserialize_with = "ist_time::deserialize")]
    close_time: DateTime<Tz>,
    interval: Interval,
    #[serde(with = "rust_decimal::serde::str")]
    open: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    high: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    low: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    close: Decimal,
    volume: Qty,
    adjusted: bool,
}

impl<'de> Deserialize<'de> for Candle {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = RawCandle::deserialize(d)?;
        Self::new(
            r.close_time,
            r.interval,
            [r.open, r.high, r.low, r.close],
            r.volume,
            r.adjusted,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl Candle {
    /// Build a validated candle. `ohlc` is `[open, high, low, close]`; `volume` is a `Qty`
    /// (unsigned, so it cannot be negative).
    ///
    /// # Errors
    /// `NegativePrice` if any price is negative; `InvalidCandle` if `high < low` or open/close
    /// fall outside `[low, high]`.
    pub fn new(
        close_time: DateTime<Tz>,
        interval: Interval,
        ohlc: [Decimal; 4],
        volume: Qty,
        adjusted: bool,
    ) -> Result<Self, DomainError> {
        let [open, high, low, close] = ohlc;
        if ohlc.iter().any(|p| *p < Decimal::ZERO) {
            return Err(DomainError::NegativePrice);
        }
        if high < low {
            return Err(DomainError::InvalidCandle("high below low"));
        }
        if open < low || open > high {
            return Err(DomainError::InvalidCandle("open outside [low, high]"));
        }
        if close < low || close > high {
            return Err(DomainError::InvalidCandle("close outside [low, high]"));
        }
        Ok(Self {
            close_time,
            interval,
            open,
            high,
            low,
            close,
            volume,
            adjusted,
        })
    }

    /// When the bar closed (IST).
    #[must_use]
    pub const fn close_time(&self) -> DateTime<Tz> {
        self.close_time
    }
    /// Bar size.
    #[must_use]
    pub const fn interval(&self) -> Interval {
        self.interval
    }
    /// Opening price.
    #[must_use]
    pub const fn open(&self) -> Decimal {
        self.open
    }
    /// Highest price.
    #[must_use]
    pub const fn high(&self) -> Decimal {
        self.high
    }
    /// Lowest price.
    #[must_use]
    pub const fn low(&self) -> Decimal {
        self.low
    }
    /// Closing price.
    #[must_use]
    pub const fn close(&self) -> Decimal {
        self.close
    }
    /// Traded quantity.
    #[must_use]
    pub const fn volume(&self) -> Qty {
        self.volume
    }
    /// Whether prices are corporate-action adjusted.
    #[must_use]
    pub const fn adjusted(&self) -> bool {
        self.adjusted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn t() -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 3, 2, 9, 16, 0).unwrap()
    }

    fn mk(o: &str, h: &str, l: &str, c: &str) -> Result<Candle, DomainError> {
        Candle::new(
            t(),
            Interval::Min1,
            [d(o), d(h), d(l), d(c)],
            Qty(10),
            false,
        )
    }

    #[test]
    fn valid_candle_and_accessors() {
        let c = mk("100", "102.5", "99.5", "101").unwrap();
        assert_eq!(c.high(), d("102.5"));
        assert_eq!(c.low(), d("99.5"));
        assert_eq!(c.open(), d("100"));
        assert_eq!(c.close(), d("101"));
        assert_eq!(c.volume(), Qty(10));
        assert_eq!(c.interval(), Interval::Min1);
        assert!(!c.adjusted());
        assert_eq!(c.close_time(), t());
    }

    #[test]
    fn flat_candle_is_valid() {
        assert!(mk("5", "5", "5", "5").is_ok());
    }

    #[test]
    fn rejects_invalid_shapes() {
        let hl = Err(DomainError::InvalidCandle("high below low"));
        let op = Err(DomainError::InvalidCandle("open outside [low, high]"));
        let cl = Err(DomainError::InvalidCandle("close outside [low, high]"));
        assert_eq!(mk("100", "99", "101", "100"), hl);
        assert_eq!(mk("98", "102", "99", "100"), op);
        assert_eq!(mk("103", "102", "99", "100"), op);
        assert_eq!(mk("100", "102", "99", "98"), cl);
        assert_eq!(mk("100", "102", "99", "103"), cl);
        assert_eq!(mk("-1", "2", "-2", "0"), Err(DomainError::NegativePrice));
    }

    #[test]
    fn serde_round_trips_exactly() {
        let c = mk("100.10", "102.50", "99.05", "101.00").unwrap();
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("+05:30"));
        let back: Candle = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.close().to_string(), "101.00");
    }

    #[test]
    fn deserialize_validates() {
        let c = mk("100", "102", "99", "101").unwrap();
        let json = serde_json::to_string(&c)
            .unwrap()
            .replace("\"high\":\"102\"", "\"high\":\"98\"");
        assert!(serde_json::from_str::<Candle>(&json).is_err());
    }

    #[test]
    fn deserialize_normalises_to_ist() {
        let json = r#"{"close_time":"2026-03-02T03:46:00Z","interval":"min1","open":"1","high":"1","low":"1","close":"1","volume":0,"adjusted":true}"#;
        let c: Candle = serde_json::from_str(json).unwrap();
        assert_eq!(c.close_time(), t());
        assert_eq!(c.close_time().timezone(), Kolkata);
    }

    #[test]
    fn interval_minutes() {
        assert_eq!(Interval::Min1.minutes(), Some(1));
        assert_eq!(Interval::Min3.minutes(), Some(3));
        assert_eq!(Interval::Min5.minutes(), Some(5));
        assert_eq!(Interval::Min10.minutes(), Some(10));
        assert_eq!(Interval::Min15.minutes(), Some(15));
        assert_eq!(Interval::Min30.minutes(), Some(30));
        assert_eq!(Interval::Min60.minutes(), Some(60));
        assert_eq!(Interval::Day.minutes(), None);
    }
}
