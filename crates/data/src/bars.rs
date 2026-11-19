//! Bar builder: 1-minute candles into 3/5/10/15/30/60-minute and daily bars, session aware.
//!
//! Buckets align to the normal-session open read from the [`MarketCalendar`] (never a literal).
//! A bar is emitted only when its last minute has arrived, so nothing looks ahead. Malformed,
//! duplicate, out-of-order or out-of-session minutes are counted and skipped, never repaired; a
//! bucket with a missing minute is dropped and counted, never filled.
//!
//! Only the Normal session is aggregated: special sessions (muhurat, mock) are not, and their
//! minutes count as `out_of_session`. A bucket still forming when the input ends is neither
//! emitted nor counted (a trailing partial bucket is indistinguishable from one still arriving).
//! A bucket that mixes adjusted and unadjusted minutes is dropped and counted.

use bazaar_core::calendar::{CalendarError, MarketCalendar, Session};
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::segment::{InstrumentKind, Segment};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Asia::Kolkata;
use rust_decimal::Decimal;
use thiserror::Error;

/// Largest slice [`aggregate`] accepts (about 7 years of 1-minute bars for one instrument).
pub const MAX_INPUT_BARS: usize = 1_000_000;

/// Bar builder failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BarError {
    /// The target must be a coarser interval than one minute.
    #[error("target interval must be coarser than 1 minute")]
    BadTarget,
    /// Too many input bars for one call.
    #[error("input of {0} bars exceeds the limit of {MAX_INPUT_BARS}")]
    TooManyBars(usize),
}

/// Counters for everything skipped or dropped. Nothing is repaired.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BarStats {
    /// Minutes accepted into a bucket.
    pub accepted: u64,
    /// Bars emitted.
    pub emitted: u64,
    /// Input that was not a 1-minute bar, not on a whole minute, or failed re-validation.
    pub malformed: u64,
    /// Close time not after the previous accepted one (duplicate or out of order).
    pub out_of_order: u64,
    /// Outside the normal session or on a non-trading day (special sessions included).
    pub out_of_session: u64,
    /// Minutes on a date the calendar could not answer for (for example a year missing from
    /// `calendar.toml`); a stale calendar shows up here, not as ordinary overnight data.
    pub calendar_errors: u64,
    /// Buckets dropped because adjusted and unadjusted minutes were mixed.
    pub mixed_adjusted_buckets: u64,
    /// Buckets dropped because a minute was missing.
    pub incomplete_buckets: u64,
    /// Minutes missing from dropped buckets (counted when the next bucket starts).
    pub gap_minutes: u64,
    /// Buckets dropped because summed volume overflowed `u64`.
    pub overflow_buckets: u64,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    open: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Debug)]
struct Bucket {
    id: (NaiveDate, i64),
    end: DateTime<Utc>,
    expected: u64,
    count: u64,
    ohlc: [Decimal; 4],
    volume: Option<u64>,
    adjusted: bool,
    mixed: bool,
}

/// Streaming 1-minute to higher-interval builder for one instrument.
#[derive(Debug)]
pub struct BarBuilder<'a> {
    cal: &'a MarketCalendar,
    seg: Segment,
    kind: InstrumentKind,
    target: Interval,
    stats: BarStats,
    cur: Option<Bucket>,
    last_seen: Option<DateTime<Utc>>,
    window: Option<(NaiveDate, Result<Option<Window>, CalendarError>)>,
}

impl<'a> BarBuilder<'a> {
    /// New builder.
    ///
    /// # Errors
    /// `BadTarget` when `target` is `Min1`.
    pub fn new(
        cal: &'a MarketCalendar,
        seg: Segment,
        kind: InstrumentKind,
        target: Interval,
    ) -> Result<Self, BarError> {
        if target == Interval::Min1 {
            return Err(BarError::BadTarget);
        }
        Ok(Self {
            cal,
            seg,
            kind,
            target,
            stats: BarStats::default(),
            cur: None,
            last_seen: None,
            window: None,
        })
    }

    /// Counters so far.
    #[must_use]
    pub const fn stats(&self) -> BarStats {
        self.stats
    }

    /// Normal-session window of a date: first Normal minute to the end of the last one.
    fn probe(&self, date: NaiveDate) -> Result<Option<Window>, CalendarError> {
        if !self.cal.is_trading_day(date, self.seg)? {
            return Ok(None);
        }
        let (mut open, mut end) = (None, None);
        for m in 0..1440u32 {
            let Some(time) = NaiveTime::from_hms_opt(m / 60, m % 60, 0) else {
                continue;
            };
            let Some(t) = Kolkata
                .from_local_datetime(&date.and_time(time))
                .single()
                .map(|t| t.with_timezone(&Utc))
            else {
                continue;
            };
            if self.cal.session_at(t, self.seg, self.kind)? == Session::Normal {
                open.get_or_insert(t);
                end = t.checked_add_signed(Duration::minutes(1));
            }
        }
        Ok(open.zip(end).map(|(open, end)| Window { open, end }))
    }

    /// Window of a date, probed once per date: input is strictly increasing (see `push`), so one
    /// cached date is enough and the 1440-minute probe runs at most once per distinct date.
    fn window_for(&mut self, date: NaiveDate) -> Result<Option<Window>, CalendarError> {
        if let Some((d, w)) = &self.window
            && *d == date
        {
            return w.clone();
        }
        let w = self.probe(date);
        self.window = Some((date, w.clone()));
        w
    }

    fn drop_bucket(&mut self) {
        if let Some(b) = self.cur.take() {
            self.stats.incomplete_buckets += 1;
            self.stats.gap_minutes += b.expected.saturating_sub(b.count);
        }
    }

    /// Feed one 1-minute candle; returns a bar only when its bucket just completed.
    pub fn push(&mut self, c: &Candle) -> Option<Candle> {
        let close = c.close_time().with_timezone(&Utc);
        if c.interval() != Interval::Min1 || close.second() != 0 || close.nanosecond() != 0 {
            self.stats.malformed += 1;
            return None;
        }
        if self.last_seen.is_some_and(|l| close <= l) {
            self.stats.out_of_order += 1;
            return None;
        }
        // Every well-formed minute moves the order mark, accepted or not, so rejected input cannot
        // make the window probe repeat.
        self.last_seen = Some(close);
        let Some(start) = close.checked_sub_signed(Duration::minutes(1)) else {
            self.stats.malformed += 1;
            return None;
        };
        let date = start.with_timezone(&Kolkata).date_naive();
        let Ok(w) = self.window_for(date) else {
            self.stats.calendar_errors += 1;
            return None;
        };
        let Some(w) = w.filter(|w| w.open <= start && close <= w.end) else {
            self.stats.out_of_session += 1;
            return None;
        };
        let (id, b_start, b_end) = match self.target.minutes() {
            Some(n) => {
                let n = i64::from(n);
                let idx = (start - w.open).num_minutes() / n;
                let b_start = w.open + Duration::minutes(idx * n);
                (
                    (date, idx),
                    b_start,
                    (b_start + Duration::minutes(n)).min(w.end),
                )
            }
            None => ((date, 0), w.open, w.end),
        };
        if self.cur.as_ref().is_some_and(|b| b.id != id) {
            self.drop_bucket();
        }
        self.stats.accepted += 1;
        let b = self.cur.get_or_insert_with(|| Bucket {
            id,
            end: b_end,
            expected: u64::try_from((b_end - b_start).num_minutes()).unwrap_or(u64::MAX),
            count: 0,
            ohlc: [c.open(), c.high(), c.low(), c.close()],
            volume: Some(0),
            adjusted: c.adjusted(),
            mixed: false,
        });
        b.mixed |= b.adjusted != c.adjusted();
        if b.count > 0 {
            b.ohlc[1] = b.ohlc[1].max(c.high());
            b.ohlc[2] = b.ohlc[2].min(c.low());
            b.ohlc[3] = c.close();
        }
        b.volume = b.volume.and_then(|v| v.checked_add(c.volume()));
        b.count += 1;
        if b.count < b.expected || close != b.end {
            return None;
        }
        let b = self.cur.take()?;
        if b.mixed {
            self.stats.mixed_adjusted_buckets += 1;
            return None;
        }
        let Some(volume) = b.volume else {
            self.stats.overflow_buckets += 1;
            return None;
        };
        let end = b.end.with_timezone(&Kolkata);
        if let Ok(bar) = Candle::new(end, self.target, b.ohlc, volume, b.adjusted) {
            self.stats.emitted += 1;
            Some(bar)
        } else {
            self.stats.malformed += 1;
            None
        }
    }
}

/// Aggregate a time-ordered slice of 1-minute candles in one call.
///
/// # Errors
/// `BadTarget` for `Min1`; `TooManyBars` above [`MAX_INPUT_BARS`].
pub fn aggregate(
    cal: &MarketCalendar,
    seg: Segment,
    kind: InstrumentKind,
    target: Interval,
    candles: &[Candle],
) -> Result<(Vec<Candle>, BarStats), BarError> {
    if candles.len() > MAX_INPUT_BARS {
        return Err(BarError::TooManyBars(candles.len()));
    }
    let mut b = BarBuilder::new(cal, seg, kind, target)?;
    let out = candles.iter().filter_map(|c| b.push(c)).collect();
    Ok((out, b.stats()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_default_is_zero() {
        assert_eq!(BarStats::default().emitted, 0);
        assert_eq!(BarStats::default().accepted, 0);
    }

    #[test]
    fn bar_error_messages() {
        assert!(BarError::BadTarget.to_string().contains("coarser"));
        assert!(BarError::TooManyBars(3).to_string().contains('3'));
    }
}
