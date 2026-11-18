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

/// Keep only finite values. Seam so the rejection path is testable.
fn finite(x: Option<f64>) -> Option<f64> {
    x.filter(|v| v.is_finite())
}

/// `Decimal::to_f64` always yields a finite value for in-range decimals, so the guard is
/// defensive: it keeps NaN/inf out should the conversion ever change. Precision beyond about
/// 15 significant digits is lost (harmless for tick-sized prices).
fn conv(d: Decimal) -> Option<f64> {
    finite(d.to_f64())
}

/// Volume as f64. Exact up to 2^53 (about 9e15), far above any real traded quantity; above that
/// it rounds to the nearest representable value, still finite.
#[allow(clippy::cast_precision_loss)]
const fn volume_f64(v: u64) -> f64 {
    v as f64
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
            v: volume_f64(c.volume()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazaar_core::candle::Interval;
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
            7,
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
    fn finite_rejects_nan_inf_and_none() {
        assert_eq!(finite(Some(1.5)), Some(1.5));
        assert_eq!(finite(Some(f64::NAN)), None);
        assert_eq!(finite(Some(f64::INFINITY)), None);
        assert_eq!(finite(Some(f64::NEG_INFINITY)), None);
        assert_eq!(finite(None), None);
    }

    #[test]
    fn large_volume_converts() {
        assert!((volume_f64(u64::from(u32::MAX) + 1) - 4_294_967_296.0).abs() < 1e-6);
    }

    #[test]
    fn error_names_field() {
        assert_eq!(
            AdapterError("open").to_string(),
            "candle field `open` is not representable as a finite f64"
        );
    }
}
