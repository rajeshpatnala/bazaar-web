//! Fixture-series test kit for strategy tests: daily candles on real calendar sessions, an
//! instrument with a tick, regimes aligned to the decision bar, and a `World` that owns everything a
//! `StrategyContext` borrows. Fakes only; nothing here reads a clock or a store. Test support: it
//! may panic on a broken fixture, which is why the panic lints are relaxed in this module.

#![allow(
    clippy::missing_panics_doc,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{self, CalendarConfig};
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_regime::{Fact, RegimeFit, RegimeState, Trend, Tri, VolState};
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::context::{PortfolioView, StrategyContext};
use crate::strategy::StrategyMeta;

/// `(open, high, low, close, volume)` with whole-rupee prices.
pub type Row = (i64, i64, i64, i64, u64);

/// First fixture session (a Monday). Later sessions follow the calendar, holidays skipped.
const FIRST_SESSION: (i32, u32, u32) = (2026, 3, 2);

/// Index of the decision bar a fresh [`World`] is aligned to (a 5-bar series).
pub const DEFAULT_LAST: usize = 4;

/// The shipped `calendar.toml`.
#[must_use]
pub fn calendar() -> MarketCalendar {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    MarketCalendar::from_config(&config::load::<CalendarConfig>(&dir).unwrap())
}

/// Session dates, starting at the first fixture session.
fn sessions(cal: &MarketCalendar, n: usize) -> Vec<NaiveDate> {
    let (y, m, d) = FIRST_SESSION;
    let mut day = NaiveDate::from_ymd_opt(y, m, d).unwrap();
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(day);
        day = cal.next_trading_day(day, Segment::Equity).unwrap();
    }
    out
}

fn close_at(cal: &MarketCalendar, day: NaiveDate) -> DateTime<Tz> {
    cal.close_time(day, Segment::Equity, InstrumentKind::CashNonFno)
        .unwrap()
        .with_timezone(&chrono_tz::Asia::Kolkata)
}

/// Close time (IST) of fixture session `i`.
#[must_use]
pub fn session_time(i: usize) -> DateTime<Tz> {
    let cal = calendar();
    close_at(&cal, sessions(&cal, i + 1)[i])
}

/// Daily candles on consecutive sessions. Asserts the strictly increasing times the engine must
/// guarantee, and that every row is a valid candle.
#[must_use]
pub fn bars(rows: &[Row], adjusted: bool) -> Vec<Candle> {
    let cal = calendar();
    let out: Vec<Candle> = sessions(&cal, rows.len())
        .into_iter()
        .zip(rows)
        .map(|(day, &(o, h, l, c, v))| {
            let p = |x: i64| Decimal::from(x);
            Candle::new(
                close_at(&cal, day),
                Interval::Day,
                [p(o), p(h), p(l), p(c)],
                v,
                adjusted,
            )
            .unwrap()
        })
        .collect();
    assert!(
        out.windows(2)
            .all(|w| w[0].close_time() < w[1].close_time()),
        "fixture bars must be strictly increasing"
    );
    out
}

/// An NSE equity with the given tick (from the "instrument master" fixture, never hardcoded in code
/// under test).
#[must_use]
pub fn instrument(tick: Decimal) -> Instrument {
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
        tick_size: tick,
        lot_size: 1,
    }
}

/// A settled regime state with the given labels, as of `as_of`.
#[must_use]
pub fn regime(trend: Trend, vol: VolState, breakdown: Tri, as_of: DateTime<Tz>) -> RegimeState {
    RegimeState {
        trend,
        vol,
        breakdown,
        ..RegimeState::unknown(as_of, Fact::new("fixture", "n/a", "fixture regime"))
    }
}

/// Metadata of an uptrend strategy: market Up or Range; stock Up in any vol, High is weaker.
#[must_use]
pub fn meta(min_history_bars: usize) -> StrategyMeta {
    let cells = |vols: &[VolState]| vols.iter().map(|v| (Trend::Up, *v)).collect::<Vec<_>>();
    StrategyMeta {
        id: "test",
        name: "Test",
        version: 1,
        min_history_bars,
        regime_fit: RegimeFit {
            market_trends: vec![Trend::Up, Trend::Range],
            stock_cells: cells(&[VolState::Low, VolState::Normal, VolState::High]),
            weak_cells: cells(&[VolState::High]),
        },
        penalty_weight: 0.5,
    }
}

/// Owns what a `StrategyContext` borrows. Regimes start aligned to session [`DEFAULT_LAST`] (the
/// decision bar of a 5-bar series); use [`World::at`] for other lengths.
#[derive(Debug)]
pub struct World {
    pub instrument: Instrument,
    pub market: RegimeState,
    pub stock: RegimeState,
    pub portfolio: PortfolioView,
    pub calendar: MarketCalendar,
}

impl World {
    /// Tick 0.05, empty portfolio, regimes `(market, stock)` with market `Breakdown` = `False`.
    #[must_use]
    pub fn new(mt: Trend, mv: VolState, st: Trend, sv: VolState) -> Self {
        let t = session_time(DEFAULT_LAST);
        Self {
            instrument: instrument(Decimal::new(5, 2)),
            market: regime(mt, mv, Tri::False, t),
            stock: regime(st, sv, Tri::Unknown, t),
            portfolio: PortfolioView::default(),
            calendar: calendar(),
        }
    }

    /// Re-align both regimes to the close of session `last` (index of the decision bar).
    #[must_use]
    pub fn at(mut self, last: usize) -> Self {
        let t = session_time(last);
        self.market.as_of = t;
        self.stock.as_of = t;
        self
    }

    #[must_use]
    pub fn ctx(&self) -> StrategyContext<'_> {
        StrategyContext {
            instrument: &self.instrument,
            market_regime: &self.market,
            stock_regime: &self.stock,
            portfolio: &self.portfolio,
            calendar: &self.calendar,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_follow_calendar_sessions_and_skip_the_holiday() {
        let b = bars(&[(100, 101, 99, 100, 1); 3], true);
        // Mon 2, then Wed 4 (Tue 3 is a holiday in calendar.toml), then Thu 5.
        let days: Vec<_> = b.iter().map(|c| c.close_time().date_naive()).collect();
        assert_eq!(days[0], NaiveDate::from_ymd_opt(2026, 3, 2).unwrap());
        assert_eq!(days[1], NaiveDate::from_ymd_opt(2026, 3, 4).unwrap());
        assert_eq!(days[2], NaiveDate::from_ymd_opt(2026, 3, 5).unwrap());
        assert_eq!(session_time(1), b[1].close_time());
        assert!(b.iter().all(Candle::adjusted));
        assert!(!bars(&[(1, 1, 1, 1, 1)], false)[0].adjusted());
    }

    #[test]
    fn a_fresh_world_is_aligned_to_the_fifth_bar_and_at_moves_it() {
        let w = World::new(Trend::Up, VolState::Normal, Trend::Up, VolState::Normal);
        let b = bars(&[(100, 101, 99, 100, 1); 5], true);
        let view = bazaar_core::dataview::DataView::ending_at(&b, DEFAULT_LAST).unwrap();
        assert_eq!(w.ctx().check_closed_bar(&view), Ok(()));
        let w = w.at(2);
        assert_eq!(w.market.as_of, session_time(2));
        assert_eq!(w.stock.as_of, session_time(2));
    }

    #[test]
    #[should_panic(expected = "valid")]
    fn broken_rows_fail_loudly() {
        // high below low: `Candle::new` rejects it and the kit unwraps.
        let _ = bars(&[(100, 98, 99, 100, 1)], true);
    }
}
