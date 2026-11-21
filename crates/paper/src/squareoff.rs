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

/// What one `on_clock` call did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SquareOffRun {
    /// Working MIS orders cancelled.
    pub cancelled: Vec<IntentId>,
    /// Square-off orders queued: (id, symbol, qty).
    pub queued: Vec<(IntentId, String, u32)>,
}

/// Square-off trigger; holds only the configured time and the last date it ran.
#[derive(Debug, Clone)]
pub struct SquareOff {
    at: NaiveTime,
    done_on: Option<NaiveDate>,
}

impl SquareOff {
    #[must_use]
    pub const fn new(at: NaiveTime) -> Self {
        Self { at, done_on: None }
    }

    /// Run once per IST date when `now` has reached the configured time.
    pub fn on_clock(&mut self, now: DateTime<Tz>, book: &mut WorkingBook) -> SquareOffRun {
        let today = now.date_naive();
        if now.time() < self.at || self.done_on == Some(today) {
            return SquareOffRun::default();
        }
        self.done_on = Some(today);
        let mut run = SquareOffRun::default();
        run.cancelled = book
            .orders
            .iter()
            .filter(|o| o.product == Product::Mis && !o.square_off)
            .map(|o| o.id)
            .collect();
        book.orders
            .retain(|o| !(o.product == Product::Mis && !o.square_off));
        for (symbol, net) in book.open_positions(Product::Mis) {
            let Some(spec) = book.specs.get(&symbol).cloned() else {
                continue;
            };
            let Ok(qty) = u32::try_from(net.unsigned_abs()) else {
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

    #[test]
    fn queues_opposite_order_once_per_day_at_the_configured_time() {
        let mut b = WorkingBook::default();
        mis_long(&mut b, 7);
        let mut so = SquareOff::new(t(13, 30));
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
        let run = SquareOff::new(t(13, 30)).on_clock(at(14, 0, 0), &mut b);
        assert_eq!(run.cancelled, vec![IntentId(5)]);
        assert_eq!(b.orders.len(), 1);
        assert_eq!(b.orders[0].id, IntentId(6));
    }

    #[test]
    fn squareoff_fills_on_next_tick_with_slippage_and_flat_position() {
        let mut b = WorkingBook::default();
        mis_long(&mut b, 4);
        SquareOff::new(t(13, 30)).on_clock(at(13, 30, 0), &mut b);
        let mut sim = FillSimulator::new(crate::fill::tests::cfg());
        let mut tk = crate::fill::tests::tick("100", 1000, 0);
        tk.at = at(13, 30, 1);
        let fills = sim.on_tick(&tk, tk.at, &mut b);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].reason, FillReason::SquareOff);
        assert_eq!(fills[0].price, d("99.90"));
        b.record(&fills[0]);
        assert_eq!(b.position("ABC", Product::Mis), 0);
    }

    #[test]
    fn no_open_mis_position_queues_nothing() {
        let mut b = WorkingBook::default();
        let run = SquareOff::new(t(13, 30)).on_clock(at(14, 0, 0), &mut b);
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
}
