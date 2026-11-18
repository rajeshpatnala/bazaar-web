//! The one `Clock` trait. All time-dependent code takes `&dyn Clock`; no wall-clock reads elsewhere.
//!
//! `SystemClock` is the production clock; the test-only `FakeClock` is in `testutil` (feature `test-util`).

use chrono::DateTime;
use chrono_tz::Tz;

/// Source of the current time in `Asia/Kolkata`.
pub trait Clock: Send + Sync {
    /// Current instant in Asia/Kolkata.
    fn now_ist(&self) -> DateTime<Tz>;
}

/// Production clock: the single wall-clock read in the workspace.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    #[allow(clippy::disallowed_methods)]
    fn now_ist(&self) -> DateTime<Tz> {
        chrono::Utc::now().with_timezone(&chrono_tz::Asia::Kolkata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    struct Fixed(DateTime<Tz>);

    impl Clock for Fixed {
        fn now_ist(&self) -> DateTime<Tz> {
            self.0
        }
    }

    #[test]
    fn clock_is_object_safe_and_returns_ist() {
        let t = Kolkata.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let clock: &dyn Clock = &Fixed(t);
        assert_eq!(clock.now_ist(), t);
        assert_eq!(clock.now_ist().timezone(), Kolkata);
    }

    #[test]
    fn system_clock_reports_ist() {
        let c: &dyn Clock = &SystemClock;
        assert_eq!(c.now_ist().timezone(), Kolkata);
    }
}
