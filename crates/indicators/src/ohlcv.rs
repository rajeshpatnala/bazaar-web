//! f64 working copy of a `Candle`. `TryFrom<&Candle>` is the only Decimal to f64 conversion in
//! this crate; it rejects values that are not finite instead of propagating NaN.

use bazaar_core::candle::Candle;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use thiserror::Error;

/// A candle value could not be converted to a finite `f64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("candle field `{0}` is not representable as a finite f64")]
pub struct AdapterError(pub &'static str);

/// f64 view of a closed bar. `ts_ms` is the close time in Unix milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ohlcv {
    /// Close time, Unix milliseconds.
    pub ts_ms: i64,
    /// Open.
    pub o: f64,
    /// High.
    pub h: f64,
    /// Low.
    pub l: f64,
    /// Close.
    pub c: f64,
    /// Volume.
    pub v: f64,
}

fn conv(d: Decimal) -> Option<f64> {
    d.to_f64().filter(|x| x.is_finite())
}

impl TryFrom<&Candle> for Ohlcv {
    type Error = AdapterError;
    fn try_from(c: &Candle) -> Result<Self, AdapterError> {
        let f = |d: Decimal, name: &'static str| conv(d).ok_or(AdapterError(name));
        Ok(Self {
            ts_ms: c.close_time().timestamp_millis(),
            o: f(c.open(), "open")?,
            h: f(c.high(), "high")?,
            l: f(c.low(), "low")?,
            c: f(c.close(), "close")?,
            v: f64::from(c.volume().0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazaar_core::candle::Interval;
    use bazaar_core::money::Qty;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn converts_hand_case() {
        let t = Kolkata.with_ymd_and_hms(2026, 3, 2, 9, 16, 0).unwrap();
        let c = Candle::new(
            t,
            Interval::Min1,
            [d("100.5"), d("101"), d("100"), d("100.25")],
            Qty(7),
            false,
        )
        .unwrap();
        let o = Ohlcv::try_from(&c).unwrap();
        assert_eq!(o.ts_ms, t.timestamp_millis());
        assert!((o.o - 100.5).abs() < 1e-12);
        assert!((o.h - 101.0).abs() < 1e-12);
        assert!((o.l - 100.0).abs() < 1e-12);
        assert!((o.c - 100.25).abs() < 1e-12);
        assert!((o.v - 7.0).abs() < 1e-12);
    }

    #[test]
    fn conv_accepts_finite_values() {
        assert_eq!(conv(d("2.5")), Some(2.5));
        assert!(conv(Decimal::MAX).is_some_and(f64::is_finite));
    }

    #[test]
    fn error_names_field() {
        assert_eq!(
            AdapterError("open").to_string(),
            "candle field `open` is not representable as a finite f64"
        );
    }
}
