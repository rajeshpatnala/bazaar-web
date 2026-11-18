//! `calendar.toml` data types and validation. `MarketCalendar` (slice core-calendar) is built
//! from [`CalendarConfig`]; no session time, holiday or weekday literal lives in Rust.

use std::collections::HashSet;

use chrono::{Datelike, NaiveDate, NaiveTime};
use serde::Deserialize;
use serde::de::{Deserializer, Error as _, IntoDeserializer};

use super::{ConfigError, ConfigFile, impl_header};
use crate::segment::{InstrumentKind, Segment};

/// Which instrument kinds a session window covers. `all` is a loader wildcard, not an
/// [`InstrumentKind`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliesTo {
    All,
    Kind(InstrumentKind),
}

impl AppliesTo {
    /// Does this window cover `kind`?
    #[must_use]
    pub fn covers(self, kind: InstrumentKind) -> bool {
        match self {
            Self::All => true,
            Self::Kind(k) => k == kind,
        }
    }

    fn overlaps(self, other: Self) -> bool {
        match (self, other) {
            (Self::All, _) | (_, Self::All) => true,
            (Self::Kind(a), Self::Kind(b)) => a == b,
        }
    }
}

impl<'de> Deserialize<'de> for AppliesTo {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "all" {
            return Ok(Self::All);
        }
        let de: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
            s.as_str().into_deserializer();
        InstrumentKind::deserialize(de)
            .map(Self::Kind)
            .map_err(D::Error::custom)
    }
}

/// Session phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    PreOpen,
    Normal,
    ClosingAuction,
    PostClose,
}

/// Holiday kind; clearing holidays do not close trading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HolidayKind {
    Trading,
    Clearing,
}

/// Special-session kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialKind {
    Muhurat,
    Budget,
    Other,
    Mock,
}

/// `"HH:MM"` (IST) as a `NaiveTime`.
mod hhmm {
    use chrono::NaiveTime;
    use serde::Deserialize;
    use serde::de::{Deserializer, Error};

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
        let s = String::deserialize(d)?;
        match NaiveTime::parse_from_str(&s, "%H:%M") {
            Ok(t) if s.len() == 5 => return Ok(t),
            _ => {}
        }
        Err(D::Error::custom(format!("expected \"HH:MM\", got {s:?}")))
    }
}

/// One `[[session]]` window, effective-dated.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRow {
    pub segment: Segment,
    pub applies_to: AppliesTo,
    pub phase: Phase,
    #[serde(deserialize_with = "hhmm::deserialize")]
    pub start: NaiveTime,
    #[serde(deserialize_with = "hhmm::deserialize")]
    pub end: NaiveTime,
    pub effective_from: NaiveDate,
    /// Inclusive; omitted means open-ended.
    #[serde(default)]
    pub effective_to: Option<NaiveDate>,
    pub source: String,
}

/// One `[[holiday]]` row.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HolidayRow {
    pub date: NaiveDate,
    pub segments: Vec<Segment>,
    pub kind: HolidayKind,
    pub name: String,
}

/// One `[[special_session]]` row.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecialSessionRow {
    pub date: NaiveDate,
    pub segments: Vec<Segment>,
    pub kind: SpecialKind,
    #[serde(deserialize_with = "hhmm::deserialize")]
    pub start: NaiveTime,
    #[serde(deserialize_with = "hhmm::deserialize")]
    pub end: NaiveTime,
    /// `false` for mock/DR sessions: never feed analytics.
    pub live_data: bool,
    pub source: String,
}

/// `calendar.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarConfig {
    pub schema_version: u32,
    pub verified_on: NaiveDate,
    pub source: String,
    /// A date outside these years is `OutOfRange` in `MarketCalendar`.
    pub covered_years: Vec<i32>,
    #[serde(default)]
    pub session: Vec<SessionRow>,
    #[serde(default)]
    pub holiday: Vec<HolidayRow>,
    #[serde(default)]
    pub special_session: Vec<SpecialSessionRow>,
}

fn bad(key: String, reason: &str) -> ConfigError {
    ConfigError::invalid(CalendarConfig::FILE, key, reason)
}

fn dates_overlap(a: &SessionRow, b: &SessionRow) -> bool {
    let a_ends_before_b = a.effective_to.is_some_and(|t| t < b.effective_from);
    let b_ends_before_a = b.effective_to.is_some_and(|t| t < a.effective_from);
    !(a_ends_before_b || b_ends_before_a)
}

impl CalendarConfig {
    fn validate_sessions(&self) -> Result<(), ConfigError> {
        for (i, s) in self.session.iter().enumerate() {
            if s.start >= s.end {
                return Err(bad(format!("session[{i}].start"), "must be before end"));
            }
            if s.effective_to.is_some_and(|t| t < s.effective_from) {
                return Err(bad(
                    format!("session[{i}].effective_to"),
                    "must not be before effective_from",
                ));
            }
            if s.source.trim().is_empty() {
                return Err(bad(format!("session[{i}].source"), "must not be empty"));
            }
            for (j, o) in self.session[..i].iter().enumerate() {
                if s.segment == o.segment
                    && s.applies_to.overlaps(o.applies_to)
                    && dates_overlap(s, o)
                    && s.start < o.end
                    && o.start < s.end
                {
                    return Err(bad(
                        format!("session[{i}]"),
                        &format!("overlaps session[{j}]"),
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_days(&self) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        for (i, h) in self.holiday.iter().enumerate() {
            if h.segments.is_empty() {
                return Err(bad(format!("holiday[{i}].segments"), "must not be empty"));
            }
            if !self.covered_years.contains(&h.date.year()) {
                return Err(bad(
                    format!("holiday[{i}].date"),
                    "year is not in covered_years",
                ));
            }
            for seg in &h.segments {
                if !seen.insert((h.date, *seg)) {
                    return Err(bad(
                        format!("holiday[{i}].date"),
                        "duplicate date for a segment",
                    ));
                }
            }
        }
        for (i, s) in self.special_session.iter().enumerate() {
            if s.segments.is_empty() {
                return Err(bad(
                    format!("special_session[{i}].segments"),
                    "must not be empty",
                ));
            }
            if s.start >= s.end {
                return Err(bad(
                    format!("special_session[{i}].start"),
                    "must be before end",
                ));
            }
            if s.kind == SpecialKind::Mock && s.live_data {
                return Err(bad(
                    format!("special_session[{i}].live_data"),
                    "a mock session must not carry live data",
                ));
            }
            if s.source.trim().is_empty() {
                return Err(bad(
                    format!("special_session[{i}].source"),
                    "must not be empty",
                ));
            }
            if !self.covered_years.contains(&s.date.year()) {
                return Err(bad(
                    format!("special_session[{i}].date"),
                    "year is not in covered_years",
                ));
            }
        }
        Ok(())
    }
}

impl ConfigFile for CalendarConfig {
    const FILE: &'static str = "calendar.toml";
    impl_header!();

    fn validate(&self) -> Result<(), ConfigError> {
        self.validate_sessions()?;
        self.validate_days()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load, parse_str};
    use std::path::Path;

    const HEAD: &str = "schema_version = 1\nverified_on = \"2026-01-01\"\nsource = \"x\"\ncovered_years = [2026]\n";

    fn session(kind: &str, phase: &str, start: &str, end: &str, extra: &str) -> String {
        format!(
            "[[session]]\nsegment = \"equity\"\napplies_to = \"{kind}\"\nphase = \"{phase}\"\n\
             start = \"{start}\"\nend = \"{end}\"\neffective_from = \"2020-01-01\"\n{extra}source = \"s\"\n"
        )
    }

    fn err_of(body: &str) -> String {
        parse_str::<CalendarConfig>(&format!("{HEAD}{body}"))
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn fixture_calendar_parses_with_every_shape() {
        let cfg: CalendarConfig = load(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/config"
        )))
        .unwrap();
        assert_eq!(cfg.session.len(), 8);
        assert_eq!(cfg.holiday.len(), 3);
        assert_eq!(cfg.special_session.len(), 2);
        assert!(cfg.session.iter().any(|s| s.applies_to == AppliesTo::All));
        assert!(cfg.session.iter().any(|s| s.effective_to.is_some()));
        assert!(cfg.holiday.iter().any(|h| h.kind == HolidayKind::Clearing));
        assert!(
            cfg.special_session
                .iter()
                .any(|s| s.kind == SpecialKind::Mock && !s.live_data)
        );
    }

    #[test]
    fn applies_to_wildcard_and_kinds() {
        assert!(AppliesTo::All.covers(InstrumentKind::Derivative));
        assert!(AppliesTo::Kind(InstrumentKind::CashFno).covers(InstrumentKind::CashFno));
        assert!(!AppliesTo::Kind(InstrumentKind::CashFno).covers(InstrumentKind::CashNonFno));
        let cfg = parse_str::<CalendarConfig>(&format!(
            "{HEAD}{}",
            session("all", "normal", "10:00", "11:00", "")
        ))
        .unwrap();
        assert_eq!(cfg.session[0].applies_to, AppliesTo::All);
        assert!(
            err_of(&session("everything", "normal", "10:00", "11:00", ""))
                .contains("calendar.toml")
        );
    }

    #[test]
    fn rejects_bad_times() {
        assert!(
            err_of(&session("all", "normal", "11:00", "10:00", "")).contains("session[0].start")
        );
        assert!(err_of(&session("all", "normal", "10:00", "10:00", "")).contains("before end"));
        assert!(err_of(&session("all", "normal", "25:00", "26:00", "")).contains("HH:MM"));
        assert!(err_of(&session("all", "normal", "9:00", "10:00", "")).contains("HH:MM"));
        assert!(err_of(&session("all", "normal", "10:00:00", "11:00", "")).contains("HH:MM"));
    }

    #[test]
    fn rejects_overlaps_but_allows_adjacent_and_disjoint_dates() {
        let two = |a: &str, b: &str| format!("{a}{b}");
        let overlap = two(
            &session("cash_fno", "normal", "10:00", "11:00", ""),
            &session("all", "closing_auction", "10:30", "11:30", ""),
        );
        assert!(err_of(&overlap).contains("session[1]: overlaps session[0]"));
        let adjacent = two(
            &session("cash_fno", "normal", "10:00", "11:00", ""),
            &session("cash_fno", "closing_auction", "11:00", "11:20", ""),
        );
        assert!(parse_str::<CalendarConfig>(&format!("{HEAD}{adjacent}")).is_ok());
        let other_kind = two(
            &session("cash_fno", "normal", "10:00", "11:00", ""),
            &session("cash_non_fno", "normal", "10:00", "11:00", ""),
        );
        assert!(parse_str::<CalendarConfig>(&format!("{HEAD}{other_kind}")).is_ok());
        let regime = two(
            &session(
                "cash_fno",
                "normal",
                "10:00",
                "11:00",
                "effective_to = \"2021-12-31\"\n",
            ),
            &session("cash_fno", "normal", "10:00", "11:30", "")
                .replace("2020-01-01", "2022-01-01"),
        );
        assert!(parse_str::<CalendarConfig>(&format!("{HEAD}{regime}")).is_ok());
    }

    #[test]
    fn rejects_bad_effective_range_and_blank_source() {
        let bad_range = session(
            "all",
            "normal",
            "10:00",
            "11:00",
            "effective_to = \"2019-01-01\"\n",
        );
        assert!(err_of(&bad_range).contains("effective_to"));
        let blank = session("all", "normal", "10:00", "11:00", "")
            .replace("source = \"s\"", "source = \" \"");
        assert!(err_of(&blank).contains("session[0].source"));
    }

    #[test]
    fn holiday_rules() {
        let h = |date: &str, segs: &str| {
            format!(
                "[[holiday]]\ndate = \"{date}\"\nsegments = [{segs}]\nkind = \"trading\"\nname = \"n\"\n"
            )
        };
        assert!(
            parse_str::<CalendarConfig>(&format!(
                "{HEAD}{}",
                h("2026-03-12", "\"equity\", \"currency\"")
            ))
            .is_ok()
        );
        let dup = format!(
            "{}{}",
            h("2026-03-12", "\"equity\""),
            h("2026-03-12", "\"equity\"")
        );
        assert!(err_of(&dup).contains("holiday[1].date"));
        // same date for a different segment is fine
        let two_segs = format!(
            "{}{}",
            h("2026-03-12", "\"equity\""),
            h("2026-03-12", "\"currency\"")
        );
        assert!(parse_str::<CalendarConfig>(&format!("{HEAD}{two_segs}")).is_ok());
        assert!(err_of(&h("2027-03-12", "\"equity\"")).contains("covered_years"));
        assert!(err_of(&h("2026-03-12", "")).contains("segments"));
        assert!(err_of(&h("2026-03-12", "\"slb\"")).contains("calendar.toml"));
    }

    #[test]
    fn special_session_rules() {
        let s = |kind: &str, live: bool, start: &str, end: &str| {
            format!(
                "[[special_session]]\ndate = \"2026-11-07\"\nsegments = [\"equity\"]\nkind = \"{kind}\"\n\
                 start = \"{start}\"\nend = \"{end}\"\nlive_data = {live}\nsource = \"s\"\n"
            )
        };
        assert!(
            parse_str::<CalendarConfig>(&format!("{HEAD}{}", s("muhurat", true, "18:00", "19:00")))
                .is_ok()
        );
        assert!(
            parse_str::<CalendarConfig>(&format!("{HEAD}{}", s("mock", false, "18:00", "19:00")))
                .is_ok()
        );
        assert!(err_of(&s("mock", true, "18:00", "19:00")).contains("live_data"));
        assert!(err_of(&s("muhurat", true, "19:00", "18:00")).contains("special_session[0].start"));
        assert!(
            err_of(&s("muhurat", true, "18:00", "19:00").replace("2026-11-07", "2028-11-07"))
                .contains("covered_years")
        );
    }

    #[test]
    fn covered_years_is_required_and_unknown_keys_fail() {
        let no_years = "schema_version = 1\nverified_on = \"2026-01-01\"\nsource = \"x\"\n";
        assert!(parse_str::<CalendarConfig>(no_years).is_err());
        assert!(err_of("weekend = \"sat\"\n").contains("calendar.toml"));
    }
}
