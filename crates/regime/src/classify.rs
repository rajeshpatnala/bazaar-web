//! `classify`: label one daily series at its last closed bar (spec `docs/strategy/F7-regime-definitions.md`).
//!
//! The series is replayed bar by bar through streaming F4 indicators, so the label at bar `t`
//! depends only on bars `0..=t` (no look-ahead) and the confirmation state is rebuilt from the
//! data (there is no `prev` state to carry; same input, same output). Candles become `f64`
//! through the single adapter in `bazaar-indicators` (`Ohlcv::try_from`). Every threshold comes
//! from `RegimeParams`. A label describes recent behaviour; it does not predict the next bar.

use std::cmp::Ordering;

use bazaar_core::candle::Candle;
use bazaar_indicators::{AdapterError, Indicator, Ohlcv, ma::Ema, trend::Adx, trend::Atr};

use crate::{Fact, MarketVolSource, ParamsError, RegimeParams, RegimeState, Trend, Tri, VolState};

/// Which series is being labelled: the index (market regime) or a stock's own series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Series {
    Market,
    Stock,
}

/// Optional inputs from later data features (F3). `None` means "not available", never a guess.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Extras {
    pub breadth_pct_above_50dma: Option<f64>,
    pub india_vix: Option<f64>,
    pub vix_percentile: Option<f64>,
}

/// Why a series could not be classified.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ClassifyError {
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error("no candles to classify")]
    NoCandles,
    #[error(transparent)]
    Candle(#[from] AdapterError),
    #[error("indicator period rejected: {0}")]
    Indicator(String),
}

/// Publishes a new label only after `need` consecutive closes agree on it.
#[derive(Debug)]
struct Confirm<T> {
    confirmed: Option<T>,
    cand: Option<T>,
    run: usize,
}

impl<T: Copy + PartialEq> Confirm<T> {
    const fn new() -> Self {
        Self {
            confirmed: None,
            cand: None,
            run: 0,
        }
    }

    fn push(&mut self, raw: T, need: usize) -> Option<T> {
        if self.confirmed == Some(raw) {
            self.cand = None;
            self.run = 0;
        } else {
            if self.cand == Some(raw) {
                self.run = self.run.saturating_add(1);
            } else {
                self.cand = Some(raw);
                self.run = 1;
            }
            if self.run >= need {
                self.confirmed = Some(raw);
                self.cand = None;
                self.run = 0;
            }
        }
        self.confirmed
    }
}

/// Indicator values at one bar that the raw trend rule reads.
#[derive(Debug, Clone, Copy)]
struct RawIn {
    close: f64,
    mid: f64,
    slow: f64,
    slope_pct: f64,
    adx: f64,
}

/// Raw trend before smoothing. Up/Down are tested first, then Range, else Mixed. A held Up (Down)
/// needs only `adx_trend_stay`; entering needs `adx_trend_enter`.
fn raw_trend(x: &RawIn, p: &RegimeParams, held: Option<Trend>) -> Trend {
    let need = |dir: Trend| {
        if held == Some(dir) {
            p.adx_trend_stay
        } else {
            p.adx_trend_enter
        }
    };
    if x.close > x.mid && x.mid > x.slow && x.slope_pct > 0.0 && x.adx >= need(Trend::Up) {
        Trend::Up
    } else if x.close < x.mid && x.mid < x.slow && x.slope_pct < 0.0 && x.adx >= need(Trend::Down) {
        Trend::Down
    } else if x.adx < p.adx_range_max && x.slope_pct.abs() <= p.slope_flat_max_pct {
        Trend::Range
    } else {
        Trend::Mixed
    }
}

/// Share of `window` below `today`, counting ties as half, in percent (0..=100). Ties as half keeps
/// a flat series at 50 (Normal) instead of calling it High.
fn percentile_rank(window: &[f64], today: f64) -> Option<f64> {
    let n: f64 = window.iter().map(|_| 1.0).sum();
    if n < 1.0 || !today.is_finite() {
        return None;
    }
    let below: f64 = window
        .iter()
        .map(|v| match v.partial_cmp(&today) {
            Some(Ordering::Less) => 1.0,
            Some(Ordering::Equal) => 0.5,
            _ => 0.0,
        })
        .sum();
    Some(100.0 * below / n)
}

fn vol_from_rank(rank: f64, p: &RegimeParams) -> VolState {
    if rank <= p.vol_low_pct {
        VolState::Low
    } else if rank >= p.vol_high_pct {
        VolState::High
    } else {
        VolState::Normal
    }
}

/// Everything known after one bar (compared across prefixes by the look-ahead test).
#[derive(Debug, Clone, PartialEq)]
struct Snap {
    trend: Trend,
    vol: VolState,
    close: f64,
    ema_mid: Option<f64>,
    ema_slow: Option<f64>,
    adx: Option<f64>,
    atr_pct: Option<f64>,
    rank: Option<f64>,
}

fn ind<T, E: std::fmt::Debug>(r: Result<T, E>) -> Result<T, ClassifyError> {
    r.map_err(|e| ClassifyError::Indicator(format!("{e:?}")))
}

fn run(candles: &[Candle], p: &RegimeParams) -> Result<Vec<Snap>, ClassifyError> {
    let (mut e_mid, mut e_slow) = (ind(Ema::new(p.ema_mid))?, ind(Ema::new(p.ema_slow))?);
    let (mut adx, mut atr) = (ind(Adx::new(p.adx_period))?, ind(Atr::new(p.atr_period))?);
    let (mut mids, mut atr_pcts): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    let (mut trend_c, mut vol_c) = (Confirm::new(), Confirm::new());
    let mut out = Vec::with_capacity(candles.len());
    for c in candles {
        let o = Ohlcv::try_from(c)?;
        let mid = e_mid.update(o.c);
        let slow = e_slow.update(o.c);
        let adx_v = adx.update(o).and_then(|a| a.adx);
        let atr_pct = atr.update(o).filter(|_| o.c > 0.0).map(|a| a / o.c);
        let past = mids
            .len()
            .checked_sub(p.slope_bars)
            .and_then(|i| mids.get(i))
            .copied()
            .filter(|v| *v > 0.0);
        let slope_pct = mid.zip(past).map(|(m, q)| (m - q) / q * 100.0);
        if let Some(m) = mid {
            mids.push(m);
        }
        let rank = atr_pct.and_then(|a| {
            atr_pcts.push(a);
            let start = atr_pcts.len().checked_sub(p.vol_lookback_bars)?;
            percentile_rank(atr_pcts.get(start..)?, a)
        });
        if let (Some(m), Some(s), Some(sl), Some(a)) = (mid, slow, slope_pct, adx_v) {
            let x = RawIn {
                close: o.c,
                mid: m,
                slow: s,
                slope_pct: sl,
                adx: a,
            };
            trend_c.push(raw_trend(&x, p, trend_c.confirmed), p.confirm_bars);
        }
        if let Some(r) = rank {
            vol_c.push(vol_from_rank(r, p), p.confirm_bars);
        }
        out.push(Snap {
            trend: trend_c.confirmed.unwrap_or(Trend::Unknown),
            vol: vol_c.confirmed.unwrap_or(VolState::Unknown),
            close: o.c,
            ema_mid: mid,
            ema_slow: slow,
            adx: adx_v,
            atr_pct,
            rank,
        });
    }
    Ok(out)
}

/// Classify `candles` (closed daily bars, oldest first) at the last bar.
///
/// Fewer than `min_history_bars` bars gives an all-`Unknown` state with a reason. Breakdown is only
/// judged for `Series::Market`. Missing `extras` stay `None` and each gets a reason line.
///
/// # Errors
/// Invalid params, no candles, a candle that is not finite as `f64`, or a rejected period.
pub fn classify(
    candles: &[Candle],
    p: &RegimeParams,
    kind: Series,
    extras: Extras,
) -> Result<RegimeState, ClassifyError> {
    p.validate()?;
    let last = candles.last().ok_or(ClassifyError::NoCandles)?;
    let as_of = last.close_time();
    let need = p.min_history_bars();
    if candles.len() < need {
        let reason = Fact::new(
            "Not enough data",
            format!("{} of {need} daily bars", candles.len()),
            "Not enough history to judge; no trade ideas from this label.",
        );
        return Ok(RegimeState::unknown(as_of, reason));
    }
    let snaps = run(candles, p)?;
    let s = snaps.last().ok_or(ClassifyError::NoCandles)?;
    let mut reasons = vec![Fact::new(
        "Trend",
        format!("{:?}", s.trend),
        "From the 50 and 200 day averages, their slope and trend strength (ADX), confirmed over several closes.",
    )];
    let mut vol = s.vol;
    let mut vix_used = false;
    let use_vix = kind == Series::Market && p.market_vol_source == MarketVolSource::VixIfAvailable;
    if let Some(v) = extras.vix_percentile.filter(|v| use_vix && v.is_finite()) {
        vol = vol_from_rank(v, p);
        vix_used = true;
    }
    reasons.push(Fact::new(
        "Swings",
        format!("{vol:?}"),
        if vix_used {
            "India VIX percentile (not smoothed)."
        } else {
            "Today's daily swing size (ATR%) against this series' last year."
        },
    ));
    let breadth = extras.breadth_pct_above_50dma.filter(|b| b.is_finite());
    if kind == Series::Market {
        if breadth.is_none() {
            reasons.push(Fact::new("Breadth", "n/a", "Breadth not available."));
        }
        if extras.india_vix.is_none() || extras.vix_percentile.is_none() {
            reasons.push(Fact::new("India VIX", "n/a", "India VIX not available."));
        }
    }
    let breakdown = match (kind, s.ema_slow) {
        (Series::Market, Some(slow)) if s.trend != Trend::Unknown && vol != VolState::Unknown => {
            let weak =
                s.trend == Trend::Down || breadth.is_some_and(|b| b < p.breadth_breakdown_max_pct);
            if s.close < slow && vol == VolState::High && weak {
                Tri::True
            } else {
                Tri::False
            }
        }
        _ => Tri::Unknown,
    };
    Ok(RegimeState {
        trend: s.trend,
        vol,
        breakdown,
        as_of,
        ema_mid: s.ema_mid,
        ema_slow: s.ema_slow,
        adx: s.adx,
        atr_pct: s.atr_pct,
        atr_pct_rank: s.rank,
        breadth_pct_above_50dma: breadth,
        india_vix: extras.india_vix,
        vix_percentile: extras.vix_percentile,
        reasons,
    })
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss
)]
mod tests {
    use super::*;
    use crate::params::shipped_params;
    use bazaar_core::candle::Interval;
    use chrono::{Duration, TimeZone};
    use chrono_tz::Asia::Kolkata;
    use proptest::prelude::*;
    use rust_decimal::Decimal;

    /// Synthetic daily bars: open = close, high/low = close +- `half`, volume `vol`.
    fn bars(closes: &[f64], half: f64, vol: u64) -> Vec<Candle> {
        let d = |x: f64| Decimal::new((x * 100.0).round() as i64, 2);
        let t0 = Kolkata.with_ymd_and_hms(2026, 1, 1, 15, 30, 0).unwrap();
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let t = t0 + Duration::days(i as i64);
                Candle::new(
                    t,
                    Interval::Day,
                    [d(c), d(c + half), d(c - half), d(c)],
                    vol,
                    false,
                )
                .unwrap()
            })
            .collect()
    }

    fn ramp(start: f64, step: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| start + step * i as f64).collect()
    }

    /// Deterministic noise in [-1, 1) from a seed (no rand dependency).
    fn noise(seed: u64, n: usize) -> Vec<f64> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
            })
            .collect()
    }

    fn walk(seed: u64, n: usize) -> Vec<f64> {
        let mut x = 1000.0;
        noise(seed, n)
            .iter()
            .map(|e| {
                x += e * 10.0;
                x
            })
            .collect()
    }

    fn go(closes: &[f64], kind: Series) -> RegimeState {
        classify(
            &bars(closes, 0.5, 1000),
            &shipped_params(),
            kind,
            Extras::default(),
        )
        .unwrap()
    }

    #[test]
    fn uptrend_is_up_with_hand_computed_values() {
        // close = 100 + t, 320 bars. Steady state: EMA(N) = close - slope*(N-1)/2, so
        // EMA50 = close - 24.5 and EMA200 = close - 99.5. TR is 1.5 every bar so ATR = 1.5.
        let s = go(&ramp(100.0, 1.0, 320), Series::Market);
        let close = 419.0;
        assert!((s.ema_mid.unwrap() - (close - 24.5)).abs() < 0.01);
        assert!((s.ema_slow.unwrap() - (close - 99.5)).abs() < 0.01);
        assert!((s.atr_pct.unwrap() - 1.5 / close).abs() < 1e-9);
        assert_eq!(
            (s.trend, s.vol, s.breakdown),
            (Trend::Up, VolState::Low, Tri::False)
        );
        assert!(s.adx.unwrap() > 90.0);
        assert_eq!(s.as_of, bars(&[1.0; 320], 0.5, 1)[319].close_time());
    }

    #[test]
    fn downtrend_and_crash_are_down_and_breakdown() {
        // close = 1000 - t: ATR% rises as price falls, so today is the highest of the year.
        let c = ramp(1000.0, -1.0, 320);
        let m = go(&c, Series::Market);
        assert_eq!(
            (m.trend, m.vol, m.breakdown),
            (Trend::Down, VolState::High, Tri::True)
        );
        assert!(
            m.reasons
                .iter()
                .any(|r| r.meaning == "Breadth not available.")
        );
        let stock = go(&c, Series::Stock);
        assert_eq!((stock.trend, stock.breakdown), (Trend::Down, Tri::Unknown));
        // Breadth present and healthy does not rescue a Down market from Breakdown (Down is enough).
        let x = Extras {
            breadth_pct_above_50dma: Some(60.0),
            ..Extras::default()
        };
        let b = classify(&bars(&c, 0.5, 1), &shipped_params(), Series::Market, x).unwrap();
        assert_eq!(
            (b.breakdown, b.breadth_pct_above_50dma),
            (Tri::True, Some(60.0))
        );
    }

    #[test]
    fn sideways_is_range() {
        let c: Vec<f64> = (0..320).map(|i| 100.0 + f64::from(i % 2)).collect();
        let s = go(&c, Series::Stock);
        assert_eq!(s.trend, Trend::Range);
        assert!(s.adx.unwrap() < 20.0, "{:?}", s.adx);
        // ATR% alternates with the zigzag close, so no vol label is ever confirmed (Unknown).
        assert!(s.atr_pct_rank.is_some());
    }

    #[test]
    fn range_trend_boundary_and_hysteresis_on_strength() {
        let p = shipped_params(); // stay 20, enter 25, range_max 20, flat 2.0
        let up = |adx, slope_pct| RawIn {
            close: 110.0,
            mid: 105.0,
            slow: 100.0,
            slope_pct,
            adx,
        };
        assert_eq!(raw_trend(&up(25.0, 1.0), &p, None), Trend::Up);
        assert_eq!(raw_trend(&up(24.9, 1.0), &p, None), Trend::Mixed); // between stay and enter
        assert_eq!(raw_trend(&up(24.9, 1.0), &p, Some(Trend::Up)), Trend::Up); // held trend
        assert_eq!(raw_trend(&up(20.0, 1.0), &p, Some(Trend::Up)), Trend::Up); // stay is inclusive
        assert_eq!(raw_trend(&up(19.9, 1.0), &p, Some(Trend::Up)), Trend::Range); // Range, flat slope
        assert_eq!(raw_trend(&up(19.9, 2.5), &p, Some(Trend::Up)), Trend::Mixed); // slope too steep
        assert_eq!(raw_trend(&up(20.0, 1.0), &p, None), Trend::Mixed); // range_max is exclusive
    }

    #[test]
    fn one_contrary_close_never_flips() {
        let mut c = Confirm::new();
        let seq = [1, 1, 1, 2, 1, 1, 2, 2, 1, 1];
        let got: Vec<_> = seq.iter().map(|&r| c.push(r, 3)).collect();
        assert!(got.iter().all(|g| *g != Some(2)));
        assert_eq!(got[0..2], [None, None]);
        assert_eq!(got[2], Some(1));
        let mut c = Confirm::new();
        let flips: Vec<_> = [1, 1, 1, 2, 2, 2].iter().map(|&r| c.push(r, 3)).collect();
        assert_eq!(flips[5], Some(2)); // three agreeing closes do flip
    }

    #[test]
    fn noisy_series_flips_rarely() {
        let snaps = run(&bars(&walk(7, 1000), 5.0, 100), &shipped_params()).unwrap();
        let trends: Vec<_> = snaps.iter().map(|s| s.trend).collect();
        let flips = trends.windows(2).filter(|w| w[0] != w[1]).count();
        // Measured 24 on this seed (2.4 per 100 bars, a real random walk drifts); bound 30.
        assert!(flips <= 30, "trend flips on 1000 bars: {flips}");
    }

    #[test]
    fn short_series_is_unknown_with_reason() {
        let p = shipped_params();
        let n = p.min_history_bars();
        let s = go(&ramp(100.0, 1.0, n - 1), Series::Market);
        assert_eq!(
            (s.trend, s.vol, s.breakdown),
            (Trend::Unknown, VolState::Unknown, Tri::Unknown)
        );
        assert_eq!(s.reasons[0].label, "Not enough data");
        assert_ne!(
            go(&ramp(100.0, 1.0, n + 40), Series::Market).trend,
            Trend::Unknown
        );
        assert_eq!(
            classify(&[], &p, Series::Market, Extras::default()),
            Err(ClassifyError::NoCandles)
        );
    }

    #[test]
    fn flat_and_zero_volume_series_have_no_nan() {
        let s = classify(
            &bars(&[100.0; 320], 0.0, 0),
            &shipped_params(),
            Series::Market,
            Extras::default(),
        )
        .unwrap();
        for v in [s.ema_mid, s.ema_slow, s.adx, s.atr_pct, s.atr_pct_rank]
            .into_iter()
            .flatten()
        {
            assert!(v.is_finite());
        }
        assert_eq!(s.vol, VolState::Normal); // all ATR% equal: rank 50, not High
        assert_ne!(s.trend, Trend::Up);
    }

    #[test]
    fn missing_extras_stay_none_with_reasons_and_vix_is_used_when_given() {
        let s = go(&ramp(100.0, 1.0, 320), Series::Market);
        assert!(
            s.india_vix.is_none()
                && s.vix_percentile.is_none()
                && s.breadth_pct_above_50dma.is_none()
        );
        assert!(
            s.reasons
                .iter()
                .any(|r| r.meaning == "India VIX not available.")
        );
        let x = Extras {
            india_vix: Some(15.0),
            vix_percentile: Some(90.0),
            ..Extras::default()
        };
        let v = classify(
            &bars(&ramp(100.0, 1.0, 320), 0.5, 1),
            &shipped_params(),
            Series::Market,
            x,
        )
        .unwrap();
        assert_eq!((v.vol, v.vix_percentile), (VolState::High, Some(90.0)));
        let idx = RegimeParams {
            market_vol_source: MarketVolSource::IndexAtr,
            ..shipped_params()
        };
        let w = classify(
            &bars(&ramp(100.0, 1.0, 320), 0.5, 1),
            &idx,
            Series::Market,
            x,
        )
        .unwrap();
        assert_eq!(w.vol, VolState::Low);
    }

    #[test]
    fn invalid_params_are_refused_and_rank_is_a_percentile() {
        let bad = RegimeParams {
            confirm_bars: 0,
            ..shipped_params()
        };
        assert!(matches!(
            classify(
                &bars(&[1.0], 0.0, 1),
                &bad,
                Series::Stock,
                Extras::default()
            ),
            Err(ClassifyError::Params(_))
        ));
        assert_eq!(percentile_rank(&[1.0, 2.0, 3.0, 4.0], 4.0), Some(87.5));
        assert_eq!(percentile_rank(&[2.0, 2.0], 2.0), Some(50.0));
        assert_eq!(percentile_rank(&[], 1.0), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]
        #[test]
        fn lookahead_prefix_output_ignores_later_bars(seed in any::<u64>(), k in 270usize..360) {
            let p = shipped_params();
            let all = bars(&walk(seed, 400), 5.0, 10);
            let full = run(&all, &p).unwrap();
            let prefix = run(all.get(..k).unwrap(), &p).unwrap();
            prop_assert_eq!(full.get(k - 1), prefix.last());
            let a = classify(all.get(..k).unwrap(), &p, Series::Market, Extras::default()).unwrap();
            prop_assert_eq!(a.trend, full[k - 1].trend);
            prop_assert_eq!(a.as_of, all[k - 1].close_time());
        }
    }
}
