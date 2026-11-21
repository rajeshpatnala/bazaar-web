//! MIS auto square-off. At `[execution] paper_mis_square_off_ist` (IST) `on_clock` cancels every
//! working MIS order and queues one market-like order per open MIS position in the opposite
//! direction; it fills on the next tick with slippage like any market order (so it also waits
//! for a fresh, in-session, non-circuit tick). Once per IST date. This is a simulated broker
//! action, not a user order; the executor audits it.
//!
//! CR-96-5: [`check_square_off_time`] bounds the configured time to the normal session of the
//! calendar (a time such as 00:00 is refused).

use bazaar_core::calendar::{CalendarError, MarketCalendar, Session};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_execution::types::{OrderKind, Product, Side};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Asia::Kolkata;
use chrono_tz::Tz;
use thiserror::Error;

use crate::fill::{WorkingBook, WorkingOrder};
use bazaar_execution::types::IntentId;

/// Why a square-off time is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SquareOffError {
    #[error("square-off time is not inside the normal session")]
    OutsideSession,
    #[error("calendar: {0}")]
    Calendar(CalendarError),
}

/// Check that `at` on `date` falls in the normal session for the segment and kind.
///
/// # Errors
/// `OutsideSession` when it does not; `Calendar` when the date is not covered (fail closed).
pub fn check_square_off_time(
    cal: &MarketCalendar,
    date: NaiveDate,
    at: NaiveTime,
    seg: Segment,
    kind: InstrumentKind,
) -> Result<(), SquareOffError> {
    let local = Kolkata
        .from_local_datetime(&date.and_time(at))
        .single()
        .ok_or(SquareOffError::OutsideSession)?;
    match cal.session_at(local.with_timezone(&Utc), seg, kind) {
        Ok(Session::Normal) => Ok(()),
        Ok(_) => Err(SquareOffError::OutsideSession),
        Err(e) => Err(SquareOffError::Calendar(e)),
    }
}

/// A square-off time proven (against the calendar) to lie inside the normal session of every
/// segment and kind in use. The only way to build one is [`Self::new`], so a mis-set
/// `paper_mis_square_off_ist` (00:00, 23:59, a holiday-only time, an uncovered year) is refused at
/// construction instead of silently disabling the square-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedSquareOffTime(NaiveTime);

impl ValidatedSquareOffTime {
    /// # Errors
    /// `OutsideSession` when `at` is not in the normal session of any listed segment (or the list
    /// is empty); `Calendar` when `date` is not covered (fail closed).
    pub fn new(
        cal: &MarketCalendar,
        date: NaiveDate,
        at: NaiveTime,
        segments: &[(Segment, InstrumentKind)],
    ) -> Result<Self, SquareOffError> {
        if segments.is_empty() {
            return Err(SquareOffError::OutsideSession);
        }
        for (seg, kind) in segments {
            check_square_off_time(cal, date, at, *seg, *kind)?;
        }
        Ok(Self(at))
    }

    #[must_use]
    pub const fn time(self) -> NaiveTime {
        self.0
    }
}

/// What one `on_clock` call did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SquareOffRun {
    /// Working MIS orders cancelled.
    pub cancelled: Vec<IntentId>,
    /// Square-off orders queued: (id, symbol, qty).
    pub queued: Vec<(IntentId, String, u32)>,
    /// Symbols that could not be queued (no spec, or quantity too large). Not final: the run is
    /// repeated on the next clock call until none fail, and the caller must alarm.
    pub failed: Vec<String>,
}

/// Square-off trigger; holds only the configured time and the dates it last ran / was called.
#[derive(Debug, Clone)]
pub struct SquareOff {
    at: NaiveTime,
    done_on: Option<NaiveDate>,
    last_seen: Option<NaiveDate>,
}

impl SquareOff {
    #[must_use]
    pub const fn new(at: ValidatedSquareOffTime) -> Self {
        Self {
            at: at.0,
            done_on: None,
            last_seen: None,
        }
    }

    /// Run once per IST date when `now` has reached the configured time (repeated while a symbol
    /// fails). On the first call of a new IST date any MIS position still open was carried over
    /// and is overdue: it is queued at once (it fills on the first in-session tick).
    pub fn on_clock(&mut self, now: DateTime<Tz>, book: &mut WorkingBook) -> SquareOffRun {
        let today = now.date_naive();
        let new_day = self.last_seen.is_some_and(|d| d < today);
        self.last_seen = Some(today);
        let due = now.time() >= self.at && self.done_on != Some(today);
        if !due && !new_day {
            return SquareOffRun::default();
        }
        let is_working_mis =
            |o: &crate::fill::WorkingOrder| o.product == Product::Mis && !o.square_off;
        let mut run = SquareOffRun {
            cancelled: book
                .orders
                .iter()
                .filter(|o| is_working_mis(o))
                .map(|o| o.id)
                .collect(),
            ..SquareOffRun::default()
        };
        book.orders.retain(|o| !is_working_mis(o));
        for (symbol, net) in book.open_positions(Product::Mis) {
            if book
                .orders
                .iter()
                .any(|o| o.square_off && o.symbol == symbol)
            {
                continue; // already queued
            }
            let Some(spec) = book.specs.get(&symbol).cloned() else {
                run.failed.push(symbol);
                continue;
            };
            let Ok(qty) = u32::try_from(net.unsigned_abs()) else {
                run.failed.push(symbol);
                continue;
            };
            let side = if net > 0 { Side::Sell } else { Side::Buy };
            let id = book.synth_id();
            book.orders.push(WorkingOrder {
                id,
                symbol: symbol.clone(),
                side,
                qty,
                filled: 0,
                kind: OrderKind::Market,
                product: Product::Mis,
                spec,
                accepted_at: now,
                stale_flagged: false,
                square_off: true,
            });
            run.queued.push((id, symbol, qty));
        }
        if due && run.failed.is_empty() {
            self.done_on = Some(today);
        }
        run
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fill::tests::{at, d, order, spec};
    use crate::fill::{FillReason, FillSimulator, SimFill};
    use chrono::TimeZone;

    fn mis_long(book: &mut WorkingBook, qty: u32) {
        book.specs.insert("ABC".into(), spec());
        book.record(&SimFill {
            id: IntentId(1),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty,
            price: d("100"),
            product: Product::Mis,
            at: at(10, 0, 1),
            complete: true,
            reason: FillReason::Market,
        });
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn vt(h: u32, m: u32) -> ValidatedSquareOffTime {
        let day = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        ValidatedSquareOffTime::new(
            &crate::fill::tests::cal(),
            day,
            t(h, m),
            &[(Segment::Equity, InstrumentKind::CashNonFno)],
        )
        .unwrap()
    }

    #[test]
    fn queues_opposite_order_once_per_day_at_the_configured_time() {
        let mut b = WorkingBook::default();
        mis_long(&mut b, 7);
        let mut so = SquareOff::new(vt(13, 30));
        assert_eq!(so.on_clock(at(13, 29, 59), &mut b), SquareOffRun::default());
        let run = so.on_clock(at(13, 30, 0), &mut b);
        assert_eq!(run.queued.len(), 1);
        assert_eq!((run.queued[0].1.as_str(), run.queued[0].2), ("ABC", 7));
        assert_eq!(b.orders[0].side, Side::Sell);
        assert!(b.orders[0].square_off);
        assert_eq!(so.on_clock(at(13, 31, 0), &mut b), SquareOffRun::default());
        assert_eq!(b.orders.len(), 1, "not queued twice");
    }

    #[test]
    fn working_mis_orders_are_cancelled_cnc_untouched() {
        let mut b = WorkingBook::default();
        let mut m = order(5, Side::Buy, 3, OrderKind::Market);
        m.product = Product::Mis;
        b.orders.push(m);
        b.orders.push(order(6, Side::Buy, 3, OrderKind::Market));
        let run = SquareOff::new(vt(13, 30)).on_clock(at(14, 0, 0), &mut b);
        assert_eq!(run.cancelled, vec![IntentId(5)]);
        assert_eq!(b.orders.len(), 1);
        assert_eq!(b.orders[0].id, IntentId(6));
    }

    #[test]
    fn squareoff_fills_on_next_tick_with_slippage_and_flat_position() {
        let mut b = WorkingBook::default();
        mis_long(&mut b, 4);
        SquareOff::new(vt(13, 30)).on_clock(at(13, 30, 0), &mut b);
        let mut sim = FillSimulator::new(crate::fill::tests::cfg());
        let mut tk = crate::fill::tests::tick("100", 1000, 0);
        tk.at = at(13, 30, 1);
        let fills = sim.on_tick(&tk, tk.at, &crate::fill::tests::cal(), &mut b);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].reason, FillReason::SquareOff);
        assert_eq!(fills[0].price, d("99.90"));
        b.record(&fills[0]);
        assert_eq!(b.position("ABC", Product::Mis), 0);
    }

    #[test]
    fn no_open_mis_position_queues_nothing() {
        let mut b = WorkingBook::default();
        let run = SquareOff::new(vt(13, 30)).on_clock(at(14, 0, 0), &mut b);
        assert_eq!(run, SquareOffRun::default());
    }

    #[test]
    fn time_is_bounded_to_the_session_calendar() {
        let cfg: bazaar_core::config::CalendarConfig =
            bazaar_core::config::load(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/config"
            )))
            .unwrap();
        let cal = MarketCalendar::from_config(&cfg);
        let day = Kolkata
            .with_ymd_and_hms(2026, 1, 5, 0, 0, 0)
            .unwrap()
            .date_naive();
        let (s, k) = (Segment::Equity, InstrumentKind::CashNonFno);
        assert_eq!(check_square_off_time(&cal, day, t(13, 30), s, k), Ok(()));
        assert_eq!(
            check_square_off_time(&cal, day, t(0, 0), s, k),
            Err(SquareOffError::OutsideSession)
        );
        assert_eq!(
            check_square_off_time(&cal, day, t(14, 0), s, k),
            Err(SquareOffError::OutsideSession)
        );
        let far = NaiveDate::from_ymd_opt(2030, 1, 7).unwrap();
        assert!(matches!(
            check_square_off_time(&cal, far, t(13, 30), s, k),
            Err(SquareOffError::Calendar(_))
        ));
    }

    #[test]
    fn validated_time_refuses_out_of_session_and_empty_lists() {
        let cal = crate::fill::tests::cal();
        let day = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let eq = [(Segment::Equity, InstrumentKind::CashNonFno)];
        for bad in [t(0, 0), t(23, 59), t(15, 45)] {
            assert_eq!(
                ValidatedSquareOffTime::new(&cal, day, bad, &eq),
                Err(SquareOffError::OutsideSession)
            );
        }
        assert_eq!(
            ValidatedSquareOffTime::new(&cal, day, t(13, 30), &[]),
            Err(SquareOffError::OutsideSession)
        );
        assert_eq!(
            ValidatedSquareOffTime::new(&cal, day, t(13, 30), &eq)
                .unwrap()
                .time(),
            t(13, 30)
        );
    }

    #[test]
    fn missing_spec_is_reported_and_retried_not_marked_done() {
        let mut b = WorkingBook::default();
        mis_long(&mut b, 3);
        b.specs.clear();
        let mut so = SquareOff::new(vt(13, 30));
        let run = so.on_clock(at(13, 30, 0), &mut b);
        assert_eq!(
            (run.failed.clone(), run.queued.len()),
            (vec!["ABC".to_owned()], 0)
        );
        b.specs.insert("ABC".into(), spec());
        let run = so.on_clock(at(13, 31, 0), &mut b);
        assert_eq!((run.failed.len(), run.queued.len()), (0, 1));
        assert_eq!(so.on_clock(at(13, 32, 0), &mut b), SquareOffRun::default());
    }

    #[test]
    fn carried_mis_position_is_queued_at_the_first_clock_call_of_the_next_date() {
        let mut b = WorkingBook::default();
        mis_long(&mut b, 3);
        let mut so = SquareOff::new(vt(13, 30));
        assert_eq!(so.on_clock(at(10, 0, 0), &mut b), SquareOffRun::default());
        let next = at(8, 0, 0) + chrono::Duration::days(1);
        let run = so.on_clock(next, &mut b);
        assert_eq!(run.queued.len(), 1, "carried position is overdue");
        let run = so.on_clock(next + chrono::Duration::minutes(5), &mut b);
        assert_eq!(run, SquareOffRun::default(), "not queued twice");
    }
}
