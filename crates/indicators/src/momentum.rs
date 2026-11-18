//! ROC, RSI, MACD over a stream of `f64` closes.
//!
//! Warm-up (`bz-quant` conventions), `warmup()` = index of first `Some` + 1:
//! ROC(N) first `Some` at index N (no seed); RSI(N) at index N (Wilder, seeded with the SMA of the
//! first N changes); MACD(f,s,sig) at index s+sig-2 (EMAs SMA-seeded, signal seeded from MACD values).

use std::collections::VecDeque;

use crate::Indicator;
use crate::ma::{Ema, ParamError, n_f64};

/// Rate of change in percent: `(x_t - x_{t-N}) / x_{t-N} * 100`. First `Some` at index N.
/// Returns `None` when the value N bars ago is zero (undefined), never inf/NaN.
#[derive(Debug, Clone)]
pub struct Roc {
    n: usize,
    buf: VecDeque<f64>,
}

impl Roc {
    /// # Errors
    /// `ZeroPeriod` when `period == 0`.
    pub fn new(period: usize) -> Result<Self, ParamError> {
        if period == 0 {
            return Err(ParamError::ZeroPeriod);
        }
        Ok(Self {
            n: period,
            buf: VecDeque::with_capacity(period + 2),
        })
    }
}

impl Indicator for Roc {
    type Input = f64;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.n + 1
    }
    fn update(&mut self, x: f64) -> Option<f64> {
        self.buf.push_back(x);
        if self.buf.len() > self.n + 1 {
            self.buf.pop_front();
        }
        if self.buf.len() < self.n + 1 {
            return None;
        }
        let old = *self.buf.front()?;
        (old != 0.0)
            .then(|| (x - old) / old * 100.0)
            .filter(|v| v.is_finite())
    }
    fn reset(&mut self) {
        self.buf.clear();
    }
}

/// Wilder RSI. Seed: average gain/loss are the SMA of the first N changes (first `Some` at index
/// N); afterwards `avg = (avg*(N-1) + x)/N`. RSI = 100 - 100/(1+RS); 100 when avg loss is 0 and
/// avg gain > 0; 50 when both are 0 (flat series, documented choice).
#[derive(Debug, Clone)]
pub struct Rsi {
    n: usize,
    prev: Option<f64>,
    changes: usize,
    gain: f64,
    loss: f64,
}

impl Rsi {
    /// # Errors
    /// `ZeroPeriod` when `period == 0`.
    pub fn new(period: usize) -> Result<Self, ParamError> {
        if period == 0 {
            return Err(ParamError::ZeroPeriod);
        }
        Ok(Self {
            n: period,
            prev: None,
            changes: 0,
            gain: 0.0,
            loss: 0.0,
        })
    }

    fn value(&self) -> f64 {
        if self.loss == 0.0 {
            if self.gain == 0.0 { 50.0 } else { 100.0 }
        } else {
            100.0 - 100.0 / (1.0 + self.gain / self.loss)
        }
    }
}

impl Indicator for Rsi {
    type Input = f64;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.n + 1
    }
    fn update(&mut self, x: f64) -> Option<f64> {
        let last = self.prev.replace(x)?;
        let change = x - last;
        let (up, down) = if change > 0.0 {
            (change, 0.0)
        } else {
            (0.0, -change)
        };
        let nf = n_f64(self.n);
        if self.changes < self.n {
            self.changes += 1;
            self.gain += up;
            self.loss += down;
            if self.changes < self.n {
                return None;
            }
            self.gain /= nf;
            self.loss /= nf;
        } else {
            self.gain = (self.gain * (nf - 1.0) + up) / nf;
            self.loss = (self.loss * (nf - 1.0) + down) / nf;
        }
        Some(self.value())
    }
    fn reset(&mut self) {
        self.prev = None;
        self.changes = 0;
        self.gain = 0.0;
        self.loss = 0.0;
    }
}

/// MACD periods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacdParams {
    fast: usize,
    slow: usize,
    signal: usize,
}

impl MacdParams {
    /// # Errors
    /// `ZeroPeriod` if any period is 0; `FastNotBelowSlow` unless `fast < slow`.
    pub const fn new(fast: usize, slow: usize, signal: usize) -> Result<Self, ParamError> {
        if fast == 0 || slow == 0 || signal == 0 {
            return Err(ParamError::ZeroPeriod);
        }
        if fast >= slow {
            return Err(ParamError::FastNotBelowSlow);
        }
        Ok(Self { fast, slow, signal })
    }
}

impl Default for MacdParams {
    /// The conventional 12/26/9.
    fn default() -> Self {
        Self {
            fast: 12,
            slow: 26,
            signal: 9,
        }
    }
}

/// One MACD reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacdValue {
    /// Fast EMA minus slow EMA.
    pub macd: f64,
    /// EMA of the MACD line.
    pub signal: f64,
    /// `macd - signal`.
    pub hist: f64,
}

/// MACD: EMA(fast) - EMA(slow), signal = EMA(signal) of the MACD line (SMA-seeded on MACD
/// values). The line exists from index slow-1; first `Some` at index slow+signal-2.
#[derive(Debug, Clone)]
pub struct Macd {
    p: MacdParams,
    fast: Ema,
    slow: Ema,
    signal: Ema,
}

impl Macd {
    /// Build from params.
    ///
    /// # Errors
    /// Propagates `ZeroPeriod` (cannot occur for params built through `MacdParams::new`).
    pub fn new(p: MacdParams) -> Result<Self, ParamError> {
        Ok(Self {
            p,
            fast: Ema::new(p.fast)?,
            slow: Ema::new(p.slow)?,
            signal: Ema::new(p.signal)?,
        })
    }
}

impl Indicator for Macd {
    type Input = f64;
    type Output = MacdValue;
    fn warmup(&self) -> usize {
        self.p.slow + self.p.signal - 1
    }
    fn update(&mut self, x: f64) -> Option<MacdValue> {
        let f = self.fast.update(x);
        let s = self.slow.update(x);
        let line = f? - s?;
        let signal = self.signal.update(line)?;
        Some(MacdValue {
            macd: line,
            signal,
            hist: line - signal,
        })
    }
    fn reset(&mut self) {
        self.fast.reset();
        self.slow.reset();
        self.signal.reset();
    }
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::{batch, testkit::check_contract};
    use proptest::prelude::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-8 * (1.0 + b.abs())
    }

    fn naive_roc(xs: &[f64], n: usize) -> Vec<Option<f64>> {
        (0..xs.len())
            .map(|i| (i >= n && xs[i - n] != 0.0).then(|| (xs[i] - xs[i - n]) / xs[i - n] * 100.0))
            .collect()
    }

    /// RSI from the definition: gains/losses arrays, Wilder averages computed forward.
    fn naive_rsi(xs: &[f64], n: usize) -> Vec<Option<f64>> {
        let g: Vec<f64> = xs.windows(2).map(|w| (w[1] - w[0]).max(0.0)).collect();
        let l: Vec<f64> = xs.windows(2).map(|w| (w[0] - w[1]).max(0.0)).collect();
        let mut out = vec![None; xs.len()];
        if xs.len() <= n {
            return out;
        }
        let nf = n as f64;
        let mut ag = g[..n].iter().sum::<f64>() / nf;
        let mut al = l[..n].iter().sum::<f64>() / nf;
        for t in n..xs.len() {
            if t > n {
                ag = (ag * (nf - 1.0) + g[t - 1]) / nf;
                al = (al * (nf - 1.0) + l[t - 1]) / nf;
            }
            out[t] = Some(if al == 0.0 && ag == 0.0 {
                50.0
            } else if al == 0.0 {
                100.0
            } else {
                100.0 - 100.0 / (1.0 + ag / al)
            });
        }
        out
    }

    fn naive_ema(xs: &[f64], n: usize) -> Vec<Option<f64>> {
        let a = 2.0 / (n as f64 + 1.0);
        let mut out = vec![None; xs.len()];
        let mut prev = 0.0;
        for i in 0..xs.len() {
            if i + 1 == n {
                prev = xs[..n].iter().sum::<f64>() / n as f64;
                out[i] = Some(prev);
            } else if i + 1 > n {
                prev = a * xs[i] + (1.0 - a) * prev;
                out[i] = Some(prev);
            }
        }
        out
    }

    fn naive_macd(xs: &[f64], f: usize, s: usize, sg: usize) -> Vec<Option<MacdValue>> {
        let ef = naive_ema(xs, f);
        let es = naive_ema(xs, s);
        let line: Vec<f64> = (s - 1..xs.len())
            .map(|i| ef[i].unwrap() - es[i].unwrap())
            .collect();
        let sig = naive_ema(&line, sg);
        let mut out = vec![None; xs.len()];
        for (k, sv) in sig.iter().enumerate() {
            if let Some(sv) = sv {
                out[s - 1 + k] = Some(MacdValue {
                    macd: line[k],
                    signal: *sv,
                    hist: line[k] - sv,
                });
            }
        }
        out
    }

    #[test]
    fn zero_and_order_params_rejected() {
        assert_eq!(Roc::new(0).unwrap_err(), ParamError::ZeroPeriod);
        assert_eq!(Rsi::new(0).unwrap_err(), ParamError::ZeroPeriod);
        assert_eq!(
            MacdParams::new(0, 5, 3).unwrap_err(),
            ParamError::ZeroPeriod
        );
        assert_eq!(
            MacdParams::new(5, 5, 3).unwrap_err(),
            ParamError::FastNotBelowSlow
        );
    }

    #[test]
    fn roc_hand_cases() {
        // N=2: (12-10)/10*100 = 20; (9-11)/11*100 = -18.1818...
        let out = batch(Roc::new(2).unwrap(), [10.0, 11.0, 12.0, 9.0]);
        assert_eq!(out[..2], [None, None]);
        assert!(close(out[2].unwrap(), 20.0));
        assert!(close(out[3].unwrap(), -200.0 / 11.0));
        // zero base is undefined, not inf
        assert_eq!(batch(Roc::new(1).unwrap(), [0.0, 5.0]), vec![None, None]);
    }

    #[test]
    fn rsi_hand_cases() {
        // N=2, closes 10,11,10,12: changes +1,-1,+2.
        // seed avg gain = 0.5, avg loss = 0.5 -> RSI 50 at index 2.
        // index 3: gain = (0.5*1 + 2)/2 = 1.25, loss = (0.5*1 + 0)/2 = 0.25 -> RS 5 -> 100-100/6
        let out = batch(Rsi::new(2).unwrap(), [10.0, 11.0, 10.0, 12.0]);
        assert_eq!(out[..2], [None, None]);
        assert!(close(out[2].unwrap(), 50.0));
        assert!(close(out[3].unwrap(), 100.0 - 100.0 / 6.0));
    }

    #[test]
    fn rsi_extremes() {
        let up = batch(Rsi::new(3).unwrap(), [1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(up[3], Some(100.0));
        let down = batch(Rsi::new(3).unwrap(), [5.0, 4.0, 3.0, 2.0, 1.0]);
        assert_eq!(down[3], Some(0.0));
        let flat = batch(Rsi::new(3).unwrap(), [2.0; 5]);
        assert_eq!(flat[3], Some(50.0));
    }

    #[test]
    fn macd_hand_cases() {
        // 2/3/2 on 1..6: fast a=2/3 (N=2), slow a=1/2 (N=3), signal N=2.
        // idx2: fast = 1.5 + 2/3*(3-1.5) = 2.5; slow seed = 2; line 0.5.
        // idx3: fast = 2.5 + 2/3*1.5 = 3.5; slow = 2 + 0.5*2 = 3; line 0.5.
        // signal seed (0.5+0.5)/2 = 0.5 at idx3; hist 0.
        let m = Macd::new(MacdParams::new(2, 3, 2).unwrap()).unwrap();
        let out = batch(m, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(out[..3], [None, None, None]);
        let v = out[3].unwrap();
        assert!(close(v.macd, 0.5) && close(v.signal, 0.5) && close(v.hist, 0.0));
        // default 12/26/9: first Some at index 33, so warmup 34
        assert_eq!(Macd::new(MacdParams::default()).unwrap().warmup(), 34);
    }

    proptest! {
        #[test]
        fn roc_matches_naive(xs in proptest::collection::vec(-1e6f64..1e6, 0..60), n in 1usize..10) {
            check_contract(|| Roc::new(n).unwrap(), &xs, false);
            let got = batch(Roc::new(n).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_roc(&xs, n)) {
                match (g, w) {
                    (None, None) => {}
                    (Some(a), Some(b)) => prop_assert!(close(*a, b)),
                    _ => prop_assert!(false, "mismatch"),
                }
            }
        }

        #[test]
        fn rsi_matches_naive(xs in proptest::collection::vec(-1e6f64..1e6, 0..80), n in 1usize..10) {
            check_contract(|| Rsi::new(n).unwrap(), &xs, true);
            let got = batch(Rsi::new(n).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_rsi(&xs, n)) {
                match (g, w) {
                    (None, None) => {}
                    (Some(a), Some(b)) => {
                        prop_assert!(close(*a, b));
                        prop_assert!((0.0..=100.0).contains(a));
                    }
                    _ => prop_assert!(false, "mismatch"),
                }
            }
        }

        #[test]
        fn macd_matches_naive(
            xs in proptest::collection::vec(-1e6f64..1e6, 0..80),
            f in 1usize..5, extra in 1usize..6, sg in 1usize..5,
        ) {
            let s = f + extra;
            let p = MacdParams::new(f, s, sg).unwrap();
            check_contract(|| Macd::new(p).unwrap(), &xs, true);
            let got = batch(Macd::new(p).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_macd(&xs, f, s, sg)) {
                match (g, w) {
                    (None, None) => {}
                    (Some(a), Some(b)) => {
                        prop_assert!(close(a.macd, b.macd) && close(a.signal, b.signal) && close(a.hist, b.hist));
                    }
                    _ => prop_assert!(false, "mismatch"),
                }
            }
        }
    }
}
