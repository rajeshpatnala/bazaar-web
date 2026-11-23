//! `GuardedQuotes`: every quote source the runtime uses sits behind this wrapper, so a future
//! adapter (BZ-410) cannot hand the re-gate a hung, stale, future-dated or nonsensical quote.
//!
//! A quote is returned only when: the read finished within `io_timeout`; its time parses and is
//! not after the `Clock`'s now and not older than `max_quote_age` (G15); `last > 0`; and `last`
//! is not outside its own circuits (a crossed pair of circuits is refused too). Anything else is
//! `Unavailable`, so the re-gate blocks. Time comes from the injected `Clock`, never the wall.

use std::sync::Arc;
use std::time::Duration;

use bazaar_core::clock::Clock;
use bazaar_core::ports::{QuoteSnapshot, QuoteSource, SourceError};
use chrono::DateTime;
use rust_decimal::Decimal;

/// A quote source with the timeout, age, sanity and circuit checks applied.
pub struct GuardedQuotes<Q> {
    inner: Q,
    clock: Arc<dyn Clock>,
    io_timeout: Duration,
    max_age: chrono::Duration,
}

impl<Q> std::fmt::Debug for GuardedQuotes<Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedQuotes").finish_non_exhaustive()
    }
}

impl<Q> GuardedQuotes<Q> {
    /// `io_timeout` is `execution.io_timeout()`, `max_quote_age_secs` is `data.max_quote_age_secs`.
    pub fn new(
        inner: Q,
        clock: Arc<dyn Clock>,
        io_timeout: Duration,
        max_quote_age_secs: u32,
    ) -> Self {
        Self {
            inner,
            clock,
            io_timeout,
            max_age: chrono::Duration::seconds(i64::from(max_quote_age_secs)),
        }
    }

    fn check(&self, q: &QuoteSnapshot) -> Result<(), SourceError> {
        let at = DateTime::parse_from_rfc3339(&q.at_ist_rfc3339)
            .map_err(|_| SourceError::Unavailable)?;
        let now = self.clock.now_ist();
        if at > now || now.signed_duration_since(at) > self.max_age {
            return Err(SourceError::Unavailable);
        }
        if q.last <= Decimal::ZERO {
            return Err(SourceError::Unavailable);
        }
        if let (Some(lo), Some(hi)) = (q.lower_circuit, q.upper_circuit)
            && lo > hi
        {
            return Err(SourceError::Unavailable);
        }
        if q.lower_circuit.is_some_and(|lo| q.last < lo)
            || q.upper_circuit.is_some_and(|hi| q.last > hi)
        {
            return Err(SourceError::Unavailable);
        }
        Ok(())
    }
}

impl<Q: QuoteSource> QuoteSource for GuardedQuotes<Q> {
    async fn quote(&self, symbol: &str) -> Result<QuoteSnapshot, SourceError> {
        let q = tokio::time::timeout(self.io_timeout, self.inner.quote(symbol))
            .await
            .map_err(|_| SourceError::Unavailable)??;
        self.check(&q)?;
        Ok(q)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]
mod tests {
    use bazaar_core::testutil::FakeClock;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    use super::*;

    struct Fixed(Result<QuoteSnapshot, SourceError>);
    impl QuoteSource for Fixed {
        async fn quote(&self, _: &str) -> Result<QuoteSnapshot, SourceError> {
            self.0.clone()
        }
    }

    struct Hangs;
    impl QuoteSource for Hangs {
        async fn quote(&self, _: &str) -> Result<QuoteSnapshot, SourceError> {
            std::future::pending().await
        }
    }

    fn clock() -> Arc<FakeClock> {
        Arc::new(FakeClock::at(
            Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap(),
        ))
    }

    fn quote(c: &FakeClock, age_secs: i64, last: &str) -> QuoteSnapshot {
        QuoteSnapshot {
            last: last.parse().unwrap(),
            lower_circuit: None,
            upper_circuit: None,
            bar_volume: None,
            at_ist_rfc3339: (c.now_ist() - chrono::Duration::seconds(age_secs)).to_rfc3339(),
        }
    }

    fn guard(c: &Arc<FakeClock>, q: QuoteSnapshot) -> GuardedQuotes<Fixed> {
        GuardedQuotes::new(Fixed(Ok(q)), c.clone(), Duration::from_secs(5), 5)
    }

    #[tokio::test]
    async fn a_good_quote_passes_unchanged() {
        let c = clock();
        let mut q = quote(&c, 5, "50");
        q.lower_circuit = Some("45".parse().unwrap());
        q.upper_circuit = Some("55".parse().unwrap());
        assert_eq!(guard(&c, q.clone()).quote("X").await, Ok(q));
    }

    #[tokio::test]
    async fn each_bad_quote_is_refused() {
        let c = clock();
        let dec = |s: &str| Some(s.parse::<Decimal>().unwrap());
        let mut crossed = quote(&c, 0, "50");
        (crossed.lower_circuit, crossed.upper_circuit) = (dec("60"), dec("40"));
        let mut below = quote(&c, 0, "44");
        below.lower_circuit = dec("45");
        let mut above = quote(&c, 0, "56");
        above.upper_circuit = dec("55");
        let mut garbled = quote(&c, 0, "50");
        garbled.at_ist_rfc3339 = "yesterday".into();
        let cases = [
            ("stale by one second", quote(&c, 6, "50")),
            ("in the future", quote(&c, -1, "50")),
            ("zero last", quote(&c, 0, "0")),
            ("negative last", quote(&c, 0, "-1")),
            ("below lower circuit", below),
            ("above upper circuit", above),
            ("crossed circuits", crossed),
            ("unparseable time", garbled),
        ];
        for (name, q) in cases {
            assert_eq!(
                guard(&c, q).quote("X").await,
                Err(SourceError::Unavailable),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn age_follows_the_injected_clock_not_the_wall() {
        let c = clock();
        let g = guard(&c, quote(&c, 0, "50"));
        assert!(g.quote("X").await.is_ok());
        c.advance(chrono::Duration::seconds(6));
        assert_eq!(g.quote("X").await, Err(SourceError::Unavailable));
    }

    #[tokio::test]
    async fn a_source_error_and_a_hang_both_end_in_err() {
        let c = clock();
        let g = GuardedQuotes::new(
            Fixed(Err(SourceError::Unavailable)),
            c.clone(),
            Duration::from_secs(5),
            5,
        );
        assert_eq!(g.quote("X").await, Err(SourceError::Unavailable));
        let g = GuardedQuotes::new(Hangs, c, Duration::from_millis(20), 5);
        assert_eq!(g.quote("X").await, Err(SourceError::Unavailable));
    }
}
