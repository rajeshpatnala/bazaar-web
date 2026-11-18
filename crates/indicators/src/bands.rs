//! Bollinger, Donchian, Keltner bands.
//!
//! Conventions (`bz-quant` "Warm-up and smoothing conventions"): Bollinger and Donchian first
//! yield at bar index N-1; Keltner uses an SMA-seeded EMA of closes and a Wilder ATR seeded with
//! the SMA of the first M true ranges (first ATR at bar index M).

use crate::{Indicator, Ohlcv};
use std::collections::VecDeque;
use thiserror::Error;

/// An indicator was constructed with an invalid parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ParamError {
    /// A period of 0 was given.
    #[error("period `{0}` must be at least 1")]
    ZeroPeriod(&'static str),
    /// A period above [`MAX_PERIOD`] was given.
    #[error("period `{0}` exceeds the maximum")]
    PeriodTooLarge(&'static str),
    /// A multiplier was not finite or not positive.
    #[error("multiplier `{0}` must be finite and positive")]
    BadMultiplier(&'static str),
}

/// Largest accepted period; keeps `n + 1` and window allocations bounded.
pub const MAX_PERIOD: usize = 10_000;

/// Reject a zero period or one above [`MAX_PERIOD`].
pub(crate) const fn period(n: usize, name: &'static str) -> Result<usize, ParamError> {
    if n == 0 {
        Err(ParamError::ZeroPeriod(name))
    } else if n > MAX_PERIOD {
        Err(ParamError::PeriodTooLarge(name))
    } else {
        Ok(n)
    }
}

/// True when every price and volume field of the bar is finite. Bars failing this are skipped
/// (the indicator returns `None` and its state is untouched) so NaN/inf never enter the state.
pub(crate) fn finite_bar(x: &Ohlcv) -> bool {
    [x.o, x.h, x.l, x.c, x.v].iter().all(|v| v.is_finite())
}

/// `usize` to `f64`; exact for any realistic window length.
#[allow(clippy::cast_precision_loss)]
pub(crate) const fn nf(n: usize) -> f64 {
    n as f64
}

/// Fixed-size rolling window of the last `n` values.
#[derive(Debug, Clone)]
pub(crate) struct Window {
    n: usize,
    buf: VecDeque<f64>,
}

impl Window {
    pub(crate) const fn new(n: usize) -> Self {
        Self {
            n,
            buf: VecDeque::new(),
        }
    }
    pub(crate) const fn n(&self) -> usize {
        self.n
    }
    /// Push a value; true once the window holds `n` values.
    pub(crate) fn push(&mut self, x: f64) -> bool {
        self.buf.push_back(x);
        if self.buf.len() > self.n {
            self.buf.pop_front();
        }
        self.is_full()
    }
    pub(crate) fn is_full(&self) -> bool {
        self.buf.len() == self.n
    }
    pub(crate) fn mean(&self) -> f64 {
        self.buf.iter().sum::<f64>() / nf(self.buf.len().max(1))
    }
    pub(crate) fn max(&self) -> f64 {
        self.buf.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    }
    pub(crate) fn min(&self) -> f64 {
        self.buf.iter().copied().fold(f64::INFINITY, f64::min)
    }
    pub(crate) fn clear(&mut self) {
        self.buf.clear();
    }
}

/// Bollinger output for one bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BollingerOut {
    /// SMA(N) of closes.
    pub mid: f64,
    /// `mid + k * sd`.
    pub upper: f64,
    /// `mid - k * sd`.
    pub lower: f64,
    /// `(close - lower) / (upper - lower)`; 0.5 when the bands have zero width.
    pub pct_b: f64,
    /// `(upper - lower) / mid`; 0 when `mid` is 0.
    pub bandwidth: f64,
}

/// Bollinger bands over closes, population standard deviation (divide by N). Warm-up N bars.
#[derive(Debug, Clone)]
pub struct Bollinger {
    k: f64,
    w: Window,
}

impl Bollinger {
    /// `n` closes, `k` standard deviations (defaults 20 and 2).
    ///
    /// # Errors
    /// `n == 0`, or `k` not finite and positive.
    pub fn new(n: usize, k: f64) -> Result<Self, ParamError> {
        let n = period(n, "n")?;
        if !k.is_finite() || k <= 0.0 {
            return Err(ParamError::BadMultiplier("k"));
        }
        Ok(Self {
            k,
            w: Window::new(n),
        })
    }
}

impl Indicator for Bollinger {
    type Input = f64;
    type Output = BollingerOut;
    fn warmup(&self) -> usize {
        self.w.n()
    }
    fn update(&mut self, close: f64) -> Option<BollingerOut> {
        if !close.is_finite() {
            return None;
        }
        if !self.w.push(close) {
            return None;
        }
        let mid = self.w.mean();
        let var = self
            .w
            .buf
            .iter()
            .map(|x| (x - mid) * (x - mid))
            .sum::<f64>()
            / nf(self.w.n());
        let sd = var.sqrt();
        let (upper, lower) = (self.k.mul_add(sd, mid), self.k.mul_add(-sd, mid));
        let width = upper - lower;
        let pct_b = if width > 0.0 {
            (close - lower) / width
        } else {
            0.5
        };
        let bandwidth = if mid == 0.0 { 0.0 } else { width / mid };
        Some(BollingerOut {
            mid,
            upper,
            lower,
            pct_b,
            bandwidth,
        })
    }
    fn reset(&mut self) {
        self.w.clear();
    }
}

/// Donchian channel output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DonchianOut {
    /// Highest high of the last N bars (including the current one).
    pub upper: f64,
    /// Lowest low of the last N bars.
    pub lower: f64,
    /// Midpoint.
    pub mid: f64,
}

/// Donchian channel. Warm-up N bars. The window includes the current bar: for a breakout test
/// compare the close with the previous bar's output, not the current one.
#[derive(Debug, Clone)]
pub struct Donchian {
    hi: Window,
    lo: Window,
}

impl Donchian {
    /// Channel over `n` bars.
    ///
    /// # Errors
    /// `n == 0`.
    pub fn new(n: usize) -> Result<Self, ParamError> {
        let n = period(n, "n")?;
        Ok(Self {
            hi: Window::new(n),
            lo: Window::new(n),
        })
    }
}

impl Indicator for Donchian {
    type Input = Ohlcv;
    type Output = DonchianOut;
    fn warmup(&self) -> usize {
        self.hi.n()
    }
    fn update(&mut self, x: Ohlcv) -> Option<DonchianOut> {
        if !finite_bar(&x) {
            return None;
        }
        let a = self.hi.push(x.h);
        let b = self.lo.push(x.l);
        (a && b).then(|| {
            let (upper, lower) = (self.hi.max(), self.lo.min());
            DonchianOut {
                upper,
                lower,
                mid: f64::midpoint(upper, lower),
            }
        })
    }
    fn reset(&mut self) {
        self.hi.clear();
        self.lo.clear();
    }
}

/// True range of a bar given the previous close.
fn true_range(x: &Ohlcv, prev_close: f64) -> f64 {
    (x.h - x.l)
        .max((x.h - prev_close).abs())
        .max((x.l - prev_close).abs())
}

/// Keltner channel output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeltnerOut {
    /// EMA of closes.
    pub mid: f64,
    /// `mid + m * ATR`.
    pub upper: f64,
    /// `mid - m * ATR`.
    pub lower: f64,
}

/// Keltner channel: EMA(N) of closes +/- m * Wilder ATR(M). First `Some` at bar index
/// `max(N-1, M)`, so warm-up is `max(N, M+1)` bars.
#[derive(Debug, Clone)]
pub struct Keltner {
    ema_n: usize,
    atr_n: usize,
    mult: f64,
    bars: usize,
    prev_close: Option<f64>,
    ema_seed: f64,
    ema: Option<f64>,
    tr_sum: f64,
    tr_count: usize,
    atr: Option<f64>,
}

impl Keltner {
    /// EMA period `ema_n`, ATR period `atr_n`, multiplier `mult`.
    ///
    /// # Errors
    /// A zero period, or `mult` not finite and positive.
    pub fn new(ema_n: usize, atr_n: usize, mult: f64) -> Result<Self, ParamError> {
        let ema_n = period(ema_n, "ema_n")?;
        let atr_n = period(atr_n, "atr_n")?;
        if !mult.is_finite() || mult <= 0.0 {
            return Err(ParamError::BadMultiplier("mult"));
        }
        Ok(Self {
            ema_n,
            atr_n,
            mult,
            bars: 0,
            prev_close: None,
            ema_seed: 0.0,
            ema: None,
            tr_sum: 0.0,
            tr_count: 0,
            atr: None,
        })
    }
}

impl Indicator for Keltner {
    type Input = Ohlcv;
    type Output = KeltnerOut;
    fn warmup(&self) -> usize {
        self.ema_n.max(self.atr_n + 1)
    }
    fn update(&mut self, x: Ohlcv) -> Option<KeltnerOut> {
        if !finite_bar(&x) {
            return None;
        }
        self.bars += 1;
        // EMA, SMA-seeded.
        if let Some(e) = self.ema {
            let alpha = 2.0 / (nf(self.ema_n) + 1.0);
            self.ema = Some(e + alpha * (x.c - e));
        } else {
            self.ema_seed += x.c;
            if self.bars == self.ema_n {
                self.ema = Some(self.ema_seed / nf(self.ema_n));
            }
        }
        // ATR, Wilder, seeded with the mean of the first M true ranges.
        if let Some(pc) = self.prev_close {
            let tr = true_range(&x, pc);
            if let Some(a) = self.atr {
                self.atr = Some(a + (tr - a) / nf(self.atr_n));
            } else {
                self.tr_sum += tr;
                self.tr_count += 1;
                if self.tr_count == self.atr_n {
                    self.atr = Some(self.tr_sum / nf(self.atr_n));
                }
            }
        }
        self.prev_close = Some(x.c);
        let (mid, atr) = (self.ema?, self.atr?);
        Some(KeltnerOut {
            mid,
            upper: self.mult.mul_add(atr, mid),
            lower: self.mult.mul_add(-atr, mid),
        })
    }
    fn reset(&mut self) {
        self.bars = 0;
        self.prev_close = None;
        self.ema_seed = 0.0;
        self.ema = None;
        self.tr_sum = 0.0;
        self.tr_count = 0;
        self.atr = None;
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::Ohlcv;

    /// Deterministic bar from (high, low, close); open = close, volume given.
    #[allow(clippy::many_single_char_names)]
    pub(crate) fn bar(i: i64, h: f64, l: f64, c: f64, v: f64) -> Ohlcv {
        Ohlcv {
            ts_ms: i * 60_000,
            o: c,
            h,
            l,
            c,
            v,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::bar;
    use super::*;
    use crate::{batch, testkit};
    use proptest::prelude::*;

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-8, "{a} vs {b}");
    }

    #[test]
    fn rejects_bad_params() {
        assert_eq!(
            Bollinger::new(0, 2.0).unwrap_err(),
            ParamError::ZeroPeriod("n")
        );
        assert_eq!(
            Bollinger::new(5, 0.0).unwrap_err(),
            ParamError::BadMultiplier("k")
        );
        assert!(Bollinger::new(5, f64::NAN).is_err());
        assert!(Donchian::new(0).is_err());
        assert_eq!(
            Keltner::new(0, 3, 1.0).unwrap_err(),
            ParamError::ZeroPeriod("ema_n")
        );
        assert_eq!(
            Keltner::new(3, 0, 1.0).unwrap_err(),
            ParamError::ZeroPeriod("atr_n")
        );
        assert!(Keltner::new(3, 3, -1.0).is_err());
        assert_eq!(
            ParamError::ZeroPeriod("n").to_string(),
            "period `n` must be at least 1"
        );
    }

    #[test]
    fn bollinger_hand_cases() {
        // closes 2,4,4,4,5,5,7,9: mean 5, population variance 4, sd 2 (classic textbook set).
        let xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let out = batch(Bollinger::new(8, 2.0).unwrap(), xs);
        assert!(out[..7].iter().all(Option::is_none));
        let o = out[7].unwrap();
        near(o.mid, 5.0);
        near(o.upper, 9.0);
        near(o.lower, 1.0);
        // %B = (9 - 1) / (9 - 1) = 1; bandwidth = 8 / 5 = 1.6
        near(o.pct_b, 1.0);
        near(o.bandwidth, 1.6);
        // N=2 over 1,3: mid 2, sd 1, k=1 -> 3 and 1
        let o = batch(Bollinger::new(2, 1.0).unwrap(), [1.0, 3.0])[1].unwrap();
        near(o.upper, 3.0);
        near(o.lower, 1.0);
    }

    #[test]
    fn rejects_huge_period_and_skips_non_finite() {
        assert_eq!(
            Bollinger::new(MAX_PERIOD + 1, 2.0).unwrap_err(),
            ParamError::PeriodTooLarge("n")
        );
        assert!(Bollinger::new(MAX_PERIOD, 2.0).is_ok());
        assert!(Keltner::new(2, usize::MAX, 1.0).is_err());
        assert_eq!(
            ParamError::PeriodTooLarge("n").to_string(),
            "period `n` exceeds the maximum"
        );
        let out = batch(
            Bollinger::new(2, 2.0).unwrap(),
            [1.0, f64::NAN, 3.0, f64::INFINITY],
        );
        assert_eq!(out[1], None);
        assert_eq!(out[3], None);
        near(out[2].unwrap().mid, 2.0);
        let bad = Ohlcv {
            h: f64::NAN,
            ..bar(0, 1.0, 1.0, 1.0, 1.0)
        };
        assert_eq!(batch(Donchian::new(1).unwrap(), [bad])[0], None);
        assert_eq!(batch(Keltner::new(1, 1, 1.0).unwrap(), [bad])[0], None);
    }

    #[test]
    fn bollinger_flat_and_zero_mid() {
        let o = batch(Bollinger::new(3, 2.0).unwrap(), [5.0; 3])[2].unwrap();
        near(o.pct_b, 0.5);
        near(o.bandwidth, 0.0);
        let o = batch(Bollinger::new(2, 2.0).unwrap(), [-1.0, 1.0])[1].unwrap();
        near(o.mid, 0.0);
        near(o.bandwidth, 0.0);
    }

    #[test]
    fn donchian_hand_case() {
        // N=3 highs 5,7,6,4 lows 1,2,0,3: bar2 -> 7/0 mid 3.5; bar3 -> highs 7,6,4 lows 2,0,3
        let xs = [
            bar(0, 5.0, 1.0, 3.0, 1.0),
            bar(1, 7.0, 2.0, 3.0, 1.0),
            bar(2, 6.0, 0.0, 3.0, 1.0),
            bar(3, 4.0, 3.0, 3.0, 1.0),
        ];
        let out = batch(Donchian::new(3).unwrap(), xs);
        assert!(out[0].is_none() && out[1].is_none());
        assert_eq!(
            out[2].unwrap(),
            DonchianOut {
                upper: 7.0,
                lower: 0.0,
                mid: 3.5
            }
        );
        assert_eq!(
            out[3].unwrap(),
            DonchianOut {
                upper: 7.0,
                lower: 0.0,
                mid: 3.5
            }
        );
        let o = batch(Donchian::new(2).unwrap(), xs)[3].unwrap();
        assert_eq!(
            o,
            DonchianOut {
                upper: 6.0,
                lower: 0.0,
                mid: 3.0
            }
        );
    }

    #[test]
    fn keltner_hand_case() {
        // EMA(2), ATR(2), m=2. closes 10,12,11,13; H/L: (11,9),(13,11),(12,10),(14,12).
        // EMA seed at bar1 = 11; alpha 2/3: bar2 = 11 + (11-11)*2/3 = 11; bar3 = 11 + (13-11)*2/3 = 12.3333.
        // TR bar1 = max(2,|13-10|,|11-10|) = 3; bar2 = max(2,|12-12|,|10-12|) = 2 -> ATR(bar2) = 2.5;
        // TR bar3 = max(2,|14-11|,|12-11|) = 3 -> ATR = 2.5 + (3-2.5)/2 = 2.75.
        let xs = [
            bar(0, 11.0, 9.0, 10.0, 1.0),
            bar(1, 13.0, 11.0, 12.0, 1.0),
            bar(2, 12.0, 10.0, 11.0, 1.0),
            bar(3, 14.0, 12.0, 13.0, 1.0),
        ];
        let out = batch(Keltner::new(2, 2, 2.0).unwrap(), xs);
        assert!(out[..2].iter().all(Option::is_none));
        let a = out[2].unwrap();
        near(a.mid, 11.0);
        near(a.upper, 16.0);
        near(a.lower, 6.0);
        let b = out[3].unwrap();
        near(b.mid, 12.0 + 1.0 / 3.0);
        near(b.upper, 12.0 + 1.0 / 3.0 + 5.5);
        near(b.lower, 12.0 + 1.0 / 3.0 - 5.5);
        assert_eq!(Keltner::new(5, 2, 1.0).unwrap().warmup(), 5);
        assert_eq!(Keltner::new(2, 5, 1.0).unwrap().warmup(), 6);
    }

    #[allow(clippy::many_single_char_names)]
    fn naive_boll(xs: &[f64], n: usize, k: f64) -> Vec<Option<BollingerOut>> {
        (0..xs.len())
            .map(|i| {
                if i + 1 < n {
                    return None;
                }
                let w = &xs[i + 1 - n..=i];
                let m = w.iter().sum::<f64>() / nf(n);
                let sd = (w.iter().map(|x| (x - m).powi(2)).sum::<f64>() / nf(n)).sqrt();
                let (u, l) = (m + k * sd, m - k * sd);
                Some(BollingerOut {
                    mid: m,
                    upper: u,
                    lower: l,
                    pct_b: if u > l { (xs[i] - l) / (u - l) } else { 0.5 },
                    bandwidth: if m == 0.0 { 0.0 } else { (u - l) / m },
                })
            })
            .collect()
    }

    fn naive_keltner(xs: &[Ohlcv], en: usize, an: usize, m: f64) -> Vec<Option<KeltnerOut>> {
        (0..xs.len())
            .map(|i| {
                if i + 1 < en || i < an {
                    return None;
                }
                let mut e = xs[..en].iter().map(|b| b.c).sum::<f64>() / nf(en);
                for b in &xs[en..=i] {
                    e += (b.c - e) * 2.0 / (nf(en) + 1.0);
                }
                let tr: Vec<f64> = (1..=i)
                    .map(|j| {
                        let (h, l, pc) = (xs[j].h, xs[j].l, xs[j - 1].c);
                        (h - l).max((h - pc).abs()).max((l - pc).abs())
                    })
                    .collect();
                let mut a = tr[..an].iter().sum::<f64>() / nf(an);
                for t in &tr[an..] {
                    a = (a * (nf(an) - 1.0) + t) / nf(an);
                }
                Some(KeltnerOut {
                    mid: e,
                    upper: e + m * a,
                    lower: e - m * a,
                })
            })
            .collect()
    }

    fn bars_from(v: &[(f64, f64, f64)]) -> Vec<Ohlcv> {
        v.iter()
            .enumerate()
            .map(|(i, &(c, up, dn))| bar(i64::try_from(i).unwrap(), c + up, c - dn, c, 1.0))
            .collect()
    }

    proptest! {
        #[test]
        fn bollinger_matches_naive(xs in proptest::collection::vec(1.0f64..500.0, 0..40), n in 1usize..8, k in 0.5f64..3.0) {
            testkit::check_contract(|| Bollinger::new(n, k).unwrap(), &xs, true);
            let got = batch(Bollinger::new(n, k).unwrap(), xs.clone());
            for (g, w) in got.iter().zip(naive_boll(&xs, n, k)) {
                match (g, w) {
                    (Some(g), Some(w)) => {
                        near(g.mid, w.mid); near(g.upper, w.upper); near(g.lower, w.lower);
                        near(g.bandwidth, w.bandwidth);
                        prop_assert!((g.pct_b - w.pct_b).abs() < 1e-6);
                    }
                    (None, None) => {}
                    _ => prop_assert!(false, "warm-up mismatch"),
                }
            }
        }

        #[test]
        fn donchian_matches_naive(v in proptest::collection::vec((10.0f64..200.0, 0.0f64..5.0, 0.0f64..5.0), 0..40), n in 1usize..8) {
            let xs = bars_from(&v);
            testkit::check_contract(|| Donchian::new(n).unwrap(), &xs, true);
            let got = batch(Donchian::new(n).unwrap(), xs.clone());
            for (i, g) in got.iter().enumerate() {
                if i + 1 < n { prop_assert!(g.is_none()); continue; }
                let w = &xs[i + 1 - n..=i];
                let hi = w.iter().map(|b| b.h).fold(f64::MIN, f64::max);
                let lo = w.iter().map(|b| b.l).fold(f64::MAX, f64::min);
                let g = g.unwrap();
                prop_assert!((g.upper - hi).abs() < 1e-12 && (g.lower - lo).abs() < 1e-12);
                near(g.mid, f64::midpoint(hi, lo));
            }
        }

        #[test]
        fn keltner_matches_naive(v in proptest::collection::vec((10.0f64..200.0, 0.0f64..5.0, 0.0f64..5.0), 0..40), en in 1usize..8, an in 1usize..8) {
            let xs = bars_from(&v);
            testkit::check_contract(|| Keltner::new(en, an, 1.5).unwrap(), &xs, true);
            let got = batch(Keltner::new(en, an, 1.5).unwrap(), xs.clone());
            for (g, w) in got.iter().zip(naive_keltner(&xs, en, an, 1.5)) {
                match (g, w) {
                    (Some(g), Some(w)) => { near(g.mid, w.mid); near(g.upper, w.upper); near(g.lower, w.lower); }
                    (None, None) => {}
                    _ => prop_assert!(false, "warm-up mismatch"),
                }
            }
        }
    }
}
