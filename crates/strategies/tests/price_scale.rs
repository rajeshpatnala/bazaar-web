//! Rules must not depend on the absolute price level: scaling every fixture price by +-20% (and
//! rounding to the 0.05 tick) keeps each strategy firing on bar `k` only, with tick-multiple
//! levels and a valid signal. Also covers a Downtrend stock regime on the firing bar (no signal).
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use bazaar_core::candle::{Candle, Interval};
use bazaar_regime::{Trend, VolState};
use common::{CASES, World, candles, on_bar, strategy};
use rust_decimal::Decimal;

fn tick() -> Decimal {
    Decimal::new(5, 2)
}

fn scaled(bars: &[Candle], num: i64, den: i64) -> Vec<Candle> {
    let f = |d: Decimal| (d * Decimal::from(num) / Decimal::from(den) / tick()).round() * tick();
    bars.iter()
        .map(|c| {
            let ohlc = [c.open(), c.high(), c.low(), c.close()].map(f);
            Candle::new(c.close_time(), Interval::Day, ohlc, c.volume(), true).unwrap()
        })
        .collect()
}

#[test]
fn rules_fire_on_the_same_bar_at_plus_and_minus_20_percent_prices() {
    for c in &CASES {
        for (num, den) in [(4, 5), (1, 1), (6, 5)] {
            let bars = scaled(&candles(c.csv, true), num, den);
            let mut s = strategy(c.id);
            for end in 0..bars.len() {
                let w = World::for_case(c, &bars[end]);
                let got = on_bar(s.as_mut(), &bars, end, &w);
                assert_eq!(got.is_some(), end == c.k, "{} x{num}/{den} bar {end}", c.id);
                if let Some(sig) = got {
                    for p in [sig.entry_low, sig.entry_high, sig.stop] {
                        assert_eq!(p % tick(), Decimal::ZERO, "{} tick {p}", c.id);
                    }
                    assert!(sig.stop < sig.entry_low, "{} stop below entry", c.id);
                    assert!(sig.validate().is_ok(), "{}", c.id);
                }
            }
        }
    }
}

#[test]
fn a_downtrend_stock_never_fires_on_the_firing_bar() {
    for c in &CASES {
        let bars = candles(c.csv, true);
        for vol in [VolState::Low, VolState::Normal, VolState::High] {
            let w = World::new(c.market, (Trend::Down, vol), &bars[c.k]);
            assert!(
                on_bar(strategy(c.id).as_mut(), &bars, c.k, &w).is_none(),
                "{}",
                c.id
            );
        }
    }
}
