//! `classify`: label one daily series at its last closed bar (spec `docs/strategy/F7-regime-definitions.md`).
//!
//! The series is replayed bar by bar through streaming F4 indicators, so the label at bar `t`
//! depends only on bars `0..=t` (no look-ahead) and the confirmation state is rebuilt from the
//! data (there is no `prev` state to carry; same input, same output). Candles become `f64`
//! through the single adapter in `bazaar-indicators` (`Ohlcv::try_from`). Every threshold comes
//! from `RegimeParams`. A label describes recent behaviour; it does not predict the next bar.

use std::cmp::Ordering;

use bazaar_core::candle::{Candle, Interval};
use bazaar_indicators::{AdapterError, Indicator, Ohlcv, ma::Ema, trend::Adx, trend::Atr};

use crate::{Fact, MarketVolSource, ParamsError, RegimeParams, RegimeState, Trend, Tri, VolState};

/// Which series is being labelled: the index (market regime) or a stock's own series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Series {
    Market,
    Stock,
}

/// Optional inputs from later data features (F3). `None` or empty means "not available", never a
/// guess. Percent values must be in 0..=100; anything else is treated as not available, with a reason.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Extras<'a> {
    /// Share of index members above their 50-day average, at the last bar.
    pub breadth_pct_above_50dma: Option<f64>,
    /// India VIX level at the last bar (shown only; not used in a rule).
    pub india_vix: Option<f64>,
    /// India VIX percentile per bar, oldest first, last value = last bar (may be shorter than the
    /// candles). It runs through the same `confirm_bars` rule as ATR% before it drives `vol`.
    pub vix_percentile_series: &'a [f64],
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
    #[error("bad series: {0}")]
    BadSeries(&'static str),
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
    #[allow(clippy::cast_precision_loss)] // window length is far below 2^52
    let n = window.len() as f64;
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

/// Daily, adjusted, strictly time-ordered: the only series this classifier means anything for.
fn check_series(candles: &[Candle]) -> Result<(), ClassifyError> {
    let mut prev = None;
    for c in candles {
        if c.interval() != Interval::Day {
            return Err(ClassifyError::BadSeries("every candle must be a daily bar"));
        }
        if !c.adjusted() {
            return Err(ClassifyError::BadSeries(
                "every candle must be corporate-action adjusted",
            ));
        }
        if prev.is_some_and(|t| c.close_time() <= t) {
            return Err(ClassifyError::BadSeries(
                "close times must be strictly increasing",
            ));
        }
        prev = Some(c.close_time());
    }
    Ok(())
}

/// Market Breakdown: close below the slow EMA, volatility High, and the market weak (trend Down, or
/// breadth below its limit when breadth is present). `Unknown` when a needed label is unknown.
fn breakdown(kind: Series, s: &Snap, vol: VolState, breadth: Option<f64>, p: &RegimeParams) -> Tri {
    match (kind, s.ema_slow) {
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
    }
}

fn pct_ok(v: f64) -> bool {
    v.is_finite() && (0.0..=100.0).contains(&v)
}

/// Classify `candles` (daily, adjusted, oldest first, closed bars only) at the last bar.
///
/// Fewer than `min_history_bars` bars gives an all-`Unknown` state with a reason. Breakdown is only
/// judged for `Series::Market`. Missing `extras` stay `None` and each gets a reason line. The
/// caller must pass only closed bars: a candle carries no "still forming" flag to check.
///
/// # Errors
/// Invalid params, no candles, a series that is not daily/adjusted/strictly ordered, a candle that
/// is not finite as `f64`, or a rejected period.
pub fn classify(
    candles: &[Candle],
    p: &RegimeParams,
    kind: Series,
    extras: Extras<'_>,
) -> Result<RegimeState, ClassifyError> {
    p.validate()?;
    let last = candles.last().ok_or(ClassifyError::NoCandles)?;
    check_series(candles)?;
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
        format!(
            "From the {} and {} day averages, their slope and trend strength (ADX), confirmed over {} closes.",
            p.ema_mid, p.ema_slow, p.confirm_bars
        ),
    )];
    let market = kind == Series::Market;
    let mut vol = s.vol;
    let mut vix_used = false;
    let series = extras.vix_percentile_series;
    let vix_valid = !series.is_empty() && series.iter().all(|v| pct_ok(*v));
    if market && !series.is_empty() && !vix_valid {
        reasons.push(Fact::new(
            "India VIX",
            "invalid",
            "India VIX percentile outside 0..=100; not used.",
        ));
    }
    if vix_valid && market && p.market_vol_source == MarketVolSource::VixIfAvailable {
        let mut c = Confirm::new();
        for v in series {
            c.push(vol_from_rank(*v, p), p.confirm_bars);
        }
        vol = c.confirmed.unwrap_or(VolState::Unknown);
        vix_used = true;
    } else if vix_valid || extras.india_vix.is_some() {
        reasons.push(Fact::new(
            "India VIX",
            "unused",
            "India VIX given but not used (stock series, or index ATR is the configured source).",
        ));
    } else if market && series.is_empty() {
        reasons.push(Fact::new("India VIX", "n/a", "India VIX not available."));
    }
    let swing_meaning = if vix_used {
        format!(
            "India VIX percentile, confirmed over {} closes.",
            p.confirm_bars
        )
    } else {
        format!(
            "Today's daily swing size (ATR%) against the last {} bars of this series.",
            p.vol_lookback_bars
        )
    };
    reasons.push(Fact::new("Swings", format!("{vol:?}"), swing_meaning));
    let breadth = extras
        .breadth_pct_above_50dma
        .filter(|b| market && pct_ok(*b));
    if market {
        if extras.breadth_pct_above_50dma.is_some() && breadth.is_none() {
            reasons.push(Fact::new(
                "Breadth",
                "invalid",
                "Breadth outside 0..=100; not used.",
            ));
        } else if breadth.is_none() {
            reasons.push(Fact::new("Breadth", "n/a", "Breadth not available."));
        }
    }
    if s.trend == Trend::Unknown || vol == VolState::Unknown {
        reasons.push(Fact::new(
            "Not settled",
            "Unknown",
            "No stable trend or swing state has been confirmed yet; no trade ideas from this label.",
        ));
    }
    Ok(RegimeState {
        trend: s.trend,
        vol,
        breakdown: breakdown(kind, s, vol, breadth, p),
        as_of,
        ema_mid: s.ema_mid,
        ema_slow: s.ema_slow,
        adx: s.adx,
        atr_pct: s.atr_pct,
        atr_pct_rank: s.rank,
        breadth_pct_above_50dma: breadth,
        india_vix: extras.india_vix.filter(|_| vix_used),
        vix_percentile: series.last().copied().filter(|_| vix_used),
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
    use chrono::{Duration, TimeZone};
    use chrono_tz::Asia::Kolkata;
    use proptest::prelude::*;
    use rust_decimal::Decimal;

    /// Synthetic bars: open = close, high/low = close +- `half`, volume `vol`.
    fn bars_with(
        closes: &[f64],
        half: f64,
        vol: u64,
        interval: Interval,
        adjusted: bool,
    ) -> Vec<Candle> {
        let d = |x: f64| Decimal::new((x * 100.0).round() as i64, 2);
        let t0 = Kolkata.with_ymd_and_hms(2026, 1, 1, 15, 30, 0).unwrap();
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let t = t0 + Duration::days(i as i64);
                Candle::new(
                    t,
                    interval,
                    [d(c), d(c + half), d(c - half), d(c)],
                    vol,
                    adjusted,
                )
                .unwrap()
            })
            .collect()
    }

    /// Daily, adjusted bars (what `classify` accepts).
    fn bars(closes: &[f64], half: f64, vol: u64) -> Vec<Candle> {
        bars_with(closes, half, vol, Interval::Day, true)
    }

    /// EMA after `k` bars of a straight line down by `s` per bar from a flat start `p0`.
    fn ema_after_fall(p0: f64, s: f64, k: i32, n: usize) -> f64 {
        let a = 2.0 / (n as f64 + 1.0);
        p0 - s * (f64::from(k) - (1.0 - a) / a * (1.0 - (1.0 - a).powi(k)))
    }

    fn market_with(closes: &[f64], x: Extras<'_>) -> RegimeState {
        classify(
            &bars(closes, 0.5, 1000),
            &shipped_params(),
            Series::Market,
            x,
        )
        .unwrap()
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
    fn flat_then_crash_has_hand_computed_values_and_breakdown() {
        // 300 flat bars at 1000 (high-low = 1) then 40 bars down 10 a bar.
        let flat = go(&[1000.0; 300], Series::Market);
        assert!((flat.ema_mid.unwrap() - 1000.0).abs() < 1e-9);
        assert!((flat.ema_slow.unwrap() - 1000.0).abs() < 1e-9);
        assert!((flat.atr_pct.unwrap() - 0.001).abs() < 1e-12); // ATR = 1
        assert_eq!(flat.atr_pct_rank, Some(50.0));
        assert_eq!((flat.vol, flat.breakdown), (VolState::Normal, Tri::False));

        let mut c = vec![1000.0; 300];
        c.extend((1..=40).map(|k| 1000.0 - 10.0 * f64::from(k)));
        let m = go(&c, Series::Market);
        let close = 600.0;
        assert!((m.ema_mid.unwrap() - ema_after_fall(1000.0, 10.0, 40, 50)).abs() < 1e-6);
        assert!((m.ema_slow.unwrap() - ema_after_fall(1000.0, 10.0, 40, 200)).abs() < 1e-6);
        // Wilder ATR: TR = 10.5 each falling bar, ATR_k = 10.5 - 9.5 * (13/14)^k.
        let atr = 10.5 - 9.5 * (13.0_f64 / 14.0).powi(40);
        assert!((m.atr_pct.unwrap() - atr / close).abs() < 1e-9);
        // Today is the highest of the 250-bar window: 249 below, itself counts as a tie (half).
        assert!((m.atr_pct_rank.unwrap() - 100.0 * 249.5 / 250.0).abs() < 1e-9);
        assert!(m.adx.unwrap() > 25.0);
        assert_eq!(
            (m.trend, m.vol, m.breakdown),
            (Trend::Down, VolState::High, Tri::True)
        );
        // A stock series gets labels but never a Breakdown.
        let stock = go(&c, Series::Stock);
        assert_eq!((stock.trend, stock.breakdown), (Trend::Down, Tri::Unknown));
        // Down is enough on its own: a healthy breadth does not rescue it.
        let x = Extras {
            breadth_pct_above_50dma: Some(60.0),
            ..Extras::default()
        };
        let b = market_with(&c, x);
        assert_eq!(
            (b.breakdown, b.breadth_pct_above_50dma),
            (Tri::True, Some(60.0))
        );
    }

    #[test]
    fn settled_trend_and_vol_never_give_unknown_market_breakdown() {
        let p = shipped_params();
        for seed in [3, 7, 11] {
            let all = bars(&walk(seed, 600), 5.0, 100);
            for k in p.min_history_bars()..all.len() {
                let s =
                    classify(all.get(..k).unwrap(), &p, Series::Market, Extras::default()).unwrap();
                let settled = s.trend != Trend::Unknown && s.vol != VolState::Unknown;
                assert_eq!(s.breakdown == Tri::Unknown, !settled, "seed {seed} k {k}");
            }
        }
    }

    #[test]
    fn breakdown_is_false_when_a_condition_fails() {
        let mut crash = vec![1000.0; 300];
        crash.extend((1..=40).map(|k| 1000.0 - 10.0 * f64::from(k)));
        // Calm volatility (VIX percentile low) in a Down market: no Breakdown.
        let calm = [10.0; 10];
        let s = market_with(
            &crash,
            Extras {
                vix_percentile_series: &calm,
                ..Extras::default()
            },
        );
        assert_eq!(
            (s.trend, s.vol, s.breakdown),
            (Trend::Down, VolState::Low, Tri::False)
        );
        // High volatility but close above the slow average (steady rally): no Breakdown.
        let hot = [95.0; 10];
        let s = market_with(
            &ramp(100.0, 1.0, 320),
            Extras {
                vix_percentile_series: &hot,
                ..Extras::default()
            },
        );
        assert_eq!(
            (s.trend, s.vol, s.breakdown),
            (Trend::Up, VolState::High, Tri::False)
        );
    }

    #[test]
    fn breadth_alone_can_make_a_non_down_market_a_breakdown() {
        // Slow drift of 0.1 a bar with a 2-point zigzag: Range, close below the slow average.
        let c: Vec<f64> = (0..320)
            .map(|i| 1000.0 - 0.1 * f64::from(i) + 2.0 * f64::from(i % 2))
            .collect();
        let hot = [95.0; 10];
        let at = |b: Option<f64>| {
            market_with(
                &c,
                Extras {
                    breadth_pct_above_50dma: b,
                    vix_percentile_series: &hot,
                    ..Extras::default()
                },
            )
        };
        let weak = at(Some(29.9));
        assert_eq!((weak.trend, weak.vol), (Trend::Range, VolState::High));
        assert_eq!(weak.breakdown, Tri::True);
        assert_eq!(at(Some(30.0)).breakdown, Tri::False); // limit is exclusive
        assert_eq!(at(None).breakdown, Tri::False); // no breadth: not guessed as weak
    }

    #[test]
    fn sideways_is_range_with_vol_unknown() {
        let c: Vec<f64> = (0..320).map(|i| 100.0 + f64::from(i % 2)).collect();
        let s = go(&c, Series::Stock);
        assert_eq!(s.trend, Trend::Range);
        assert!(s.adx.unwrap() < 20.0, "{:?}", s.adx);
        // ATR% alternates with the zigzag close, so no vol label is ever confirmed.
        assert!(s.atr_pct_rank.is_some());
        assert_eq!(s.vol, VolState::Unknown);
        assert!(s.reasons.iter().any(|r| r.label == "Not settled"));
    }

    #[test]
    fn series_must_be_daily_adjusted_and_ordered() {
        let p = shipped_params();
        let run_it = |c: &[Candle]| classify(c, &p, Series::Market, Extras::default());
        let c = ramp(100.0, 1.0, 300);
        let intraday = bars_with(&c, 0.5, 1, Interval::Min5, true);
        assert!(matches!(
            run_it(&intraday),
            Err(ClassifyError::BadSeries(_))
        ));
        let raw = bars_with(&c, 0.5, 1, Interval::Day, false);
        assert!(matches!(run_it(&raw), Err(ClassifyError::BadSeries(_))));
        let mut unsorted = bars(&c, 0.5, 1);
        unsorted.swap(10, 11);
        assert!(matches!(
            run_it(&unsorted),
            Err(ClassifyError::BadSeries(_))
        ));
        let mut dup = bars(&c, 0.5, 1);
        dup[11] = dup[10].clone();
        assert!(matches!(run_it(&dup), Err(ClassifyError::BadSeries(_))));
        assert!(run_it(&bars(&c, 0.5, 1)).is_ok());
    }

    #[test]
    fn vix_series_is_smoothed_and_range_checked() {
        // 3 closes at 95 after a Low run: still Low after 2, High after 3 (confirm_bars = 3).
        let mut v = vec![10.0; 20];
        v.extend([95.0, 95.0]);
        let c = ramp(100.0, 1.0, 320);
        let at = |s: &[f64]| {
            market_with(
                &c,
                Extras {
                    india_vix: Some(15.0),
                    vix_percentile_series: s,
                    ..Extras::default()
                },
            )
        };
        let two = at(&v);
        assert_eq!((two.vol, two.vix_percentile), (VolState::Low, Some(95.0)));
        v.push(95.0);
        assert_eq!(at(&v).vol, VolState::High);
        // Out-of-range or NaN values are not used: ATR% vol stands, with a reason.
        for bad in [[f64::NAN; 5], [120.0; 5], [-1.0; 5]] {
            let s = at(&bad);
            assert_eq!(
                (s.vol, s.vix_percentile, s.india_vix),
                (VolState::Low, None, None)
            );
            assert!(s.reasons.iter().any(|r| r.value == "invalid"));
        }
        // Out-of-range breadth is dropped with a reason.
        let b = market_with(
            &c,
            Extras {
                breadth_pct_above_50dma: Some(130.0),
                ..Extras::default()
            },
        );
        assert_eq!(b.breadth_pct_above_50dma, None);
        assert!(
            b.reasons
                .iter()
                .any(|r| r.label == "Breadth" && r.value == "invalid")
        );
    }

    #[test]
    fn unused_extras_are_not_echoed() {
        let c = ramp(100.0, 1.0, 320);
        let x = Extras {
            breadth_pct_above_50dma: Some(40.0),
            india_vix: Some(15.0),
            vix_percentile_series: &[90.0; 5],
        };
        let s = classify(&bars(&c, 0.5, 1), &shipped_params(), Series::Stock, x).unwrap();
        assert_eq!(
            (s.breadth_pct_above_50dma, s.india_vix, s.vix_percentile),
            (None, None, None)
        );
        let idx = RegimeParams {
            market_vol_source: MarketVolSource::IndexAtr,
            ..shipped_params()
        };
        let m = classify(&bars(&c, 0.5, 1), &idx, Series::Market, x).unwrap();
        assert_eq!((m.india_vix, m.vix_percentile), (None, None));
        assert_eq!(m.vol, VolState::Low);
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
        // Trend and vol labels together, over several seeds (a real random walk drifts).
        for seed in [3, 7, 11, 19, 23] {
            let snaps = run(&bars(&walk(seed, 1000), 5.0, 100), &shipped_params()).unwrap();
            let t = snaps
                .windows(2)
                .filter(|w| w[0].trend != w[1].trend)
                .count();
            let v = snaps.windows(2).filter(|w| w[0].vol != w[1].vol).count();
            // Measured on 1000 bars: trend 24..33, vol 21..47 (about 3 and 4 flips per 100 bars:
            // vol flips more because a percentile of a noisy series crosses 25/75 often).
            assert!(
                t <= 40 && v <= 60,
                "seed {seed}: trend {t}, vol {v} flips on 1000 bars"
            );
        }
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
        // Exactly min_history bars is enough for every label (boundary n-1 / n).
        let at_n = go(&ramp(100.0, 1.0, n), Series::Market);
        assert_eq!(
            (at_n.trend, at_n.vol, at_n.breakdown),
            (Trend::Up, VolState::Low, Tri::False)
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
            vix_percentile_series: &[90.0; 5],
            ..Extras::default()
        };
        let v = market_with(&ramp(100.0, 1.0, 320), x);
        assert_eq!(
            (v.vol, v.vix_percentile, v.india_vix),
            (VolState::High, Some(90.0), Some(15.0))
        );
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
        fn lookahead_prefix_output_ignores_later_bars(seed in any::<u64>(), k in 1usize..400) {
            let p = shipped_params();
            let all = bars(&walk(seed, 400), 5.0, 10);
            let full = run(&all, &p).unwrap();
            let prefix = run(all.get(..k).unwrap(), &p).unwrap();
            prop_assert_eq!(full.get(k - 1), prefix.last());
            let a = classify(all.get(..k).unwrap(), &p, Series::Market, Extras::default()).unwrap();
            prop_assert_eq!(a.as_of, all[k - 1].close_time());
            if k >= p.min_history_bars() {
                let f = &full[k - 1];
                prop_assert_eq!(a.trend, f.trend);
                prop_assert_eq!(a.vol, f.vol);
                prop_assert_eq!(a.breakdown, breakdown(Series::Market, f, f.vol, None, &p));
            } else {
                prop_assert_eq!(a.trend, Trend::Unknown);
                prop_assert_eq!(a.breakdown, Tri::Unknown);
            }
        }
    }
}
