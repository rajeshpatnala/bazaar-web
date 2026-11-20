//! Regime fit across all three strategies, through the shipped `config/strategies.toml`: a strategy
//! fires only in the cells and market trends its table declares, never on a market breakdown, and
//! never when a regime is Unknown or Mixed.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use bazaar_regime::{Trend, Tri, VolState};
use bazaar_strategies::StrategyConfig;
use common::{CASES, World, candles, on_bar, shipped, strategy};

const TRENDS: [Trend; 5] = [
    Trend::Up,
    Trend::Down,
    Trend::Range,
    Trend::Mixed,
    Trend::Unknown,
];
const VOLS: [VolState; 4] = [
    VolState::Low,
    VolState::Normal,
    VolState::High,
    VolState::Unknown,
];

#[test]
fn every_regime_cell_fires_only_where_the_shipped_config_allows() {
    let cfg = shipped();
    for c in &CASES {
        let fit = StrategyConfig::from_table(c.id, cfg.strategy.get(c.id).unwrap())
            .unwrap()
            .regime_fit(c.id)
            .unwrap();
        let bars = candles(c.csv, true);
        let mut s = strategy(c.id);
        let mut fired = 0;
        for mt in TRENDS {
            for st in TRENDS {
                for sv in VOLS {
                    let w = World::new((mt, VolState::Normal), (st, sv), &bars[c.k]);
                    let got = on_bar(s.as_mut(), &bars, c.k, &w).is_some();
                    let allowed =
                        fit.market_trends.contains(&mt) && fit.stock_cells.contains(&(st, sv));
                    assert_eq!(got, allowed, "{} market {mt:?} stock {st:?}/{sv:?}", c.id);
                    fired += usize::from(got);
                }
            }
        }
        assert!(fired > 0, "{} fires in at least one cell", c.id);
    }
}

#[test]
fn a_market_breakdown_or_down_market_suppresses_every_strategy() {
    for c in &CASES {
        let bars = candles(c.csv, true);
        let mut s = strategy(c.id);
        let mut w = World::for_case(c, &bars[c.k]);
        assert!(
            on_bar(s.as_mut(), &bars, c.k, &w).is_some(),
            "{} control",
            c.id
        );
        w.market.breakdown = Tri::True;
        assert!(
            on_bar(s.as_mut(), &bars, c.k, &w).is_none(),
            "{} breakdown",
            c.id
        );
        w.market.breakdown = Tri::Unknown;
        assert!(
            on_bar(s.as_mut(), &bars, c.k, &w).is_none(),
            "{} breakdown unknown",
            c.id
        );
        w.market.breakdown = Tri::False;
        w.market.trend = Trend::Down;
        assert!(
            on_bar(s.as_mut(), &bars, c.k, &w).is_none(),
            "{} down",
            c.id
        );
    }
}

#[test]
fn an_instrument_already_held_gives_no_signal() {
    use bazaar_strategies::OpenPosition;
    for c in &CASES {
        let bars = candles(c.csv, true);
        let mut w = World::for_case(c, &bars[c.k]);
        w.portfolio.positions.push(OpenPosition {
            instrument_token: w.instrument.instrument_token,
            entry_price: bars[c.k].close(),
            stop: bars[c.k].low(),
            opened_at: bars[c.k - 1].close_time(),
            bars_held: 1,
        });
        assert!(
            on_bar(strategy(c.id).as_mut(), &bars, c.k, &w).is_none(),
            "{}",
            c.id
        );
    }
}
