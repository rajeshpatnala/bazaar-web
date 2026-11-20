//! Shared support for the cross-strategy tests: the shipped `config/strategies.toml`, fixture bars
//! on real calendar sessions, and a context the strategies can read. Fakes and fixtures only.
#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc
)]

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{self, CalendarConfig, StrategiesConfig};
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_regime::{Fact, RegimeState, Trend, Tri, VolState};
use bazaar_strategies::registry::build_all;
use bazaar_strategies::{PortfolioView, Signal, Strategy, StrategyContext};
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// One strategy's hand-computed fixture: it fires on bar `k` and nowhere else, in this regime.
pub struct Case {
    pub id: &'static str,
    pub csv: &'static str,
    pub k: usize,
    pub market: (Trend, VolState),
    pub stock: (Trend, VolState),
}

pub const CASES: [Case; 3] = [
    Case {
        id: "donchian_breakout",
        csv: "donchian_entry.csv",
        k: 30,
        market: (Trend::Up, VolState::Normal),
        stock: (Trend::Up, VolState::Normal),
    },
    Case {
        id: "trend_pullback",
        csv: "trend_pullback_entry.csv",
        k: 59,
        market: (Trend::Up, VolState::Normal),
        stock: (Trend::Up, VolState::Normal),
    },
    Case {
        id: "bollinger_reversion",
        csv: "bollinger_entry.csv",
        k: 21,
        market: (Trend::Up, VolState::Normal),
        stock: (Trend::Range, VolState::Normal),
    },
];

fn config_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

/// The shipped strategies config (every strategy `enabled = false`).
pub fn shipped() -> StrategiesConfig {
    config::load::<StrategiesConfig>(&config_dir()).unwrap()
}

/// The shipped config with every strategy switched on, as the tests need them built.
pub fn enabled() -> StrategiesConfig {
    let mut cfg = shipped();
    for table in cfg.strategy.values_mut() {
        table.insert("enabled".into(), true.into());
    }
    cfg
}

/// A fresh strategy by id, built through the registry from the shipped params and regime cells.
pub fn strategy(id: &str) -> Box<dyn Strategy> {
    build_all(&enabled())
        .unwrap()
        .into_iter()
        .find(|s| s.meta().id == id)
        .unwrap()
}

pub fn calendar() -> MarketCalendar {
    MarketCalendar::from_config(&config::load::<CalendarConfig>(&config_dir()).unwrap())
}

fn decimal(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// Fixture candles (adjusted) on consecutive sessions from 2026-03-02, holidays skipped.
pub fn candles(csv: &str, adjusted: bool) -> Vec<Candle> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/strategies")
        .join(csv);
    let cal = calendar();
    let mut day = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
    let mut out = Vec::new();
    for line in std::fs::read_to_string(path).unwrap().lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let close = cal
            .close_time(day, Segment::Equity, InstrumentKind::CashNonFno)
            .unwrap()
            .with_timezone(&chrono_tz::Asia::Kolkata);
        let ohlc = [decimal(f[0]), decimal(f[1]), decimal(f[2]), decimal(f[3])];
        out.push(Candle::new(close, Interval::Day, ohlc, f[4].parse().unwrap(), adjusted).unwrap());
        day = cal.next_trading_day(day, Segment::Equity).unwrap();
    }
    out
}

/// Owns what a `StrategyContext` borrows.
pub struct World {
    pub instrument: Instrument,
    pub market: RegimeState,
    pub stock: RegimeState,
    pub portfolio: PortfolioView,
    pub calendar: MarketCalendar,
}

fn state(cell: (Trend, VolState), breakdown: Tri, bar: &Candle) -> RegimeState {
    RegimeState {
        trend: cell.0,
        vol: cell.1,
        breakdown,
        ..RegimeState::unknown(bar.close_time(), Fact::new("fixture", "n/a", "fixture"))
    }
}

impl World {
    /// Regimes dated on the close of `bar` (the decision bar), market breakdown `False`.
    pub fn new(market: (Trend, VolState), stock: (Trend, VolState), bar: &Candle) -> Self {
        Self {
            instrument: Instrument {
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
            },
            market: state(market, Tri::False, bar),
            stock: state(stock, Tri::Unknown, bar),
            portfolio: PortfolioView::default(),
            calendar: calendar(),
        }
    }

    pub fn for_case(c: &Case, bar: &Candle) -> Self {
        Self::new(c.market, c.stock, bar)
    }

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

/// `on_bar` on the view ending at bar `end`, with the regimes in `world`.
pub fn on_bar(s: &mut dyn Strategy, bars: &[Candle], end: usize, w: &World) -> Option<Signal> {
    s.on_bar(&DataView::ending_at(bars, end).unwrap(), &w.ctx())
}

/// The signal a case fires on its bar `k`.
pub fn fire(c: &Case) -> Signal {
    let bars = candles(c.csv, true);
    let w = World::for_case(c, &bars[c.k]);
    on_bar(strategy(c.id).as_mut(), &bars, c.k, &w).unwrap()
}
