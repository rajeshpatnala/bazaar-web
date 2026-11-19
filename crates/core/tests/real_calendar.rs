//! The shipped `config/calendar.toml` (NSE 2026, owner-supplied) answers through `MarketCalendar`.
//! Expected dates come from the owner's pasted page (docs/owner-input/F2-calendar.md).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bazaar_core::calendar::{CalendarError, MarketCalendar, Session};
use bazaar_core::config::{CalendarConfig, load};
use bazaar_core::segment::{InstrumentKind, Segment};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Asia::Kolkata;

fn cfg() -> CalendarConfig {
    load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config"
    )))
    .unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn ist(date: NaiveDate, h: u32, mi: u32) -> DateTime<Utc> {
    Kolkata
        .from_local_datetime(&date.and_hms_opt(h, mi, 0).unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn shipped_calendar_is_real_dated_and_sourced() {
    let c = cfg();
    assert!(!c.placeholder);
    assert_eq!(c.verified_on, Some(d(2026, 11, 20)));
    assert_eq!(c.covered_years, vec![2026]);
    assert!(c.source.contains("nseindia"));
    assert!(c.session.iter().all(|s| !s.source.trim().is_empty()));
    assert!(
        c.special_session
            .iter()
            .all(|s| !s.source.trim().is_empty())
    );
    assert_eq!(c.holiday.len(), 20); // 16 weekday + 4 weekend holidays
}

#[test]
fn every_listed_weekday_holiday_is_closed_and_neighbours_open() {
    let cal = MarketCalendar::from_config(&cfg());
    let weekday = [
        (1, 15),
        (1, 26),
        (3, 3),
        (3, 26),
        (3, 31),
        (4, 3),
        (4, 14),
        (5, 1),
        (5, 28),
        (6, 26),
        (9, 14),
        (10, 2),
        (10, 20),
        (11, 10),
        (11, 24),
        (12, 25),
    ];
    for (m, day) in weekday {
        let h = d(2026, m, day);
        assert!(!cal.is_trading_day(h, Segment::Equity).unwrap(), "{h}");
        assert_eq!(
            cal.session_at(ist(h, 10, 0), Segment::Equity, InstrumentKind::CashFno)
                .unwrap(),
            Session::Closed
        );
    }
    // Wed 2026-01-14 and Fri 2026-01-16 are ordinary days.
    assert!(cal.is_trading_day(d(2026, 1, 14), Segment::Equity).unwrap());
    assert!(cal.is_trading_day(d(2026, 1, 16), Segment::Equity).unwrap());
}

#[test]
fn weekday_session_phases_come_from_the_file() {
    let cal = MarketCalendar::from_config(&cfg());
    let day = d(2026, 1, 14);
    let at = |h, m| {
        cal.session_at(ist(day, h, m), Segment::Equity, InstrumentKind::CashNonFno)
            .unwrap()
    };
    assert_eq!(at(8, 59), Session::Closed);
    assert_eq!(at(9, 0), Session::PreOpen);
    assert_eq!(at(9, 14), Session::PreOpen);
    assert_eq!(at(9, 15), Session::Normal);
    assert_eq!(at(15, 29), Session::Normal);
    assert_eq!(at(15, 30), Session::Closed);
    assert_eq!(
        cal.close_time(day, Segment::Equity, InstrumentKind::CashFno)
            .unwrap(),
        ist(day, 15, 30)
    );
}

#[test]
fn weekend_holidays_stay_closed_and_muhurat_opens_the_sunday() {
    let cal = MarketCalendar::from_config(&cfg());
    for (m, day) in [(2, 15), (3, 21), (8, 15)] {
        let h = d(2026, m, day);
        assert!(!cal.is_trading_day(h, Segment::Equity).unwrap());
    }
    let m = d(2026, 11, 8);
    assert!(cal.is_trading_day(m, Segment::Equity).unwrap());
    // Timings are UNVERIFIED in the file; this checks the file is honoured, not the real times.
    assert_eq!(
        cal.session_at(ist(m, 14, 30), Segment::Equity, InstrumentKind::CashFno)
            .unwrap(),
        Session::Special("muhurat".to_owned())
    );
    assert_eq!(
        cal.session_at(ist(m, 16, 0), Segment::Equity, InstrumentKind::CashFno)
            .unwrap(),
        Session::Closed
    );
}

#[test]
fn other_years_are_out_of_range() {
    let cal = MarketCalendar::from_config(&cfg());
    assert_eq!(
        cal.is_trading_day(d(2027, 1, 4), Segment::Equity),
        Err(CalendarError::OutOfRange(d(2027, 1, 4)))
    );
}
