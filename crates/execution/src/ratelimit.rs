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
///
/// Entries are held to all three caps. `reduce_only` exits skip the per-minute and per-day caps and
/// never touch the entry budget (RISK-105-2); they keep only the per-second loop guard, counted
/// in their own window.
#[derive(Debug)]
pub struct RateLimiter {
    per_sec: usize,
    per_min: usize,
    per_day: usize,
    hits: Mutex<Hits>,
}

#[derive(Debug, Default)]
struct Hits {
    entries: VecDeque<DateTime<Tz>>,
    exits: VecDeque<DateTime<Tz>>,
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
            hits: Mutex::new(Hits::default()),
        })
    }

    /// Record one order at `now` if every cap allows it. A refused call records nothing.
    ///
    /// # Errors
    /// `ExecError::RateLimited` when a cap is reached (or the state is poisoned: fail closed).
    pub fn try_acquire(&self, now: DateTime<Tz>, reduce_only: bool) -> Result<(), ExecError> {
        self.decide(now, reduce_only, true)
    }

    /// Same decision as `try_acquire`, recording nothing (for the pipeline's pre-check).
    ///
    /// # Errors
    /// `ExecError::RateLimited` as for `try_acquire`.
    pub fn peek(&self, now: DateTime<Tz>, reduce_only: bool) -> Result<(), ExecError> {
        self.decide(now, reduce_only, false)
    }

    fn decide(&self, now: DateTime<Tz>, reduce_only: bool, record: bool) -> Result<(), ExecError> {
        let mut h = self.hits.lock().map_err(|_| ExecError::RateLimited)?;
        let today = now.date_naive();
        let minute = Duration::seconds(60);
        let second = Duration::seconds(1);
        h.entries
            .retain(|t| t.date_naive() == today || now.signed_duration_since(*t) < minute);
        h.exits.retain(|t| now.signed_duration_since(*t) < second);
        if reduce_only {
            if h.exits.len() >= self.per_sec {
                return Err(ExecError::RateLimited);
            }
            if record {
                h.exits.push_back(now);
            }
            return Ok(());
        }
        let in_window = |w: Duration| {
            h.entries
                .iter()
                .filter(|t| now.signed_duration_since(**t) < w)
                .count()
        };
        let day = h.entries.iter().filter(|t| t.date_naive() == today).count();
        if in_window(second) >= self.per_sec
            || in_window(minute) >= self.per_min
            || day >= self.per_day
        {
            return Err(ExecError::RateLimited);
        }
        if record {
            h.entries.push_back(now);
        }
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
        assert!(l.try_acquire(at(10, 0, 0), false).is_ok());
        assert_eq!(
            l.try_acquire(at(10, 0, 0), false),
            Err(ExecError::RateLimited)
        );
        assert!(l.try_acquire(at(10, 0, 1), false).is_ok());
    }

    #[test]
    fn minute_cap_applies_with_a_high_per_second_cap() {
        let l = RateLimiter::new(100, 4, 100).unwrap();
        for s in 0..4 {
            assert!(l.try_acquire(at(10, 0, s), false).is_ok());
        }
        assert_eq!(
            l.try_acquire(at(10, 0, 5), false),
            Err(ExecError::RateLimited)
        );
        assert!(l.try_acquire(at(10, 1, 0), false).is_ok()); // first hit is 60 s old
    }

    #[test]
    fn day_cap_applies_and_resets_at_ist_midnight() {
        let l = RateLimiter::new(100, 100, 12).unwrap();
        for i in 0..12 {
            assert!(l.try_acquire(at(9 + i / 4, (i % 4) * 10, 0), false).is_ok());
        }
        assert_eq!(
            l.try_acquire(at(15, 0, 0), false),
            Err(ExecError::RateLimited)
        );
        let next = Kolkata.with_ymd_and_hms(2026, 11, 20, 0, 0, 5).unwrap();
        assert!(l.try_acquire(next, false).is_ok());
    }

    #[test]
    fn refused_calls_consume_nothing() {
        let l = RateLimiter::new(100, 2, 100).unwrap();
        l.try_acquire(at(10, 0, 0), false).unwrap();
        l.try_acquire(at(10, 0, 1), false).unwrap();
        for _ in 0..5 {
            assert!(l.try_acquire(at(10, 0, 2), false).is_err());
        }
        // Only the two accepted hits count: both age out together.
        assert!(l.try_acquire(at(10, 1, 2), false).is_ok());
    }

    #[test]
    fn exit_passes_at_minute_and_day_cap_and_leaves_entry_budget() {
        let l = RateLimiter::new(100, 2, 2).unwrap();
        l.try_acquire(at(10, 0, 0), false).unwrap();
        l.try_acquire(at(10, 0, 1), false).unwrap();
        assert!(l.try_acquire(at(10, 0, 2), false).is_err()); // entries at minute and day cap
        for s in 2..12 {
            assert!(l.try_acquire(at(10, 0, s), true).is_ok());
        }
        // Exits consumed no entry budget: after the minute slides, the day cap still holds
        // exactly the two entries (not more, not fewer).
        assert!(l.try_acquire(at(10, 2, 0), false).is_err());
        let l = RateLimiter::new(100, 2, 100).unwrap();
        l.try_acquire(at(10, 0, 0), true).unwrap();
        l.try_acquire(at(10, 0, 1), true).unwrap();
        l.try_acquire(at(10, 0, 2), true).unwrap();
        assert!(l.try_acquire(at(10, 0, 3), false).is_ok());
        assert!(l.try_acquire(at(10, 0, 4), false).is_ok());
        assert!(l.try_acquire(at(10, 0, 5), false).is_err());
    }

    #[test]
    fn exit_keeps_only_the_per_second_loop_guard() {
        let l = RateLimiter::new(2, 100, 100).unwrap();
        assert!(l.try_acquire(at(10, 0, 0), true).is_ok());
        assert!(l.try_acquire(at(10, 0, 0), true).is_ok());
        assert_eq!(
            l.try_acquire(at(10, 0, 0), true),
            Err(ExecError::RateLimited)
        );
        assert!(l.try_acquire(at(10, 0, 1), true).is_ok());
        // Entries have their own per-second window.
        assert!(l.try_acquire(at(10, 0, 1), false).is_ok());
    }

    #[test]
    fn peek_consumes_nothing() {
        let l = RateLimiter::new(100, 1, 100).unwrap();
        for _ in 0..5 {
            assert!(l.peek(at(10, 0, 0), false).is_ok());
        }
        assert!(l.try_acquire(at(10, 0, 0), false).is_ok());
        assert_eq!(l.peek(at(10, 0, 1), false), Err(ExecError::RateLimited));
    }

    #[test]
    fn clock_going_backwards_counts_as_in_window() {
        let l = RateLimiter::new(1, 100, 100).unwrap();
        l.try_acquire(at(10, 0, 5), false).unwrap();
        assert_eq!(
            l.try_acquire(at(10, 0, 4), false),
            Err(ExecError::RateLimited)
        );
    }

    #[test]
    fn concurrent_acquires_never_exceed_the_cap() {
        let l = std::sync::Arc::new(RateLimiter::new(100, 4, 100).unwrap());
        let ok: usize = (0..8)
            .map(|_| {
                let l = l.clone();
                std::thread::spawn(move || l.try_acquire(at(10, 0, 0), false).is_ok())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum();
        assert_eq!(ok, 4);
    }

    #[test]
    fn zero_cap_is_an_error() {
        assert!(RateLimiter::new(0, 1, 1).is_err());
        assert!(RateLimiter::new(1, 0, 1).is_err());
        assert!(RateLimiter::new(1, 1, 0).is_err());
    }
}
