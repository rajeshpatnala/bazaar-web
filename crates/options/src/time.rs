//! Year fractions from IST datetimes passed in by the caller (no clock read here).

use chrono::NaiveDateTime;

/// Default minutes per year: 365 x 24 x 60, the calendar-time convention (matches theta per
/// calendar day). A modelling choice, not a market fact.
pub const DEFAULT_MINUTES_PER_YEAR: f64 = 525_600.0;
/// Default floor for `T` in years (about half a minute).
pub const DEFAULT_EPSILON_YEARS: f64 = 1e-6;

/// Invalid [`TimeParams`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    /// `minutes_per_year` or `epsilon` is not finite and strictly positive.
    InvalidParams,
}

/// Time conventions. Build with [`TimeParams::new`] (validated) or `Default`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeParams {
    /// Minutes in one year used as divisor (default [`DEFAULT_MINUTES_PER_YEAR`]).
    pub minutes_per_year: f64,
    /// Floor for the returned fraction so `T` is never zero (default [`DEFAULT_EPSILON_YEARS`]).
    pub epsilon: f64,
}

impl TimeParams {
    /// Validated constructor: both fields must be finite and `> 0`.
    ///
    /// # Errors
    /// [`TimeError::InvalidParams`] otherwise.
    pub fn new(minutes_per_year: f64, epsilon: f64) -> Result<Self, TimeError> {
        let ok = |x: f64| x.is_finite() && x > 0.0;
        if ok(minutes_per_year) && ok(epsilon) {
            Ok(Self {
                minutes_per_year,
                epsilon,
            })
        } else {
            Err(TimeError::InvalidParams)
        }
    }
}

impl Default for TimeParams {
    fn default() -> Self {
        Self {
            minutes_per_year: DEFAULT_MINUTES_PER_YEAR,
            epsilon: DEFAULT_EPSILON_YEARS,
        }
    }
}

/// `max(minutes(to - from) / minutes_per_year, epsilon)`. `from`/`to` are IST wall-clock
/// datetimes by project convention (`NaiveDateTime` carries no zone; never pass UTC), supplied
/// by the caller (expiry from the calendar's close time). `to <= from` returns `epsilon`.
/// Spans beyond ~68 years saturate.
///
/// # Errors
/// [`TimeError::InvalidParams`] if `p` fails the [`TimeParams::new`] checks.
pub fn year_fraction(
    from: NaiveDateTime,
    to: NaiveDateTime,
    p: &TimeParams,
) -> Result<f64, TimeError> {
    let p = TimeParams::new(p.minutes_per_year, p.epsilon)?;
    if to <= from {
        return Ok(p.epsilon);
    }
    let secs = i32::try_from((to - from).num_seconds()).unwrap_or(i32::MAX);
    Ok((f64::from(secs) / 60.0 / p.minutes_per_year).max(p.epsilon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn dt(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, d)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
    }

    #[test]
    fn one_day_and_one_year() {
        let p = TimeParams::default();
        assert!(
            (year_fraction(dt(1, 9, 15), dt(2, 9, 15), &p).unwrap() - 1440.0 / 525_600.0).abs()
                < 1e-15
        );
        let a = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let b = NaiveDate::from_ymd_opt(2027, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert!((year_fraction(a, b, &p).unwrap() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn floored_when_expired_or_reversed() {
        let p = TimeParams::default();
        assert_eq!(year_fraction(dt(2, 9, 15), dt(2, 9, 15), &p), Ok(p.epsilon));
        assert_eq!(year_fraction(dt(3, 9, 15), dt(2, 9, 15), &p), Ok(p.epsilon));
    }

    #[test]
    fn reversed_by_decades_returns_epsilon() {
        let p = TimeParams::default();
        let old = NaiveDate::from_ymd_opt(1970, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(year_fraction(dt(1, 9, 15), old, &p), Ok(p.epsilon));
    }

    #[test]
    fn invalid_params_are_rejected() {
        for (m, e) in [
            (0.0, 1e-6),
            (-1.0, 1e-6),
            (f64::NAN, 1e-6),
            (525_600.0, 0.0),
            (525_600.0, f64::NAN),
            (f64::INFINITY, 1e-6),
        ] {
            assert_eq!(TimeParams::new(m, e), Err(TimeError::InvalidParams));
            let bad = TimeParams {
                minutes_per_year: m,
                epsilon: e,
            };
            assert_eq!(
                year_fraction(dt(1, 9, 15), dt(2, 9, 15), &bad),
                Err(TimeError::InvalidParams)
            );
        }
        assert!(TimeParams::new(525_600.0, 1e-6).is_ok());
    }
}
