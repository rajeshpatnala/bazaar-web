//! Rules must not depend on the absolute price level: scaling every fixture price by +-20% (and
//! rounding to the fixture instrument's tick) keeps each strategy firing on bar `k` only, with tick-multiple
//! levels and a valid signal (`validate()` plus `risk_per_share == entry_high - stop`). Also covers
//! a Downtrend stock regime on the firing bar (no signal). Unknown/Mixed stock regimes and market
//! `Breakdown`/`Down` are covered in `regime_fit.rs`.
// Test-only allow: the panic-free rule applies to library code, not this integration-test crate.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use bazaar_core::candle::Candle;
use bazaar_regime::{Trend, VolState};
use common::{CASES, World, candles, on_bar, strategy};
use rust_decimal::Decimal;

/// The tick of the shared fixture instrument (the same one the strategy context reads).
fn tick() -> Decimal {
    World::for_case(&CASES[0], &candles(CASES[0].csv, true)[0])
        .instrument
        .tick_size
}

fn scaled(bars: &[Candle], num: i64, den: i64, t: Decimal) -> Vec<Candle> {
    let f = |d: Decimal| (d * Decimal::from(num) / Decimal::from(den) / t).round() * t;
    bars.iter()
        .map(|c| {
            let ohlc = [c.open(), c.high(), c.low(), c.close()].map(f);
            Candle::new(c.close_time(), c.interval(), ohlc, c.volume(), true).unwrap()
        })
        .collect()
}

#[test]
fn rules_fire_on_the_same_bar_at_plus_and_minus_20_percent_prices() {
    let t = tick();
    for c in &CASES {
        for (num, den) in [(4, 5), (1, 1), (6, 5)] {
            let bars = scaled(&candles(c.csv, true), num, den, t);
            let mut s = strategy(c.id);
            for end in 0..bars.len() {
                let w = World::for_case(c, &bars[end]);
                let got = on_bar(s.as_mut(), &bars, end, &w);
                assert_eq!(got.is_some(), end == c.k, "{} x{num}/{den} bar {end}", c.id);
                if let Some(sig) = got {
                    for p in [sig.entry_low, sig.entry_high, sig.stop] {
                        assert_eq!(p % t, Decimal::ZERO, "{} tick {p}", c.id);
                    }
                    assert!(sig.stop < sig.entry_low, "{} stop below entry", c.id);
                    assert!(sig.entry_low <= sig.entry_high, "{} entry zone", c.id);
                    assert_eq!(sig.risk_per_share, sig.entry_high - sig.stop, "{}", c.id);
                    assert!(sig.reference_rr > Decimal::ZERO, "{}", c.id);
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
        // Positive control through the same `World::new` path: only the stock trend changes below.
        let control = World::new(c.market, c.stock, &bars[c.k]);
        assert!(
            on_bar(strategy(c.id).as_mut(), &bars, c.k, &control).is_some(),
            "{} control",
            c.id
        );
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
