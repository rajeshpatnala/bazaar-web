//! True range, ATR, ADX with +DI/-DI, Supertrend over `Ohlcv` bars.
//!
//! Warm-up (`bz-quant` conventions), `warmup()` = index of first `Some` + 1:
//! - `Tr`: needs a previous close, first `Some` at index 1.
//! - `Atr(N)`: Wilder, seeded with the SMA of the first N true ranges, first `Some` at index N.
//! - `Adx(N)`: +DI/-DI at index N (Wilder sums seeded with the sum of the first N TR/DM); ADX is
//!   `None` inside the output until index 2N-1 (average of the first N DX values, then Wilder).
//! - `Supertrend(N, m)`: first `Some` at index N (first ATR).
//!
//! Periods are bounded to `1..=MAX_PERIOD`; bars with a non-finite or huge (> 1e15) high, low or
//! close are skipped (`None`, state unchanged). Open and volume are unused here and not validated.
//! A `None` after warm-up therefore means a rejected bar; the next true range then spans the gap.
//! Zero denominators (flat range) give 0 for DI/DX, never NaN/inf.
//! `TrendParamError` is local to this file (the shared `ParamError` lives in `ma.rs`, on another
//! branch); the two are to be merged once the branches land.

use thiserror::Error;

use crate::{Indicator, Ohlcv};

/// Invalid trend-indicator parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TrendParamError {
    /// A period of zero was given.
    #[error("indicator period must be at least 1")]
    ZeroPeriod,
    /// The period exceeds `MAX_PERIOD`.
    #[error("indicator period must be at most {}", MAX_PERIOD)]
    PeriodTooLarge,
    /// The Supertrend multiplier must be finite and positive.
    #[error("multiplier must be finite and greater than zero")]
    BadMultiplier,
}

#[allow(clippy::cast_precision_loss)]
const fn n_f64(n: usize) -> f64 {
    n as f64
}

/// Largest accepted period; bounds memory and loop work.
pub const MAX_PERIOD: usize = 10_000;

/// Largest accepted absolute price; bars beyond it (or non-finite) are ignored.
const MAX_ABS: f64 = 1e15;

/// True when every price field is finite and within `MAX_ABS`. Bad bars are skipped by every
/// indicator here (output `None`, state untouched) so one bad value never poisons the recursion.
fn sane(x: &Ohlcv) -> bool {
    [x.h, x.l, x.c]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_ABS)
}

const fn check(period: usize) -> Result<usize, TrendParamError> {
    if period == 0 {
        Err(TrendParamError::ZeroPeriod)
    } else if period > MAX_PERIOD {
        Err(TrendParamError::PeriodTooLarge)
    } else {
        Ok(period)
    }
}

/// True range: max(h-l, |h-prev close|, |l-prev close|).
fn true_range(bar: &Ohlcv, prev_close: f64) -> f64 {
    (bar.h - bar.l)
        .max((bar.h - prev_close).abs())
        .max((bar.l - prev_close).abs())
}

/// True range per bar. First `Some` at index 1 (needs a previous close).
#[derive(Debug, Clone, Default)]
pub struct Tr {
    prev_close: Option<f64>,
}

impl Tr {
    /// New true-range indicator.
    #[must_use]
    pub const fn new() -> Self {
        Self { prev_close: None }
    }
}

impl Indicator for Tr {
    type Input = Ohlcv;
    type Output = f64;
    fn warmup(&self) -> usize {
        2
    }
    fn update(&mut self, x: Ohlcv) -> Option<f64> {
        if !sane(&x) {
            return None;
        }
        let pc = self.prev_close.replace(x.c)?;
        Some(true_range(&x, pc))
    }
    fn reset(&mut self) {
        self.prev_close = None;
    }
}

/// Average true range, Wilder smoothing seeded with the SMA of the first N true ranges.
/// First `Some` at index N.
#[derive(Debug, Clone)]
pub struct Atr {
    n: usize,
    prev_close: Option<f64>,
    count: usize,
    value: f64,
}

impl Atr {
    /// # Errors
    /// `ZeroPeriod` when `period == 0`; `PeriodTooLarge` when `period > MAX_PERIOD`.
    pub const fn new(period: usize) -> Result<Self, TrendParamError> {
        match check(period) {
            Ok(n) => Ok(Self {
                n,
                prev_close: None,
                count: 0,
                value: 0.0,
            }),
            Err(e) => Err(e),
        }
    }
}

impl Indicator for Atr {
    type Input = Ohlcv;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.n + 1
    }
    fn update(&mut self, x: Ohlcv) -> Option<f64> {
        if !sane(&x) {
            return None;
        }
        let pc = self.prev_close.replace(x.c)?;
        let tr = true_range(&x, pc);
        let nf = n_f64(self.n);
        if self.count < self.n {
            self.count += 1;
            self.value += tr;
            if self.count < self.n {
                return None;
            }
            self.value /= nf;
        } else {
            self.value = (self.value * (nf - 1.0) + tr) / nf;
        }
        Some(self.value)
    }
    fn reset(&mut self) {
        self.prev_close = None;
        self.count = 0;
        self.value = 0.0;
    }
}

/// One ADX reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdxValue {
    /// Plus directional indicator, 0..=100.
    pub plus_di: f64,
    /// Minus directional indicator, 0..=100.
    pub minus_di: f64,
    /// ADX, `None` until index 2N-1.
    pub adx: Option<f64>,
}

/// Wilder ADX with +DI/-DI. Smoothed TR/+DM/-DM are seeded with the sum of the first N values,
/// then `s = s - s/N + x`. DX = 100*|+DI - -DI|/(+DI + -DI) (0 when both are 0); ADX is seeded with
/// the mean of the first N DX, then `(adx*(N-1) + dx)/N`.
#[derive(Debug, Clone)]
pub struct Adx {
    n: usize,
    prev: Option<Ohlcv>,
    changes: usize,
    tr: f64,
    pdm: f64,
    mdm: f64,
    dx_count: usize,
    dx_sum: f64,
    avg: f64,
}

impl Adx {
    /// # Errors
    /// `ZeroPeriod` when `period == 0`; `PeriodTooLarge` when `period > MAX_PERIOD`.
    pub const fn new(period: usize) -> Result<Self, TrendParamError> {
        match check(period) {
            Ok(n) => Ok(Self {
                n,
                prev: None,
                changes: 0,
                tr: 0.0,
                pdm: 0.0,
                mdm: 0.0,
                dx_count: 0,
                dx_sum: 0.0,
                avg: 0.0,
            }),
            Err(e) => Err(e),
        }
    }
}

impl Indicator for Adx {
    type Input = Ohlcv;
    type Output = AdxValue;
    fn warmup(&self) -> usize {
        self.n + 1
    }
    fn update(&mut self, x: Ohlcv) -> Option<AdxValue> {
        if !sane(&x) {
            return None;
        }
        let p = self.prev.replace(x)?;
        let tr = true_range(&x, p.c);
        let up = x.h - p.h;
        let down = p.l - x.l;
        let pdm = if up > down && up > 0.0 { up } else { 0.0 };
        let mdm = if down > up && down > 0.0 { down } else { 0.0 };
        let nf = n_f64(self.n);
        self.changes += 1;
        if self.changes <= self.n {
            self.tr += tr;
            self.pdm += pdm;
            self.mdm += mdm;
            if self.changes < self.n {
                return None;
            }
        } else {
            self.tr += tr - self.tr / nf;
            self.pdm += pdm - self.pdm / nf;
            self.mdm += mdm - self.mdm / nf;
        }
        let (plus_di, minus_di) = if self.tr > 0.0 {
            (100.0 * self.pdm / self.tr, 100.0 * self.mdm / self.tr)
        } else {
            (0.0, 0.0)
        };
        let di_sum = plus_di + minus_di;
        let dx = if di_sum > 0.0 {
            100.0 * (plus_di - minus_di).abs() / di_sum
        } else {
            0.0
        };
        self.dx_count += 1;
        let adx = match self.dx_count.cmp(&self.n) {
            std::cmp::Ordering::Less => {
                self.dx_sum += dx;
                None
            }
            std::cmp::Ordering::Equal => {
                self.dx_sum += dx;
                self.avg = self.dx_sum / nf;
                Some(self.avg)
            }
            std::cmp::Ordering::Greater => {
                self.avg = (self.avg * (nf - 1.0) + dx) / nf;
                Some(self.avg)
            }
        };
        Some(AdxValue {
            plus_di,
            minus_di,
            adx,
        })
    }
    fn reset(&mut self) {
        let n = self.n;
        *self = Self {
            n,
            prev: None,
            changes: 0,
            tr: 0.0,
            pdm: 0.0,
            mdm: 0.0,
            dx_count: 0,
            dx_sum: 0.0,
            avg: 0.0,
        };
    }
}

/// One Supertrend reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SupertrendValue {
    /// The active band: the lower band while `up`, the upper band otherwise.
    pub value: f64,
    /// True while the trend is up (close above the line).
    pub up: bool,
}

/// Supertrend(N, m). Bands are `hl2 +- m*ATR(N)` (Wilder ATR); the final upper band only moves down
/// (or resets after the previous close broke it), the lower only up. Initial direction at the first
/// `Some` (index N): up when the close is at or above hl2 (a tie counts as up; other platforms
/// may start differently, which only affects bars before the first flip), otherwise down. Afterwards: up flips
/// down when close < final lower band; down flips up when close > final upper band.
#[derive(Debug, Clone)]
pub struct Supertrend {
    n: usize,
    mult: f64,
    atr: Atr,
    prev_close: Option<f64>,
    state: Option<(f64, f64, bool)>,
}

impl Supertrend {
    /// # Errors
    /// `ZeroPeriod` when `period == 0`; `PeriodTooLarge` when `period > MAX_PERIOD`; `BadMultiplier` unless `mult` is finite and > 0.
    pub fn new(period: usize, mult: f64) -> Result<Self, TrendParamError> {
        if !(mult.is_finite() && mult > 0.0) {
            return Err(TrendParamError::BadMultiplier);
        }
        Ok(Self {
            n: check(period)?,
            mult,
            atr: Atr::new(period)?,
            prev_close: None,
            state: None,
        })
    }
}

impl Indicator for Supertrend {
    type Input = Ohlcv;
    type Output = SupertrendValue;
    fn warmup(&self) -> usize {
        self.n + 1
    }
    fn update(&mut self, x: Ohlcv) -> Option<SupertrendValue> {
        if !sane(&x) {
            return None;
        }
        let atr = self.atr.update(x);
        let pc = self.prev_close.replace(x.c);
        let atr = atr?;
        let hl2 = f64::midpoint(x.h, x.l);
        let bu = hl2 + self.mult * atr;
        let bl = hl2 - self.mult * atr;
        let (fu, fl, up) = match (self.state, pc) {
            (Some((pfu, pfl, pup)), Some(pc)) => {
                let fu = if bu < pfu || pc > pfu { bu } else { pfu };
                let fl = if bl > pfl || pc < pfl { bl } else { pfl };
                let up = if pup { x.c >= fl } else { x.c > fu };
                (fu, fl, up)
            }
            _ => (bu, bl, x.c >= hl2),
        };
        self.state = Some((fu, fl, up));
        Some(SupertrendValue {
            value: if up { fl } else { fu },
            up,
        })
    }
    fn reset(&mut self) {
        self.atr.reset();
        self.prev_close = None;
        self.state = None;
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::needless_range_loop,
    clippy::many_single_char_names
)]
mod tests {
    use super::*;
    use crate::{batch, testkit::check_contract};
    use proptest::prelude::*;

    fn bar(h: f64, l: f64, c: f64) -> Ohlcv {
        Ohlcv {
            ts_ms: 0,
            o: c,
            h,
            l,
            c,
            v: 1.0,
        }
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-8 * (1.0 + b.abs())
    }

    fn bars_strategy() -> impl Strategy<Value = Vec<Ohlcv>> {
        proptest::collection::vec((1.0f64..1000.0, 0.0f64..50.0, 0.0f64..=1.0), 0..60).prop_map(
            |v| {
                v.into_iter()
                    .map(|(l, r, f)| bar(l + r, l, l + r * f))
                    .collect()
            },
        )
    }

    fn naive_tr(xs: &[Ohlcv]) -> Vec<Option<f64>> {
        (0..xs.len())
            .map(|i| {
                (i > 0).then(|| {
                    let a = xs[i].h - xs[i].l;
                    let b = (xs[i].h - xs[i - 1].c).abs();
                    let c = (xs[i].l - xs[i - 1].c).abs();
                    a.max(b).max(c)
                })
            })
            .collect()
    }

    fn naive_atr(xs: &[Ohlcv], n: usize) -> Vec<Option<f64>> {
        let tr = naive_tr(xs);
        let mut out = vec![None; xs.len()];
        let mut v = 0.0;
        for i in n..xs.len() {
            if i == n {
                v = (1..=n).map(|k| tr[k].unwrap()).sum::<f64>() / n as f64;
            } else {
                v = (v * (n as f64 - 1.0) + tr[i].unwrap()) / n as f64;
            }
            out[i] = Some(v);
        }
        out
    }

    /// Wilder smoothing in the "decay" form `s_t = s_{t-1}*(n-1)/n + x_t`, seeded with the sum.
    fn wilder_sum(raw: &[f64], n: usize) -> Vec<Option<f64>> {
        // raw[k] belongs to bar k+1
        let mut out = vec![None; raw.len()];
        let mut s = 0.0;
        for k in 0..raw.len() {
            if k < n {
                s += raw[k];
            } else {
                s = s * (n as f64 - 1.0) / n as f64 + raw[k];
            }
            if k + 1 >= n {
                out[k] = Some(s);
            }
        }
        out
    }

    fn naive_adx(xs: &[Ohlcv], n: usize) -> Vec<Option<AdxValue>> {
        let m = xs.len().saturating_sub(1);
        let tr: Vec<f64> = (1..xs.len()).map(|i| naive_tr(xs)[i].unwrap()).collect();
        let up = |i: usize| xs[i].h - xs[i - 1].h;
        let dn = |i: usize| xs[i - 1].l - xs[i].l;
        let pdm: Vec<f64> = (1..xs.len())
            .map(|i| {
                if up(i) > dn(i) && up(i) > 0.0 {
                    up(i)
                } else {
                    0.0
                }
            })
            .collect();
        let mdm: Vec<f64> = (1..xs.len())
            .map(|i| {
                if dn(i) > up(i) && dn(i) > 0.0 {
                    dn(i)
                } else {
                    0.0
                }
            })
            .collect();
        let (st, sp, sm) = (wilder_sum(&tr, n), wilder_sum(&pdm, n), wilder_sum(&mdm, n));
        let mut out = vec![None; xs.len()];
        let mut dxs: Vec<f64> = Vec::new();
        let mut adx = 0.0;
        for k in 0..m {
            let (Some(t), Some(p), Some(q)) = (st[k], sp[k], sm[k]) else {
                continue;
            };
            let (pd, md) = if t > 0.0 {
                (100.0 * p / t, 100.0 * q / t)
            } else {
                (0.0, 0.0)
            };
            let dx = if pd + md > 0.0 {
                100.0 * (pd - md).abs() / (pd + md)
            } else {
                0.0
            };
            dxs.push(dx);
            let a = match dxs.len().cmp(&n) {
                std::cmp::Ordering::Equal => {
                    adx = dxs.iter().sum::<f64>() / n as f64;
                    Some(adx)
                }
                std::cmp::Ordering::Greater => {
                    adx = (adx * (n as f64 - 1.0) + dx) / n as f64;
                    Some(adx)
                }
                std::cmp::Ordering::Less => None,
            };
            out[k + 1] = Some(AdxValue {
                plus_di: pd,
                minus_di: md,
                adx: a,
            });
        }
        out
    }

    fn naive_supertrend(xs: &[Ohlcv], n: usize, mult: f64) -> Vec<Option<SupertrendValue>> {
        let atr = naive_atr(xs, n);
        let mut out = vec![None; xs.len()];
        let (mut fu, mut fl, mut up) = (0.0, 0.0, false);
        for i in n..xs.len() {
            let mid = f64::midpoint(xs[i].h, xs[i].l);
            let a = atr[i].unwrap();
            let (bu, bl) = (mid + mult * a, mid - mult * a);
            if i == n {
                fu = bu;
                fl = bl;
                up = xs[i].c >= mid;
            } else {
                let pc = xs[i - 1].c;
                let (pfu, pfl) = (fu, fl);
                fu = if bu < pfu || pc > pfu { bu } else { pfu };
                fl = if bl > pfl || pc < pfl { bl } else { pfl };
                up = if up { xs[i].c >= fl } else { xs[i].c > fu };
            }
            out[i] = Some(SupertrendValue {
                value: if up { fl } else { fu },
                up,
            });
        }
        out
    }

    #[test]
    fn zero_period_and_multiplier_rejected() {
        assert_eq!(Atr::new(0).unwrap_err(), TrendParamError::ZeroPeriod);
        assert_eq!(Adx::new(0).unwrap_err(), TrendParamError::ZeroPeriod);
        assert_eq!(
            Supertrend::new(0, 3.0).unwrap_err(),
            TrendParamError::ZeroPeriod
        );
        for m in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Supertrend::new(3, m).unwrap_err(),
                TrendParamError::BadMultiplier
            );
        }
    }

    #[test]
    fn oversized_period_rejected() {
        assert_eq!(
            Atr::new(MAX_PERIOD + 1).unwrap_err(),
            TrendParamError::PeriodTooLarge
        );
        assert!(Adx::new(MAX_PERIOD).is_ok());
        assert_eq!(
            Supertrend::new(MAX_PERIOD + 1, 2.0).unwrap_err(),
            TrendParamError::PeriodTooLarge
        );
    }

    #[test]
    fn bad_bars_are_skipped_without_poisoning() {
        let good = [
            bar(10.0, 8.0, 9.0),
            bar(12.0, 9.0, 11.0),
            bar(11.0, 8.0, 9.0),
        ];
        let mut a = Atr::new(1).unwrap();
        let mut s = Supertrend::new(1, 2.0).unwrap();
        let mut d = Adx::new(1).unwrap();
        let mut t = Tr::new();
        a.update(good[0]);
        s.update(good[0]);
        d.update(good[0]);
        t.update(good[0]);
        for bad in [
            bar(f64::NAN, 1.0, 1.0),
            bar(2e15, 1.0, 1.0),
            bar(f64::INFINITY, 1.0, 1.0),
        ] {
            assert_eq!(a.update(bad), None);
            assert_eq!(s.update(bad), None);
            assert_eq!(d.update(bad), None);
            assert_eq!(t.update(bad), None);
        }
        assert_eq!(a.update(good[1]), Some(3.0));
        assert!(s.update(good[1]).is_some_and(|v| v.value.is_finite()));
        assert!(d.update(good[1]).is_some());
        assert_eq!(t.update(good[1]), Some(3.0));
    }

    #[test]
    fn tr_and_atr_hand_cases() {
        let xs = [
            bar(10.0, 8.0, 9.0),
            bar(12.0, 9.0, 11.0),
            bar(11.0, 8.0, 9.0),
            bar(13.0, 10.0, 12.0),
        ];
        // TR1 = max(3, |12-9|=3, |9-9|=0) = 3; TR2 = max(3, |11-11|=0, |8-11|=3) = 3;
        // TR3 = max(3, |13-9|=4, |10-9|=1) = 4
        let tr = batch(Tr::new(), xs);
        assert_eq!(tr, vec![None, Some(3.0), Some(3.0), Some(4.0)]);
        // ATR(2): seed (3+3)/2 = 3 at index 2; then (3*1 + 4)/2 = 3.5
        let atr = batch(Atr::new(2).unwrap(), xs);
        assert_eq!(atr, vec![None, None, Some(3.0), Some(3.5)]);
    }

    #[test]
    fn adx_hand_cases() {
        // N=1 so smoothing is the raw value. Bar1: TR 3, +DM = 12-10 = 2 (down = 8-9 < 0), -DM 0
        // -> +DI = 100*2/3, -DI = 0, DX = 100 = ADX.
        // Bar2: TR 3, up = 11-12 < 0, down = 9-8 = 1 -> -DM 1 -> +DI 0, -DI = 100/3, DX 100.
        let xs = [
            bar(10.0, 8.0, 9.0),
            bar(12.0, 9.0, 11.0),
            bar(11.0, 8.0, 9.0),
        ];
        let out = batch(Adx::new(1).unwrap(), xs);
        assert_eq!(out[0], None);
        let a = out[1].unwrap();
        assert!(near(a.plus_di, 200.0 / 3.0) && near(a.minus_di, 0.0));
        assert!(near(a.adx.unwrap(), 100.0));
        let b = out[2].unwrap();
        assert!(near(b.plus_di, 0.0) && near(b.minus_di, 100.0 / 3.0));
        assert!(near(b.adx.unwrap(), 100.0));
        // N=2: DI at index 2, ADX at index 3 (2N-1)
        let out2 = batch(
            Adx::new(2).unwrap(),
            [xs[0], xs[1], xs[2], bar(13.0, 10.0, 12.0)],
        );
        assert!(out2[2].is_some_and(|v| v.adx.is_none()));
        assert!(out2[3].is_some_and(|v| v.adx.is_some()));
    }

    #[test]
    fn adx_n2_hand_values() {
        // Bars (h,l,c): (10,8,9) (12,9,11) (11,8,9) (13,10,12) (14,11,13).
        // Per change: TR = 3, 3, 4, 3; +DM = 2, 0, 2, 1; -DM = 0, 1, 0, 0.
        // idx2 (seed = sums): TR 6, +DM 2, -DM 1 -> +DI 100/3, -DI 50/3, DX = 100*(50/3)/50 = 100/3.
        // idx3 (s = s - s/2 + x): TR 6-3+4 = 7, +DM 2-1+2 = 3, -DM 1-0.5+0 = 0.5
        //   -> +DI 300/7, -DI 50/7, DX = 100*250/350 = 500/7; ADX seed = (100/3 + 500/7)/2 = 1100/21.
        // idx4: TR 7-3.5+3 = 6.5, +DM 3-1.5+1 = 2.5, -DM 0.5-0.25 = 0.25
        //   -> +DI 250/6.5, -DI 25/6.5, DX = 100*225/275 = 900/11;
        //   ADX = (1100/21 + 900/11)/2 = 15500/231.
        let xs = [
            bar(10.0, 8.0, 9.0),
            bar(12.0, 9.0, 11.0),
            bar(11.0, 8.0, 9.0),
            bar(13.0, 10.0, 12.0),
            bar(14.0, 11.0, 13.0),
        ];
        let out = batch(Adx::new(2).unwrap(), xs);
        assert_eq!((out[0], out[1]), (None, None));
        let a = out[2].unwrap();
        assert!(near(a.plus_di, 100.0 / 3.0) && near(a.minus_di, 50.0 / 3.0));
        assert_eq!(a.adx, None);
        let b = out[3].unwrap();
        assert!(near(b.plus_di, 300.0 / 7.0) && near(b.minus_di, 50.0 / 7.0));
        assert!(near(b.adx.unwrap(), 1100.0 / 21.0));
        let c = out[4].unwrap();
        assert!(near(c.plus_di, 250.0 / 6.5) && near(c.minus_di, 25.0 / 6.5));
        assert!(near(c.adx.unwrap(), 15500.0 / 231.0));
    }

    #[test]
    fn supertrend_flips_back_to_up() {
        // N=1, m=1. First four bars as in `supertrend_hand_cases` (down, fu 10.5, fl 7.5 after bar 3).
        // Bar4 (13,10,12), prev close 5: TR max(3,|13-5|=8,|10-5|=5) = 8, hl2 11.5, bands 19.5/3.5.
        // fu: 19.5 < 10.5 no, prev close 5 > 10.5 no -> stays 10.5. fl: prev close 5 < 7.5 -> reset to 3.5.
        // Down and close 12 > fu 10.5 -> flips up, value = fl = 3.5.
        let xs = [
            bar(10.0, 8.0, 9.0),
            bar(12.0, 9.0, 11.0),
            bar(11.0, 8.0, 9.0),
            bar(8.0, 5.0, 5.0),
            bar(13.0, 10.0, 12.0),
        ];
        let out = batch(Supertrend::new(1, 1.0).unwrap(), xs);
        assert_eq!(out[3].map(|v| v.up), Some(false));
        assert_eq!(
            out[4],
            Some(SupertrendValue {
                value: 3.5,
                up: true
            })
        );
    }

    /// Outputs for the good bars of `xs` with `bad` injected after index `at`.
    fn run_with_bad<I>(
        make: impl Fn() -> I,
        xs: &[Ohlcv],
        at: usize,
        bad: Ohlcv,
    ) -> Vec<Option<I::Output>>
    where
        I: Indicator<Input = Ohlcv>,
    {
        let mut ind = make();
        let mut out = Vec::new();
        for (i, x) in xs.iter().enumerate() {
            out.push(ind.update(*x));
            if i == at {
                assert!(ind.update(bad).is_none());
            }
        }
        out
    }

    #[test]
    fn bad_bar_mid_stream_equals_run_without_it() {
        let xs: Vec<Ohlcv> = (0..12)
            .map(|i| {
                let b = 100.0 + f64::from(i) * 1.5 + if i % 3 == 0 { 4.0 } else { 0.0 };
                bar(b + 3.0, b - 2.0, b + f64::from(i % 4) - 1.0)
            })
            .collect();
        let bad = bar(f64::NAN, 1.0, 1.0);
        for at in [4, 7] {
            assert_eq!(
                run_with_bad(Tr::new, &xs, at, bad),
                batch(Tr::new(), xs.iter().copied())
            );
            assert_eq!(
                run_with_bad(|| Atr::new(3).unwrap(), &xs, at, bad),
                batch(Atr::new(3).unwrap(), xs.iter().copied())
            );
            assert_eq!(
                run_with_bad(|| Adx::new(3).unwrap(), &xs, at, bad),
                batch(Adx::new(3).unwrap(), xs.iter().copied())
            );
            assert_eq!(
                run_with_bad(|| Supertrend::new(3, 2.0).unwrap(), &xs, at, bad),
                batch(Supertrend::new(3, 2.0).unwrap(), xs.iter().copied())
            );
        }
    }

    /// Outputs at indices `< cut` must not depend on any bar at or after `cut`.
    fn assert_no_lookahead<I>(
        make: impl Fn() -> I,
        head: &[Ohlcv],
        tail_a: &[Ohlcv],
        tail_b: &[Ohlcv],
    ) -> Result<(), TestCaseError>
    where
        I: Indicator<Input = Ohlcv>,
        I::Output: PartialEq + std::fmt::Debug,
    {
        let run = |tail: &[Ohlcv]| batch(make(), head.iter().chain(tail).copied());
        let (a, b) = (run(tail_a), run(tail_b));
        prop_assert_eq!(&a[..head.len()], &b[..head.len()]);
        Ok(())
    }

    #[test]
    fn flat_bars_give_zero_not_nan() {
        let xs = [bar(5.0, 5.0, 5.0); 6];
        let out = batch(Adx::new(2).unwrap(), xs);
        let v = out[5].unwrap();
        assert_eq!((v.plus_di, v.minus_di, v.adx), (0.0, 0.0, Some(0.0)));
    }

    #[test]
    fn supertrend_hand_cases() {
        // N=1, m=1: ATR = TR. Bar1: TR 3, hl2 10.5, bands 13.5/7.5, close 11 >= 10.5 -> up, value 7.5.
        // Bar2 (11,8,9): TR 3, hl2 9.5, bands 12.5/6.5; fu = 12.5 (< 13.5), fl stays 7.5 (6.5 < 7.5,
        // prev close 11 >= 7.5); close 9 >= 7.5 -> still up, value 7.5.
        // Bar3 (8,5,5): TR max(3,|8-9|,|5-9|=4) = 4, hl2 6.5, bands 10.5/2.5; fl stays 7.5;
        // close 5 < 7.5 -> down, fu = 10.5 (< 12.5), value 10.5.
        let xs = [
            bar(10.0, 8.0, 9.0),
            bar(12.0, 9.0, 11.0),
            bar(11.0, 8.0, 9.0),
            bar(8.0, 5.0, 5.0),
        ];
        let out = batch(Supertrend::new(1, 1.0).unwrap(), xs);
        assert_eq!(out[0], None);
        assert_eq!(
            out[1],
            Some(SupertrendValue {
                value: 7.5,
                up: true
            })
        );
        assert_eq!(
            out[2],
            Some(SupertrendValue {
                value: 7.5,
                up: true
            })
        );
        assert_eq!(
            out[3],
            Some(SupertrendValue {
                value: 10.5,
                up: false
            })
        );
    }

    proptest! {
        #[test]
        fn lookahead_tr(xs in bars_strategy(), a in bars_strategy(), b in bars_strategy(), cut in 0usize..60) {
            let cut = cut.min(xs.len());
            assert_no_lookahead(Tr::new, &xs[..cut], &a, &b)?;
        }

        #[test]
        fn lookahead_atr(xs in bars_strategy(), a in bars_strategy(), b in bars_strategy(), cut in 0usize..60, n in 1usize..8) {
            let cut = cut.min(xs.len());
            assert_no_lookahead(|| Atr::new(n).unwrap(), &xs[..cut], &a, &b)?;
        }

        #[test]
        fn lookahead_adx(xs in bars_strategy(), a in bars_strategy(), b in bars_strategy(), cut in 0usize..60, n in 1usize..8) {
            let cut = cut.min(xs.len());
            assert_no_lookahead(|| Adx::new(n).unwrap(), &xs[..cut], &a, &b)?;
        }

        #[test]
        fn lookahead_supertrend(xs in bars_strategy(), a in bars_strategy(), b in bars_strategy(), cut in 0usize..60, n in 1usize..8, m in 0.5f64..4.0) {
            let cut = cut.min(xs.len());
            assert_no_lookahead(|| Supertrend::new(n, m).unwrap(), &xs[..cut], &a, &b)?;
        }

        #[test]
        fn tr_matches_naive(xs in bars_strategy()) {
            check_contract(Tr::new, &xs, true);
            let got = batch(Tr::new(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_tr(&xs)) {
                prop_assert_eq!(g.is_some(), w.is_some());
                if let (Some(a), Some(b)) = (g, w) { prop_assert!(near(*a, b)); }
            }
        }

        #[test]
        fn atr_matches_naive(xs in bars_strategy(), n in 1usize..8) {
            check_contract(|| Atr::new(n).unwrap(), &xs, true);
            let got = batch(Atr::new(n).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_atr(&xs, n)) {
                prop_assert_eq!(g.is_some(), w.is_some());
                if let (Some(a), Some(b)) = (g, w) { prop_assert!(near(*a, b)); }
            }
        }

        #[test]
        fn adx_matches_naive(xs in bars_strategy(), n in 1usize..8) {
            check_contract(|| Adx::new(n).unwrap(), &xs, true);
            let got = batch(Adx::new(n).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_adx(&xs, n)) {
                prop_assert_eq!(g.is_some(), w.is_some());
                if let (Some(a), Some(b)) = (g, w) {
                    prop_assert!(near(a.plus_di, b.plus_di) && near(a.minus_di, b.minus_di));
                    prop_assert_eq!(a.adx.is_some(), b.adx.is_some());
                    if let (Some(x), Some(y)) = (a.adx, b.adx) { prop_assert!(near(x, y)); }
                    prop_assert!((0.0..=100.0 + 1e-9).contains(&a.plus_di));
                    prop_assert!(a.adx.is_none_or(|v| (0.0..=100.0 + 1e-9).contains(&v)));
                }
            }
        }

        #[test]
        fn supertrend_matches_naive(xs in bars_strategy(), n in 1usize..8, m in 0.5f64..4.0) {
            check_contract(|| Supertrend::new(n, m).unwrap(), &xs, true);
            let got = batch(Supertrend::new(n, m).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_supertrend(&xs, n, m)) {
                prop_assert_eq!(g.is_some(), w.is_some());
                if let (Some(a), Some(b)) = (g, w) {
                    prop_assert_eq!(a.up, b.up);
                    prop_assert!(near(a.value, b.value));
                }
            }
        }
    }
}
