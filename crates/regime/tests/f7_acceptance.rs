//! F7 T2 acceptance tests: Downtrend fixture and the +-20% stability sweep (carried gaps from PR 70/72).
//! Synthetic daily bars only; params come from the shipped `config/strategies.toml`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{StrategiesConfig, load};
use bazaar_regime::{Extras, RegimeParams, Series, Trend, Tri, VolState, classify};
use chrono::{Duration, TimeZone};
use chrono_tz::Asia::Kolkata;
use rust_decimal::Decimal;

fn shipped() -> RegimeParams {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let c: StrategiesConfig = load(&dir).expect("strategies.toml loads");
    RegimeParams::from_config(&c).expect("params")
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn bars(closes: &[f64]) -> Vec<Candle> {
    let d = |x: f64| Decimal::new((x * 100.0).round() as i64, 2);
    let t0 = Kolkata.with_ymd_and_hms(2026, 1, 1, 15, 30, 0).unwrap();
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            Candle::new(
                t0 + Duration::days(i as i64),
                Interval::Day,
                [d(c), d(c + 0.5), d(c - 0.5), d(c)],
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

fn side() -> Vec<f64> {
    (0..320).map(|i| 100.0 + f64::from(i % 2)).collect()
}

fn crash() -> Vec<f64> {
    let mut c = vec![1000.0; 300];
    c.extend((1..=40).map(|k| 1000.0 - 10.0 * f64::from(k)));
    c
}

fn label(p: &RegimeParams, c: &[f64], k: Series) -> (Trend, Tri, VolState) {
    let s = classify(&bars(c), p, k, Extras::default()).unwrap();
    (s.trend, s.breakdown, s.vol)
}

#[test]
fn downtrend_fixture_is_down_with_hand_computed_values() {
    let s = classify(&bars(&down()), &shipped(), Series::Stock, Extras::default()).unwrap();
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
    assert!(s.adx.unwrap() >= 25.0, "{:?}", s.adx);
    // A stock series never carries a Breakdown verdict.
    assert_eq!(s.breakdown, Tri::Unknown);
    assert_eq!(s.as_of, bars(&down()).last().unwrap().close_time());
}

#[test]
fn downtrend_is_deterministic_and_prefix_stable() {
    let p = shipped();
    let c = down();
    let full = bars(&c);
    for cut in [270usize, 290, 310] {
        let a = classify(&full[..cut], &p, Series::Market, Extras::default()).unwrap();
        let b = classify(&bars(&c[..cut]), &p, Series::Market, Extras::default()).unwrap();
        assert_eq!(a, b);
    }
}

#[test]
fn four_fixtures_hold_their_labels_across_plus_minus_20_percent() {
    let base = shipped();
    for f in [0.8_f64, 0.9, 1.0, 1.1, 1.2] {
        let mut p = base.clone();
        // ADX thresholds move together so stay >= range_max and enter >= stay keep holding.
        p.adx_trend_enter *= f;
        p.adx_trend_stay *= f;
        p.adx_range_max *= f;
        p.slope_flat_max_pct *= f;
        p.vol_low_pct *= f;
        p.vol_high_pct *= f;
        p.breadth_breakdown_max_pct *= f;
        p.validate().unwrap();
        assert_eq!(label(&p, &up(), Series::Market).0, Trend::Up, "up f={f}");
        assert_eq!(
            label(&p, &down(), Series::Market).0,
            Trend::Down,
            "down f={f}"
        );
        assert_eq!(
            label(&p, &side(), Series::Market).0,
            Trend::Range,
            "range f={f}"
        );
        let (t, b, _) = label(&p, &crash(), Series::Market);
        assert_eq!((t, b), (Trend::Down, Tri::True), "crash f={f}");
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
        assert_eq!(label(&p, &up(), Series::Stock).0, Trend::Up, "n={n}");
        assert_eq!(label(&p, &down(), Series::Stock).0, Trend::Down, "n={n}");
        assert_eq!(label(&p, &side(), Series::Stock).0, Trend::Range, "n={n}");
    }
}
