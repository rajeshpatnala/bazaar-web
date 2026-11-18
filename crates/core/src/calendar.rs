//! `MarketCalendar` built from `calendar.toml` ([`CalendarConfig`]).
//!
//! Pure: it holds no clock; callers pass instants (`DateTime<Utc>`, typically from a `Clock`).
//! Sessions, special sessions and holidays come only from the config file. The only calendar
//! fact in Rust is that Saturday and Sunday are not regular trading days.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Asia::Kolkata;

use crate::config::{CalendarConfig, HolidayKind, Phase, SessionRow, SpecialKind};
use crate::segment::{InstrumentKind, Segment};

/// Market phase at an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    Closed,
    PreOpen,
    Normal,
    ClosingAuction,
    /// A special session window (for example `"muhurat"`, `"mock"`).
    Special(String),
}

/// Calendar query failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CalendarError {
    /// The date's year is not in `covered_years` of calendar.toml.
    #[error("date {0} is outside the years covered by calendar.toml")]
    OutOfRange(NaiveDate),
    /// A session-based query was made for a day with no trading.
    #[error("{0} is not a trading day for this segment")]
    NotTradingDay(NaiveDate),
    /// No session window in calendar.toml applies on this date.
    #[error("no session window for {segment:?}/{kind:?} on {date}")]
    NoSession {
        date: NaiveDate,
        segment: Segment,
        kind: InstrumentKind,
    },
    /// A local IST time could not be mapped to an instant.
    #[error("invalid local time on {0}")]
    InvalidLocalTime(NaiveDate),
}

#[derive(Debug, Clone)]
struct SpecialDay {
    kind: SpecialKind,
    start: NaiveTime,
    end: NaiveTime,
}

/// Holidays, sessions and special sessions for the covered years.
#[derive(Debug, Clone)]
pub struct MarketCalendar {
    covered_years: HashSet<i32>,
    holidays: HashSet<(NaiveDate, Segment)>,
    specials: HashMap<(NaiveDate, Segment), SpecialDay>,
    sessions: Vec<SessionRow>,
}

impl MarketCalendar {
    /// Build from a loaded (already validated) calendar config.
    #[must_use]
    pub fn from_config(cfg: &CalendarConfig) -> Self {
        let mut holidays = HashSet::new();
        for h in cfg
            .holiday
            .iter()
            .filter(|h| h.kind == HolidayKind::Trading)
        {
            for seg in &h.segments {
                holidays.insert((h.date, *seg));
            }
        }
        let mut specials = HashMap::new();
        for s in &cfg.special_session {
            for seg in &s.segments {
                specials.insert(
                    (s.date, *seg),
                    SpecialDay {
                        kind: s.kind,
                        start: s.start,
                        end: s.end,
                    },
                );
            }
        }
        Self {
            covered_years: cfg.covered_years.iter().copied().collect(),
            holidays,
            specials,
            sessions: cfg.session.clone(),
        }
    }

    fn ensure_covered(&self, d: NaiveDate) -> Result<(), CalendarError> {
        if self.covered_years.contains(&d.year()) {
            Ok(())
        } else {
            Err(CalendarError::OutOfRange(d))
        }
    }

    /// A mock session never makes a day tradable; any other special session does.
    fn tradable_special(&self, d: NaiveDate, seg: Segment) -> Option<&SpecialDay> {
        self.specials
            .get(&(d, seg))
            .filter(|s| s.kind != SpecialKind::Mock)
    }

    /// Is the exchange open for trading on `d` for `seg`? A special session opens a weekend
    /// or holiday; a clearing holiday does not close anything; a mock session does not open.
    ///
    /// # Errors
    /// `OutOfRange` when the year is not in calendar.toml.
    pub fn is_trading_day(&self, d: NaiveDate, seg: Segment) -> Result<bool, CalendarError> {
        self.ensure_covered(d)?;
        if self.tradable_special(d, seg).is_some() {
            return Ok(true);
        }
        let weekend = matches!(d.weekday(), Weekday::Sat | Weekday::Sun);
        Ok(!weekend && !self.holidays.contains(&(d, seg)))
    }

    fn windows(
        &self,
        d: NaiveDate,
        seg: Segment,
        kind: InstrumentKind,
    ) -> impl Iterator<Item = &SessionRow> {
        self.sessions.iter().filter(move |s| {
            s.segment == seg
                && s.applies_to.covers(kind)
                && s.effective_from <= d
                && s.effective_to.is_none_or(|t| d <= t)
        })
    }

    /// Phase at an instant for one segment and instrument kind.
    ///
    /// # Errors
    /// `OutOfRange` when the IST date's year is not in calendar.toml.
    pub fn session_at(
        &self,
        t: DateTime<Utc>,
        seg: Segment,
        kind: InstrumentKind,
    ) -> Result<Session, CalendarError> {
        let local = t.with_timezone(&Kolkata);
        let (d, time) = (local.date_naive(), local.time());
        self.ensure_covered(d)?;
        if let Some(sp) = self.specials.get(&(d, seg))
            && sp.start <= time
            && time < sp.end
        {
            return Ok(Session::Special(special_name(sp.kind).to_owned()));
        }
        if !self.is_trading_day(d, seg)? {
            return Ok(Session::Closed);
        }
        let hit = self
            .windows(d, seg, kind)
            .find(|w| w.start <= time && time < w.end);
        Ok(match hit.map(|w| w.phase) {
            Some(Phase::PreOpen) => Session::PreOpen,
            Some(Phase::Normal) => Session::Normal,
            Some(Phase::ClosingAuction) => Session::ClosingAuction,
            Some(Phase::PostClose) | None => Session::Closed,
        })
    }

    /// End of the last session of the day for the segment and kind: option expiry time and
    /// bar cut-off. A special session extends the day if it ends later.
    ///
    /// # Errors
    /// `OutOfRange`, `NotTradingDay`, or `NoSession` when no window applies.
    pub fn close_time(
        &self,
        d: NaiveDate,
        seg: Segment,
        kind: InstrumentKind,
    ) -> Result<DateTime<Utc>, CalendarError> {
        if !self.is_trading_day(d, seg)? {
            return Err(CalendarError::NotTradingDay(d));
        }
        let regular = self
            .windows(d, seg, kind)
            .filter(|w| w.phase != Phase::PostClose)
            .map(|w| w.end)
            .max();
        let special = self.tradable_special(d, seg).map(|s| s.end);
        let end = regular.max(special).ok_or(CalendarError::NoSession {
            date: d,
            segment: seg,
            kind,
        })?;
        Kolkata
            .from_local_datetime(&d.and_time(end))
            .single()
            .map(|t| t.with_timezone(&Utc))
            .ok_or(CalendarError::InvalidLocalTime(d))
    }

    /// First trading day strictly after `d`.
    ///
    /// # Errors
    /// `OutOfRange` when the search leaves the covered years.
    pub fn next_trading_day(&self, d: NaiveDate, seg: Segment) -> Result<NaiveDate, CalendarError> {
        let mut x = d;
        loop {
            x = x.succ_opt().ok_or(CalendarError::OutOfRange(x))?;
            if self.is_trading_day(x, seg)? {
                return Ok(x);
            }
        }
    }

    /// Last trading day strictly before `d`.
    ///
    /// # Errors
    /// `OutOfRange` when the search leaves the covered years.
    pub fn prev_trading_day(&self, d: NaiveDate, seg: Segment) -> Result<NaiveDate, CalendarError> {
        let mut x = d;
        loop {
            x = x.pred_opt().ok_or(CalendarError::OutOfRange(x))?;
            if self.is_trading_day(x, seg)? {
                return Ok(x);
            }
        }
    }

    /// An expiry on a non-trading day moves back to the prior trading day; a trading day is
    /// returned unchanged.
    ///
    /// # Errors
    /// `OutOfRange` when the search leaves the covered years.
    pub fn shift_to_prior_trading_day(
        &self,
        mut d: NaiveDate,
        seg: Segment,
    ) -> Result<NaiveDate, CalendarError> {
        while !self.is_trading_day(d, seg)? {
            d = d.pred_opt().ok_or(CalendarError::OutOfRange(d))?;
        }
        Ok(d)
    }
}

fn special_name(k: SpecialKind) -> &'static str {
    match k {
        SpecialKind::Muhurat => "muhurat",
        SpecialKind::Budget => "budget",
        SpecialKind::Other => "other",
        SpecialKind::Mock => "mock",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Clock;
    use crate::config::load;
    use crate::segment::InstrumentKind as K;
    use crate::segment::Segment as S;
    use chrono_tz::Tz;
    use proptest::prelude::*;
    use std::path::Path;

    struct Fixed(DateTime<Tz>);

    impl Clock for Fixed {
        fn now_ist(&self) -> DateTime<Tz> {
            self.0
        }
    }

    fn cal() -> MarketCalendar {
        let cfg: CalendarConfig = load(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/config"
        )))
        .unwrap();
        MarketCalendar::from_config(&cfg)
    }

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    fn ist(m: u32, day: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Kolkata
            .with_ymd_and_hms(2026, m, day, h, mi, s)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn non_fno_cash_boundaries() {
        let c = cal();
        let at = |h, m, s| {
            c.session_at(ist(1, 6, h, m, s), S::Equity, K::CashNonFno)
                .unwrap()
        };
        assert_eq!(at(9, 39, 59), Session::Closed);
        assert_eq!(at(9, 40, 0), Session::PreOpen);
        assert_eq!(at(9, 59, 59), Session::PreOpen);
        assert_eq!(at(10, 0, 0), Session::Normal);
        assert_eq!(at(13, 59, 59), Session::Normal);
        assert_eq!(at(14, 0, 0), Session::Closed);
    }

    #[test]
    fn fno_closing_auction_and_regime_change() {
        let c = cal();
        let at = |m, day, h, mi| {
            c.session_at(ist(m, day, h, mi, 0), S::Equity, K::CashFno)
                .unwrap()
        };
        assert_eq!(at(1, 6, 14, 0), Session::ClosingAuction);
        assert_eq!(at(1, 6, 14, 20), Session::Closed);
        // from 2026-07-01 the auction runs to 14:30
        assert_eq!(at(7, 1, 14, 20), Session::ClosingAuction);
        assert_eq!(at(7, 1, 14, 30), Session::Closed);
        let fno = c.close_time(d(1, 6), S::Equity, K::CashFno).unwrap();
        let non_fno = c.close_time(d(1, 6), S::Equity, K::CashNonFno).unwrap();
        assert_eq!(fno, ist(1, 6, 14, 20, 0));
        assert_eq!(non_fno, ist(1, 6, 14, 0, 0));
        assert_eq!(
            c.close_time(d(7, 1), S::Equity, K::CashFno).unwrap(),
            ist(7, 1, 14, 30, 0)
        );
    }

    #[test]
    fn derivative_kinds_and_wildcard() {
        let c = cal();
        assert_eq!(
            c.close_time(d(1, 6), S::EquityDerivatives, K::Derivative)
                .unwrap(),
            ist(1, 6, 14, 30, 0)
        );
        assert_eq!(
            c.close_time(d(1, 6), S::EquityDerivatives, K::DerivativeExpiringIndex)
                .unwrap(),
            ist(1, 6, 14, 10, 0)
        );
        for k in [K::CashNonFno, K::CashFno, K::Derivative] {
            assert_eq!(
                c.session_at(ist(1, 6, 9, 30, 0), S::Currency, k).unwrap(),
                Session::Normal
            );
        }
    }

    #[test]
    fn weekend_and_holiday_are_closed() {
        let c = cal();
        assert!(!c.is_trading_day(d(1, 3), S::Equity).unwrap()); // Saturday
        assert!(!c.is_trading_day(d(1, 4), S::Equity).unwrap()); // Sunday
        assert!(!c.is_trading_day(d(3, 12), S::Equity).unwrap());
        assert_eq!(
            c.session_at(ist(3, 12, 11, 0, 0), S::Equity, K::CashNonFno)
                .unwrap(),
            Session::Closed
        );
        assert_eq!(
            c.close_time(d(3, 12), S::Equity, K::CashNonFno),
            Err(CalendarError::NotTradingDay(d(3, 12)))
        );
    }

    #[test]
    fn holiday_is_per_segment_and_clearing_does_not_close() {
        let c = cal();
        assert!(!c.is_trading_day(d(6, 15), S::Equity).unwrap());
        assert!(c.is_trading_day(d(6, 15), S::Currency).unwrap());
        assert!(c.is_trading_day(d(4, 2), S::Equity).unwrap());
    }

    #[test]
    fn expiry_shifts_back_over_holidays_and_weekends() {
        let c = cal();
        let seg = S::EquityDerivatives;
        // Thursday holiday -> Wednesday
        assert_eq!(
            c.shift_to_prior_trading_day(d(3, 12), seg).unwrap(),
            d(3, 11)
        );
        // trading day unchanged
        assert_eq!(
            c.shift_to_prior_trading_day(d(3, 11), seg).unwrap(),
            d(3, 11)
        );
        // Monday holiday (equity) -> Friday across the weekend
        assert_eq!(
            c.shift_to_prior_trading_day(d(6, 15), S::Equity).unwrap(),
            d(6, 12)
        );
    }

    #[test]
    fn consecutive_holidays_shift_further() {
        let cfg: CalendarConfig = toml::from_str(
            "schema_version = 1\nverified_on = \"2026-01-05\"\nsource = \"t\"\ncovered_years = [2026]\n\
             [[holiday]]\ndate = \"2026-03-11\"\nsegments = [\"equity\"]\nkind = \"trading\"\nname = \"a\"\n\
             [[holiday]]\ndate = \"2026-03-12\"\nsegments = [\"equity\"]\nkind = \"trading\"\nname = \"b\"\n",
        )
        .unwrap();
        let c = MarketCalendar::from_config(&cfg);
        assert_eq!(
            c.shift_to_prior_trading_day(d(3, 12), S::Equity).unwrap(),
            d(3, 10)
        );
    }

    #[test]
    fn out_of_range_year_is_an_error() {
        let c = cal();
        let far = NaiveDate::from_ymd_opt(2031, 1, 1).unwrap();
        assert_eq!(
            c.is_trading_day(far, S::Equity),
            Err(CalendarError::OutOfRange(far))
        );
        assert!(c.next_trading_day(d(12, 31), S::Equity).is_err());
        assert!(c.prev_trading_day(d(1, 1), S::Equity).is_err());
        let t = Kolkata
            .with_ymd_and_hms(2031, 1, 1, 10, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert!(c.session_at(t, S::Equity, K::CashNonFno).is_err());
    }

    #[test]
    fn special_session_on_saturday() {
        let c = cal();
        assert!(c.is_trading_day(d(11, 7), S::Equity).unwrap());
        assert_eq!(
            c.session_at(ist(11, 7, 18, 30, 0), S::Equity, K::CashNonFno)
                .unwrap(),
            Session::Special("muhurat".into())
        );
        assert_eq!(
            c.session_at(ist(11, 7, 19, 0, 0), S::Equity, K::CashNonFno)
                .unwrap(),
            Session::Closed
        );
        assert_eq!(
            c.close_time(d(11, 7), S::Equity, K::CashNonFno).unwrap(),
            ist(11, 7, 19, 0, 0)
        );
        // not listed for currency
        assert!(!c.is_trading_day(d(11, 7), S::Currency).unwrap());
    }

    #[test]
    fn mock_session_is_visible_but_not_a_trading_day() {
        let c = cal();
        assert!(!c.is_trading_day(d(5, 2), S::Equity).unwrap());
        assert_eq!(
            c.session_at(ist(5, 2, 11, 30, 0), S::Equity, K::CashNonFno)
                .unwrap(),
            Session::Special("mock".into())
        );
    }

    #[test]
    fn no_session_rows_is_an_error() {
        let c = cal();
        assert!(matches!(
            c.close_time(d(1, 6), S::Commodity, K::Derivative),
            Err(CalendarError::NoSession { .. })
        ));
    }

    #[test]
    fn works_with_clock_instant() {
        let clock = Fixed(Kolkata.with_ymd_and_hms(2026, 1, 6, 10, 30, 0).unwrap());
        let now = clock.now_ist().with_timezone(&Utc);
        assert_eq!(
            cal().session_at(now, S::Equity, K::CashNonFno).unwrap(),
            Session::Normal
        );
    }

    proptest! {
        #[test]
        fn next_prev_roundtrip(day in 0u64..360) {
            let c = cal();
            let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(day);
            if c.is_trading_day(start, S::Equity).unwrap()
                && let Ok(n) = c.next_trading_day(start, S::Equity)
            {
                prop_assert!(c.is_trading_day(n, S::Equity).unwrap());
                prop_assert!(n > start);
                prop_assert!(c.prev_trading_day(n, S::Equity).unwrap() >= start);
            }
        }
    }
}
