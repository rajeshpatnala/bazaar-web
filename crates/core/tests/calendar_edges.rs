//! Calendar edge tests over the synthetic fixture calendar (F1 deferred TS-21-1..4, TS-21-6).
//! The fixture is fictional; nothing here asserts a real market fact.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bazaar_core::calendar::{CalendarError, MarketCalendar, Session};
use bazaar_core::config::{CalendarConfig, load};
use bazaar_core::segment::{InstrumentKind, Segment};
use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use chrono_tz::Asia::Kolkata;
use proptest::prelude::*;

const SEGMENTS: [Segment; 4] = [
    Segment::Equity,
    Segment::EquityDerivatives,
    Segment::Currency,
    Segment::Commodity,
];

fn cal() -> MarketCalendar {
    let cfg: CalendarConfig = load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/config"
    )))
    .unwrap();
    MarketCalendar::from_config(&cfg)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn ist(y: i32, m: u32, day: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
    Kolkata
        .with_ymd_and_hms(y, m, day, h, mi, s)
        .unwrap()
        .with_timezone(&Utc)
}

fn utc(y: i32, m: u32, day: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, day, h, mi, s).unwrap()
}

proptest! {
    /// TS-21-1: shifting is idempotent, never later, and lands on a trading day, for every
    /// segment; `session_at` is Closed on a non-trading day.
    #[test]
    fn shift_and_session_properties(day in 0u64..365, seg_i in 0usize..4) {
        let c = cal();
        let seg = SEGMENTS[seg_i];
        let x = d(2026, 1, 1) + Days::new(day);
        let s = c.shift_to_prior_trading_day(x, seg);
        // Early-January days may need to look before the covered year.
        if let Ok(s) = s {
            prop_assert!(s <= x);
            prop_assert!(c.is_trading_day(s, seg).unwrap());
            prop_assert_eq!(c.shift_to_prior_trading_day(s, seg).unwrap(), s);
        } else {
            prop_assert_eq!(s, Err(CalendarError::OutOfRange(d(2025, 12, 31))));
        }
        if !c.is_trading_day(x, seg).unwrap() {
            // 03:00 IST avoids any special-session window in the fixture.
            let t = ist(2026, x.month0() + 1, x.day0() + 1, 3, 0, 0);
            prop_assert_eq!(
                c.session_at(t, seg, InstrumentKind::CashNonFno).unwrap(),
                Session::Closed
            );
        }
    }

    /// TS-21-1: `prev(next(x)) == x` for a trading day `x` with a later trading day in range.
    #[test]
    fn prev_of_next_is_identity(day in 0u64..364, seg_i in 0usize..4) {
        let c = cal();
        let seg = SEGMENTS[seg_i];
        let x = d(2026, 1, 1) + Days::new(day);
        if c.is_trading_day(x, seg).unwrap()
            && let Ok(n) = c.next_trading_day(x, seg)
        {
            prop_assert_eq!(c.prev_trading_day(n, seg).unwrap(), x);
        }
    }
}

use chrono::Datelike;

/// TS-21-2: a Saturday without a special session is Closed all day.
#[test]
fn saturday_is_closed() {
    let c = cal();
    let sat = d(2026, 1, 10);
    for h in [0, 10, 12, 18] {
        assert_eq!(
            c.session_at(
                ist(2026, 1, sat.day(), h, 0, 0),
                Segment::Equity,
                InstrumentKind::CashNonFno
            ),
            Ok(Session::Closed)
        );
    }
}

/// TS-21-3: the muhurat window is start-inclusive, end-exclusive.
#[test]
fn special_session_edges() {
    let c = cal();
    let at = |h, m, s| {
        c.session_at(
            ist(2026, 11, 7, h, m, s),
            Segment::Equity,
            InstrumentKind::CashNonFno,
        )
        .unwrap()
    };
    assert_eq!(at(17, 59, 59), Session::Closed);
    assert_eq!(at(18, 0, 0), Session::Special("muhurat".into()));
    assert_eq!(at(18, 59, 59), Session::Special("muhurat".into()));
    assert_eq!(at(19, 0, 0), Session::Closed);
}

/// TS-21-4: sessions are read in IST: 18:29:59Z is still 23:59:59 on the IST day and
/// 18:30:00Z is the next IST day.
#[test]
fn utc_ist_day_boundary() {
    let c = cal();
    let q = |t| c.session_at(t, Segment::Equity, InstrumentKind::CashNonFno);
    assert_eq!(q(utc(2026, 1, 5, 18, 29, 59)), Ok(Session::Closed));
    assert_eq!(q(utc(2026, 1, 5, 18, 30, 0)), Ok(Session::Closed));
    // The IST date flips before the UTC date does: 10:00 IST on 6 Jan is 04:30Z, Normal.
    assert_eq!(q(utc(2026, 1, 6, 4, 29, 59)), Ok(Session::PreOpen));
    assert_eq!(q(utc(2026, 1, 6, 4, 30, 0)), Ok(Session::Normal));
    // Last covered instant vs first uncovered instant.
    assert_eq!(q(utc(2026, 12, 31, 18, 29, 59)), Ok(Session::Closed));
    assert_eq!(
        q(utc(2026, 12, 31, 18, 30, 0)),
        Err(CalendarError::OutOfRange(d(2027, 1, 1)))
    );
}

/// TS-21-6: queries for an uncovered year are `OutOfRange`.
#[test]
fn out_of_range_for_close_time_and_shift() {
    let c = cal();
    let out = d(2027, 1, 5);
    assert_eq!(
        c.close_time(out, Segment::Equity, InstrumentKind::CashNonFno),
        Err(CalendarError::OutOfRange(out))
    );
    assert_eq!(
        c.shift_to_prior_trading_day(out, Segment::Equity),
        Err(CalendarError::OutOfRange(out))
    );
    assert!(matches!(
        c.next_trading_day(d(2026, 12, 31), Segment::Equity),
        Err(CalendarError::OutOfRange(_))
    ));
}
