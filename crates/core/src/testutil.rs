//! `FakeClock` and `fixture_path`, behind the `test-util` feature.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Duration};
use chrono_tz::Tz;

use crate::clock::Clock;

/// Deterministic clock for tests; move it with `advance`/`set`, never `sleep`.
#[derive(Debug)]
pub struct FakeClock(Mutex<DateTime<Tz>>);

impl FakeClock {
    /// Clock frozen at `t`.
    #[must_use]
    pub fn at(t: DateTime<Tz>) -> Self {
        Self(Mutex::new(t))
    }

    /// Move the clock forward (or back, for a negative duration).
    pub fn advance(&self, d: Duration) {
        let mut g = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *g += d;
    }

    /// Replace the current instant.
    pub fn set(&self, t: DateTime<Tz>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = t;
    }
}

impl Clock for FakeClock {
    fn now_ist(&self) -> DateTime<Tz> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Path under the repo-root `fixtures/` tree, valid for every caller.
#[must_use]
pub fn fixture_path(rel: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    #[test]
    fn advance_and_set() {
        let t = Kolkata.with_ymd_and_hms(2026, 1, 5, 9, 15, 0).unwrap();
        let c = FakeClock::at(t);
        assert_eq!(c.now_ist(), t);
        c.advance(Duration::minutes(15));
        assert_eq!(
            c.now_ist(),
            Kolkata.with_ymd_and_hms(2026, 1, 5, 9, 30, 0).unwrap()
        );
        c.set(t);
        assert_eq!(c.now_ist(), t);
    }

    #[test]
    fn fixture_path_is_under_fixtures() {
        let p = fixture_path("config/risk.toml");
        assert!(p.ends_with("fixtures/config/risk.toml"));
    }
}
