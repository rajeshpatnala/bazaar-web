//! F7 T2 acceptance tests: Downtrend fixture and the +-20% stability sweep (carried gaps from PR 70/72).
//! Synthetic daily bars only; params come from the shipped `config/strategies.toml`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{StrategiesConfig, load};
use bazaar_regime::{Extras, RegimeParams, Series, Trend, Tri, VolState, classify};
use chrono::{Duration, TimeZone};
use chrono_tz::Asia::Kolkata;
use rust_decimal::Decimal;
use std::cell::Cell;

fn shipped() -> RegimeParams {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let c: StrategiesConfig = load(&dir).expect("strategies.toml loads");
    RegimeParams::from_config(&c).expect("params")
}

fn bars(closes: &[f64]) -> Vec<Candle> {
    bars_with_range(closes, |_| 0.5)
}

/// Daily bars stamped with their close time (15:30 IST) one calendar day apart; the half-range is
/// `half(close)`.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn bars_with_range(closes: &[f64], half: impl Fn(f64) -> f64) -> Vec<Candle> {
    let d = |x: f64| Decimal::new((x * 100.0).round() as i64, 2);
    let t0 = Kolkata.with_ymd_and_hms(2026, 1, 1, 15, 30, 0).unwrap();
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let h = half(c);
            Candle::new(
                t0 + Duration::days(i as i64),
                Interval::Day,
                [d(c), d(c + h), d(c - h), d(c)],
                1000,
                true,
            )
            .unwrap()
        })
        .collect()
}

fn up() -> Vec<f64> {
    (0..320).map(|i| 100.0 + f64::from(i)).collect()
}

/// Steady decline of 1 per bar with a small deterministic wiggle (not a crash).
fn down() -> Vec<f64> {
    (0..320)
        .map(|i| 1000.0 - f64::from(i) + if i % 7 == 0 { 0.3 } else { 0.0 })
        .collect()
}

/// A slide (-0.1% a bar) whose daily range narrows over time, so ATR% falls while price falls.
/// The linear `down()` makes ATR% climb as the close shrinks, which the market rule reads as
/// high volatility, so `down()` is a Breakdown fixture as a market series.
fn slide_bars() -> Vec<Candle> {
    let c: Vec<f64> = (0..320).map(|i| 1000.0 * 0.999_f64.powi(i)).collect();
    let t = Cell::new(0.0_f64);
    bars_with_range(&c, |x| {
        let k = t.get();
        t.set(k + 1.0);
        x * 0.004 * (1.0 - k / 400.0)
    })
}

fn side() -> Vec<f64> {
    (0..320).map(|i| 100.0 + f64::from(i % 2)).collect()
}

fn crash() -> Vec<f64> {
    let mut c = vec![1000.0; 300];
    c.extend((1..=40).map(|k| 1000.0 - 10.0 * f64::from(k)));
    c
}

fn label_of(p: &RegimeParams, b: &[Candle], k: Series, x: Extras<'_>) -> (Trend, Tri, VolState) {
    let s = classify(b, p, k, x).unwrap();
    (s.trend, s.breakdown, s.vol)
}

#[test]
fn downtrend_fixture_is_down_with_hand_computed_values() {
    let p = shipped();
    let s = classify(&bars(&down()), &p, Series::Stock, Extras::default()).unwrap();
    // Last bar i=319 (319 % 7 = 4, no wiggle): close = 681. Steady slope -1 gives
    // EMA(N) = close + (N-1)/2, so EMA50 ~ 705.5 and EMA200 ~ 780.5 (wiggle adds < 0.3).
    let close = 681.0;
    assert_eq!(s.trend, Trend::Down);
    assert!(
        (s.ema_mid.unwrap() - (close + 24.5)).abs() < 0.5,
        "{:?}",
        s.ema_mid
    );
    assert!(
        (s.ema_slow.unwrap() - (close + 99.5)).abs() < 0.5,
        "{:?}",
        s.ema_slow
    );
    // Highs and lows fall 1 per bar, so ADX sits well above the shipped trend-stay limit.
    assert!(s.adx.unwrap() >= p.adx_trend_stay, "{:?}", s.adx);
    // A stock series never carries a Breakdown verdict.
    assert_eq!(s.breakdown, Tri::Unknown);
    // The last bar closes 15:30 IST, 319 days after the first.
    let expected = Kolkata.with_ymd_and_hms(2026, 1, 1, 15, 30, 0).unwrap() + Duration::days(319);
    assert_eq!(s.as_of, expected);
    // As a market series a slide with flat ATR% is Down and not a Breakdown (vol is not High).
    let m = classify(&slide_bars(), &p, Series::Market, Extras::default()).unwrap();
    assert_eq!((m.trend, m.breakdown), (Trend::Down, Tri::False));
    assert_ne!(m.vol, VolState::High);
    // The linear decline with a fixed range is a Breakdown as a market series (ATR% rises).
    let l = classify(&bars(&down()), &p, Series::Market, Extras::default()).unwrap();
    assert_eq!(
        (l.trend, l.breakdown, l.vol),
        (Trend::Down, Tri::True, VolState::High)
    );
}

#[test]
fn downtrend_is_deterministic_on_rebuilt_prefixes() {
    // The same prefix built twice gives the same state. Look-ahead is covered by the `lookahead_`
    // property test in the regime crate; this test only pins determinism.
    let p = shipped();
    let c = down();
    for cut in [270usize, 290, 310] {
        let a = classify(&bars(&c)[..cut], &p, Series::Market, Extras::default()).unwrap();
        let b = classify(&bars(&c[..cut]), &p, Series::Market, Extras::default()).unwrap();
        assert_eq!(a, b);
    }
}

#[test]
fn four_fixtures_hold_their_labels_across_plus_minus_20_percent() {
    let base = shipped();
    // Healthy breadth: Down alone still makes a crash a Breakdown at every scaled breadth limit.
    let breadth = Extras {
        breadth_pct_above_50dma: Some(60.0),
        ..Extras::default()
    };
    for f in [0.8_f64, 0.9, 1.0, 1.1, 1.2] {
        let mut p = base.clone();
        // Every limit in `RegimeParams` is scaled (windows and periods are not limits). The ADX
        // limits move together so stay >= range_max and enter >= stay keep holding.
        p.adx_trend_enter *= f;
        p.adx_trend_stay *= f;
        p.adx_range_max *= f;
        p.slope_flat_max_pct *= f;
        p.vol_low_pct *= f;
        p.vol_high_pct *= f;
        p.breadth_breakdown_max_pct *= f;
        p.validate().unwrap();
        let m = Series::Market;
        let none = Extras::default();
        assert_eq!(
            label_of(&p, &bars(&up()), m, none),
            (Trend::Up, Tri::False, VolState::Low),
            "up f={f}"
        );
        let d = label_of(&p, &slide_bars(), m, none);
        assert_eq!((d.0, d.1), (Trend::Down, Tri::False), "down f={f}");
        assert_ne!(d.2, VolState::High, "down vol f={f}");
        assert_eq!(
            label_of(&p, &bars(&side()), m, none),
            (Trend::Range, Tri::Unknown, VolState::Unknown),
            "range f={f}"
        );
        let crash_label = (Trend::Down, Tri::True, VolState::High);
        assert_eq!(
            label_of(&p, &bars(&crash()), m, none),
            crash_label,
            "crash f={f}"
        );
        assert_eq!(
            label_of(&p, &bars(&crash()), m, breadth),
            crash_label,
            "crash with breadth f={f}"
        );
    }
}

#[test]
fn confirm_bars_plus_minus_one_keeps_fixture_labels() {
    let base = shipped();
    for n in [
        base.confirm_bars - 1,
        base.confirm_bars,
        base.confirm_bars + 1,
    ] {
        let mut p = base.clone();
        p.confirm_bars = n;
        p.validate().unwrap();
        let x = Extras::default();
        let st = Series::Stock;
        assert_eq!(label_of(&p, &bars(&up()), st, x).0, Trend::Up, "n={n}");
        assert_eq!(label_of(&p, &bars(&down()), st, x).0, Trend::Down, "n={n}");
        assert_eq!(label_of(&p, &bars(&side()), st, x).0, Trend::Range, "n={n}");
    }
}
