//! `StrategyContext`: everything a strategy may know besides its own bars.
//!
//! Defined here, not on `risk::PortfolioSnapshot`: `strategies` may not depend on `risk`
//! (architecture test). The backtest and the scanner adapt their portfolio into a `PortfolioView`.
//! The option chain is left out: no option strategy exists in F8 and `options` has no chain type yet.

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::Instrument;
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_regime::RegimeState;
use chrono::DateTime;
use chrono_tz::{Asia::Kolkata, Tz};

use crate::exit::OpenPosition;

/// Instruments currently held and their open positions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortfolioView {
    pub positions: Vec<OpenPosition>,
}

impl PortfolioView {
    /// True when the instrument is already held (a strategy never adds to a position).
    #[must_use]
    pub fn holds(&self, instrument_token: u64) -> bool {
        self.positions
            .iter()
            .any(|p| p.instrument_token == instrument_token)
    }
}

/// The caller's side of the closed-bar guarantee failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContextError {
    #[error("the view has no bars")]
    EmptyView,
    #[error("a regime is as of {0}, after the decision bar closed: look-ahead")]
    RegimeFromTheFuture(String),
    #[error("a regime is as of {0}, not the decision bar's session: stale or misaligned")]
    RegimeStale(String),
    #[error("calendar: {0}")]
    Calendar(String),
}

/// Per-call inputs. Regimes must be computed by the caller from bars up to the decision bar,
/// with index and stock bars aligned by session date.
#[derive(Debug, Clone, Copy)]
pub struct StrategyContext<'a> {
    pub instrument: &'a Instrument,
    pub market_regime: &'a RegimeState,
    pub stock_regime: &'a RegimeState,
    pub portfolio: &'a PortfolioView,
    /// Built from `calendar.toml`; the only source of session dates (no date arithmetic in strategies).
    pub calendar: &'a MarketCalendar,
}

impl StrategyContext<'_> {
    /// Closed-bar guard: each regime must be as of the decision bar's own session, at or before its
    /// close. A later `as_of` is look-ahead; an earlier session is stale or misaligned (index and
    /// stock bars align by session date, never "latest available"). Strategies call this first and
    /// return `None` on `Err`. The other half is the caller's: regimes are built only from bars the
    /// view can see.
    ///
    /// # Errors
    /// `EmptyView`, `RegimeFromTheFuture` or `RegimeStale` for the first bad regime.
    pub fn check_closed_bar(&self, view: &DataView<'_>) -> Result<(), ContextError> {
        let last = view.last().ok_or(ContextError::EmptyView)?.close_time();
        for r in [self.market_regime, self.stock_regime] {
            if r.as_of > last {
                return Err(ContextError::RegimeFromTheFuture(r.as_of.to_rfc3339()));
            }
            if r.as_of.date_naive() != last.date_naive() {
                return Err(ContextError::RegimeStale(r.as_of.to_rfc3339()));
            }
        }
        Ok(())
    }

    /// Close of the next trading session after the decision bar: the `valid_until` of a signal.
    /// Holidays and special sessions come from the calendar.
    ///
    /// # Errors
    /// `EmptyView`, or `Calendar` when the date is outside the covered years or has no session.
    pub fn next_session_close(
        &self,
        view: &DataView<'_>,
        kind: InstrumentKind,
    ) -> Result<DateTime<Tz>, ContextError> {
        let cal = |e: bazaar_core::calendar::CalendarError| ContextError::Calendar(e.to_string());
        let day = view
            .last()
            .ok_or(ContextError::EmptyView)?
            .close_time()
            .date_naive();
        let next = self
            .calendar
            .next_trading_day(day, Segment::Equity)
            .map_err(cal)?;
        let close = self
            .calendar
            .close_time(next, Segment::Equity, kind)
            .map_err(cal)?;
        Ok(close.with_timezone(&Kolkata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazaar_core::candle::{Candle, Interval};
    use bazaar_core::config::{self, CalendarConfig};
    use bazaar_core::instrument::{Exchange, InstrumentType};
    use bazaar_regime::Fact;
    use rust_decimal::Decimal;

    use crate::signal::tests::ist;

    fn calendar() -> MarketCalendar {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        MarketCalendar::from_config(&config::load::<CalendarConfig>(&dir).unwrap())
    }

    fn inst() -> Instrument {
        Instrument {
            instrument_token: 7,
            exchange_token: 7,
            tradingsymbol: "ABC".into(),
            name: "ABC".into(),
            exchange: Exchange::parse("NSE"),
            instrument_type: InstrumentType::parse("EQ"),
            segment: "NSE".into(),
            expiry: None,
            strike: None,
            tick_size: Decimal::new(5, 2),
            lot_size: 1,
        }
    }

    /// March 2026: Mon 2, Wed 4, Thu 5. Tue 3 is a trading holiday in `calendar.toml`.
    fn bars_mon_wed_thu() -> Vec<Candle> {
        [2, 4, 5]
            .into_iter()
            .map(|d| {
                let p = Decimal::from(100);
                Candle::new(ist(d), Interval::Day, [p, p, p, p], 1, true).unwrap()
            })
            .collect()
    }

    #[test]
    fn regime_must_be_the_decision_session_not_later_and_not_earlier() {
        let (i, p, cal) = (inst(), PortfolioView::default(), calendar());
        let bars = bars_mon_wed_thu();
        let view = DataView::ending_at(&bars, 2).unwrap();
        let mk = |d| RegimeState::unknown(ist(d), Fact::new("t", "v", "m"));
        let (ok, late, stale) = (mk(5), mk(6), mk(4));
        let ctx = StrategyContext {
            instrument: &i,
            market_regime: &ok,
            stock_regime: &ok,
            portfolio: &p,
            calendar: &cal,
        };
        assert_eq!(ctx.check_closed_bar(&view), Ok(()));
        let bad = StrategyContext {
            stock_regime: &late,
            ..ctx
        };
        assert!(matches!(
            bad.check_closed_bar(&view),
            Err(ContextError::RegimeFromTheFuture(_))
        ));
        let bad = StrategyContext {
            market_regime: &stale,
            ..ctx
        };
        assert!(matches!(
            bad.check_closed_bar(&view),
            Err(ContextError::RegimeStale(_))
        ));
        let bad = StrategyContext {
            stock_regime: &stale,
            ..ctx
        };
        assert!(bad.check_closed_bar(&view).is_err());
    }

    #[test]
    fn next_session_close_skips_the_holiday_from_the_calendar() {
        let (i, p, cal) = (inst(), PortfolioView::default(), calendar());
        let bars = bars_mon_wed_thu();
        let r = RegimeState::unknown(ist(2), Fact::new("t", "v", "m"));
        let ctx = StrategyContext {
            instrument: &i,
            market_regime: &r,
            stock_regime: &r,
            portfolio: &p,
            calendar: &cal,
        };
        let kind = InstrumentKind::CashNonFno;
        // Mon 2 -> next session is Wed 4 (Tue 3 is a holiday), closing 15:30 IST.
        let v = DataView::ending_at(&bars, 0).unwrap();
        assert_eq!(ctx.next_session_close(&v, kind), Ok(ist(4)));
        // Wed 4 -> Thu 5.
        let v = DataView::ending_at(&bars, 1).unwrap();
        assert_eq!(ctx.next_session_close(&v, kind), Ok(ist(5)));
    }

    #[test]
    fn holds_matches_by_token() {
        let p = PortfolioView {
            positions: vec![OpenPosition {
                instrument_token: 7,
                entry_price: Decimal::from(1),
                stop: Decimal::ZERO,
                opened_at: ist(1),
                bars_held: 0,
            }],
        };
        assert!(p.holds(7) && !p.holds(8));
        assert!(!PortfolioView::default().holds(7));
    }
}
