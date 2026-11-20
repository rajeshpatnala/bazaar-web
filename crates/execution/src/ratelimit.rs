//! Order-rate limiter: per-second, per-minute and per-IST-day caps over sliding windows.
//!
//! Pure and driven by the `now` the caller passes (from the injected `Clock`); it never sleeps.
//! All three caps always apply, so a high per-second cap cannot hide the minute or day cap.

use std::collections::VecDeque;
use std::sync::Mutex;

use chrono::{DateTime, Duration};
use chrono_tz::Tz;

use crate::executor::ExecError;

/// Three-window order cap. Caps come from `execution.max_orders_per_second` and
/// `limits.max_orders_per_minute` / `max_orders_per_day`.
#[derive(Debug)]
pub struct RateLimiter {
    per_sec: usize,
    per_min: usize,
    per_day: usize,
    hits: Mutex<VecDeque<DateTime<Tz>>>,
}

impl RateLimiter {
    /// # Errors
    /// `ExecError::InvalidInput` when any cap is 0.
    pub fn new(per_sec: u32, per_min: u32, per_day: u32) -> Result<Self, ExecError> {
        if per_sec == 0 || per_min == 0 || per_day == 0 {
            return Err(ExecError::InvalidInput("rate cap must be above 0"));
        }
        Ok(Self {
            per_sec: per_sec as usize,
            per_min: per_min as usize,
            per_day: per_day as usize,
            hits: Mutex::new(VecDeque::new()),
        })
    }

    /// Record one order at `now` if every cap allows it. A refused call records nothing.
    ///
    /// # Errors
    /// `ExecError::RateLimited` when a cap is reached (or the state is poisoned: fail closed).
    pub fn try_acquire(&self, now: DateTime<Tz>) -> Result<(), ExecError> {
        let mut hits = self.hits.lock().map_err(|_| ExecError::RateLimited)?;
        let today = now.date_naive();
        let minute = Duration::seconds(60);
        hits.retain(|t| t.date_naive() == today || now.signed_duration_since(*t) < minute);
        let in_window = |w: Duration| {
            hits.iter()
                .filter(|t| now.signed_duration_since(**t) < w)
                .count()
        };
        let day = hits.iter().filter(|t| t.date_naive() == today).count();
        if in_window(Duration::seconds(1)) >= self.per_sec
            || in_window(minute) >= self.per_min
            || day >= self.per_day
        {
            return Err(ExecError::RateLimited);
        }
        hits.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    fn at(h: u32, m: u32, s: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 11, 19, h, m, s).unwrap()
    }

    #[test]
    fn one_per_second_and_window_slides() {
        let l = RateLimiter::new(1, 100, 100).unwrap();
        assert!(l.try_acquire(at(10, 0, 0)).is_ok());
        assert_eq!(l.try_acquire(at(10, 0, 0)), Err(ExecError::RateLimited));
        assert!(l.try_acquire(at(10, 0, 1)).is_ok());
    }

    #[test]
    fn minute_cap_applies_with_a_high_per_second_cap() {
        let l = RateLimiter::new(100, 4, 100).unwrap();
        for s in 0..4 {
            assert!(l.try_acquire(at(10, 0, s)).is_ok());
        }
        assert_eq!(l.try_acquire(at(10, 0, 5)), Err(ExecError::RateLimited));
        assert!(l.try_acquire(at(10, 1, 0)).is_ok()); // first hit is 60 s old
    }

    #[test]
    fn day_cap_applies_and_resets_at_ist_midnight() {
        let l = RateLimiter::new(100, 100, 12).unwrap();
        for i in 0..12 {
            assert!(l.try_acquire(at(9 + i / 4, (i % 4) * 10, 0)).is_ok());
        }
        assert_eq!(l.try_acquire(at(15, 0, 0)), Err(ExecError::RateLimited));
        let next = Kolkata.with_ymd_and_hms(2026, 11, 20, 0, 0, 5).unwrap();
        assert!(l.try_acquire(next).is_ok());
    }

    #[test]
    fn refused_calls_consume_nothing() {
        let l = RateLimiter::new(100, 2, 100).unwrap();
        l.try_acquire(at(10, 0, 0)).unwrap();
        l.try_acquire(at(10, 0, 1)).unwrap();
        for _ in 0..5 {
            assert!(l.try_acquire(at(10, 0, 2)).is_err());
        }
        // Only the two accepted hits count: both age out together.
        assert!(l.try_acquire(at(10, 1, 2)).is_ok());
    }

    #[test]
    fn zero_cap_is_an_error() {
        assert!(RateLimiter::new(0, 1, 1).is_err());
        assert!(RateLimiter::new(1, 0, 1).is_err());
        assert!(RateLimiter::new(1, 1, 0).is_err());
    }
}
