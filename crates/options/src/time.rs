//! Year fractions from IST datetimes passed in by the caller (no clock read here).

use chrono::NaiveDateTime;

/// Time conventions. The minutes-per-year choice is a modelling convention, not a market fact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeParams {
    /// Minutes in one year used as divisor; default 525 600 (365 days x 24 h x 60 min,
    /// calendar-time convention matching theta quoted per calendar day).
    pub minutes_per_year: f64,
    /// Floor for the returned fraction, so `T` is never exactly zero (default 1e-6 years, ~0.5 min).
    pub epsilon: f64,
}

impl Default for TimeParams {
    fn default() -> Self {
        Self {
            minutes_per_year: 525_600.0,
            epsilon: 1e-6,
        }
    }
}

/// `max(minutes(to - from) / minutes_per_year, epsilon)`. `from`/`to` are IST wall-clock datetimes
/// supplied by the caller (expiry from the calendar's close time). `to <= from` returns `epsilon`.
#[must_use]
pub fn year_fraction(from: NaiveDateTime, to: NaiveDateTime, p: &TimeParams) -> f64 {
    // i32 seconds cover ~68 years; saturate beyond that.
    let secs = i32::try_from((to - from).num_seconds()).unwrap_or(i32::MAX);
    let minutes = f64::from(secs) / 60.0;
    (minutes / p.minutes_per_year).max(p.epsilon)
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
        assert!((year_fraction(dt(1, 9, 15), dt(2, 9, 15), &p) - 1440.0 / 525_600.0).abs() < 1e-15);
        let a = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let b = NaiveDate::from_ymd_opt(2027, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert!((year_fraction(a, b, &p) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn floored_when_expired_or_reversed() {
        let p = TimeParams::default();
        assert_eq!(year_fraction(dt(2, 9, 15), dt(2, 9, 15), &p), p.epsilon);
        assert_eq!(year_fraction(dt(3, 9, 15), dt(2, 9, 15), &p), p.epsilon);
    }
}
