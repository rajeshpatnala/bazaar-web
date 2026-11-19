//! SMA, EMA, WMA over a stream of `f64` values (normally closes).
//!
//! Warm-up (`bz-quant` conventions): the first `Some` is at 0-based bar index N-1, so `warmup()` is N.
//! EMA is seeded with the SMA of the first N values (`TA-Lib` / `StockCharts`).
//!
//! Input contract: a non-finite input (NaN, inf) is ignored: `update` returns `None` and the state is
//! untouched, so one bad value never poisons the series. A finite input whose arithmetic would overflow
//! (about 1e308) is treated the same way. Periods are capped at [`MAX_PERIOD`].

use std::collections::VecDeque;

use thiserror::Error;

use crate::Indicator;

/// Invalid indicator parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ParamError {
    /// A period of zero was given.
    #[error("indicator period must be at least 1")]
    ZeroPeriod,
    /// MACD needs fast < slow.
    #[error("fast period must be smaller than slow period")]
    FastNotBelowSlow,
    /// A period above [`MAX_PERIOD`] was given.
    #[error("indicator period must be at most 10000")]
    PeriodTooLarge,
}

/// Largest accepted indicator period (guards against huge allocations from bad config).
pub const MAX_PERIOD: usize = 10_000;

/// Period as `f64` (periods are tiny; exact).
#[allow(clippy::cast_precision_loss)]
pub(crate) const fn n_f64(n: usize) -> f64 {
    n as f64
}

pub(crate) const fn check(period: usize) -> Result<usize, ParamError> {
    if period == 0 {
        Err(ParamError::ZeroPeriod)
    } else if period > MAX_PERIOD {
        Err(ParamError::PeriodTooLarge)
    } else {
        Ok(period)
    }
}

/// Simple moving average of the last N values. First `Some` at index N-1.
#[derive(Debug, Clone)]
pub struct Sma {
    n: usize,
    buf: VecDeque<f64>,
}

impl Sma {
    /// Build with the given period.
    ///
    /// # Errors
    /// `ZeroPeriod` when `period == 0`; `PeriodTooLarge` above `MAX_PERIOD`.
    pub fn new(period: usize) -> Result<Self, ParamError> {
        let n = check(period)?;
        Ok(Self {
            n,
            buf: VecDeque::with_capacity(n),
        })
    }
}

impl Indicator for Sma {
    type Input = f64;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.n
    }
    fn update(&mut self, x: f64) -> Option<f64> {
        if !x.is_finite() {
            return None;
        }
        self.buf.push_back(x);
        if self.buf.len() > self.n {
            self.buf.pop_front();
        }
        (self.buf.len() == self.n)
            .then(|| self.buf.iter().sum::<f64>() / n_f64(self.n))
            .filter(|v| v.is_finite())
    }
    fn reset(&mut self) {
        self.buf.clear();
    }
}

/// Exponential moving average, alpha = 2/(N+1), seeded with the SMA of the first N values.
/// First `Some` at index N-1.
#[derive(Debug, Clone)]
pub struct Ema {
    n: usize,
    alpha: f64,
    seen: usize,
    sum: f64,
    prev: f64,
}

impl Ema {
    /// Build with the given period.
    ///
    /// # Errors
    /// `ZeroPeriod` when `period == 0`; `PeriodTooLarge` above `MAX_PERIOD`.
    pub fn new(period: usize) -> Result<Self, ParamError> {
        let n = check(period)?;
        Ok(Self {
            n,
            alpha: 2.0 / (n_f64(n) + 1.0),
            seen: 0,
            sum: 0.0,
            prev: 0.0,
        })
    }
}

impl Indicator for Ema {
    type Input = f64;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.n
    }
    fn update(&mut self, x: f64) -> Option<f64> {
        if !x.is_finite() {
            return None;
        }
        if self.seen < self.n {
            let sum = self.sum + x;
            if !sum.is_finite() {
                return None;
            }
            self.seen += 1;
            self.sum = sum;
            if self.seen < self.n {
                return None;
            }
            self.prev = self.sum / n_f64(self.n);
        } else {
            let next = self.prev + self.alpha * (x - self.prev);
            if !next.is_finite() {
                return None;
            }
            self.prev = next;
        }
        Some(self.prev)
    }
    fn reset(&mut self) {
        self.seen = 0;
        self.sum = 0.0;
        self.prev = 0.0;
    }
}

/// Linear weighted moving average, weights 1..N with the newest heaviest. First `Some` at N-1.
#[derive(Debug, Clone)]
pub struct Wma {
    n: usize,
    buf: VecDeque<f64>,
}

impl Wma {
    /// Build with the given period.
    ///
    /// # Errors
    /// `ZeroPeriod` when `period == 0`; `PeriodTooLarge` above `MAX_PERIOD`.
    pub fn new(period: usize) -> Result<Self, ParamError> {
        let n = check(period)?;
        Ok(Self {
            n,
            buf: VecDeque::with_capacity(n),
        })
    }
}

impl Indicator for Wma {
    type Input = f64;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.n
    }
    fn update(&mut self, x: f64) -> Option<f64> {
        if !x.is_finite() {
            return None;
        }
        self.buf.push_back(x);
        if self.buf.len() > self.n {
            self.buf.pop_front();
        }
        if self.buf.len() < self.n {
            return None;
        }
        let num: f64 = self
            .buf
            .iter()
            .enumerate()
            .map(|(i, v)| n_f64(i + 1) * v)
            .sum();
        Some(num / (n_f64(self.n) * (n_f64(self.n) + 1.0) / 2.0)).filter(|v| v.is_finite())
    }
    fn reset(&mut self) {
        self.buf.clear();
    }
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::{batch, testkit::check_contract};
    use proptest::prelude::*;

    fn close(a: Option<f64>, b: Option<f64>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(x), Some(y)) => (x - y).abs() <= 1e-8 * (1.0 + y.abs()),
            _ => false,
        }
    }

    fn naive_sma(xs: &[f64], n: usize) -> Vec<Option<f64>> {
        (0..xs.len())
            .map(|i| (i + 1 >= n).then(|| xs[i + 1 - n..=i].iter().sum::<f64>() / n as f64))
            .collect()
    }

    fn naive_wma(xs: &[f64], n: usize) -> Vec<Option<f64>> {
        (0..xs.len())
            .map(|i| {
                (i + 1 >= n).then(|| {
                    let w = &xs[i + 1 - n..=i];
                    let num: f64 = (0..n).map(|k| (k + 1) as f64 * w[k]).sum();
                    let den: f64 = (1..=n).map(|k| k as f64).sum();
                    num / den
                })
            })
            .collect()
    }

    /// EMA from the definition: SMA seed at index n-1, then `e_t = a*x_t + (1-a)*e_{t-1}`.
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

    #[test]
    fn zero_period_rejected() {
        assert_eq!(Sma::new(0).unwrap_err(), ParamError::ZeroPeriod);
        assert_eq!(Ema::new(0).unwrap_err(), ParamError::ZeroPeriod);
        assert_eq!(Wma::new(0).unwrap_err(), ParamError::ZeroPeriod);
    }

    #[test]
    fn huge_periods_rejected() {
        for n in [MAX_PERIOD + 1, 1 << 40, usize::MAX] {
            assert_eq!(Sma::new(n).unwrap_err(), ParamError::PeriodTooLarge);
            assert_eq!(Ema::new(n).unwrap_err(), ParamError::PeriodTooLarge);
            assert_eq!(Wma::new(n).unwrap_err(), ParamError::PeriodTooLarge);
        }
        assert!(Sma::new(MAX_PERIOD).is_ok());
    }

    #[test]
    fn non_finite_input_is_skipped_and_output_stays_finite() {
        let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
        let clean = [1.0, 2.0, 3.0, 7.0, 4.0];
        for b in bad {
            let dirty = [1.0, b, 2.0, 3.0, b, 7.0, 4.0];
            let want = batch(Sma::new(3).unwrap(), clean);
            let got: Vec<_> = batch(Sma::new(3).unwrap(), dirty)
                .into_iter()
                .flatten()
                .collect();
            assert_eq!(got, want.into_iter().flatten().collect::<Vec<_>>());
            let want = batch(Ema::new(3).unwrap(), clean);
            let got: Vec<_> = batch(Ema::new(3).unwrap(), dirty)
                .into_iter()
                .flatten()
                .collect();
            assert_eq!(got, want.into_iter().flatten().collect::<Vec<_>>());
            let want = batch(Wma::new(3).unwrap(), clean);
            let got: Vec<_> = batch(Wma::new(3).unwrap(), dirty)
                .into_iter()
                .flatten()
                .collect();
            assert_eq!(got, want.into_iter().flatten().collect::<Vec<_>>());
        }
        // overflow-sized finite values never yield NaN/inf
        let huge = [1e308, 1e308, 1e308, -1e308, 1e308, 1e308];
        for out in batch(Sma::new(2).unwrap(), huge)
            .into_iter()
            .chain(batch(Ema::new(2).unwrap(), huge))
            .chain(batch(Wma::new(2).unwrap(), huge))
            .flatten()
        {
            assert!(out.is_finite());
        }
    }

    #[test]
    fn sma_hand_cases() {
        // (1+2+3)/3 = 2, (2+3+7)/3 = 4
        let out = batch(Sma::new(3).unwrap(), [1.0, 2.0, 3.0, 7.0]);
        assert_eq!(out, vec![None, None, Some(2.0), Some(4.0)]);
        assert_eq!(
            batch(Sma::new(1).unwrap(), [5.0, 6.0]),
            vec![Some(5.0), Some(6.0)]
        );
    }

    #[test]
    fn ema_hand_cases() {
        // N=3, a=0.5: seed (2+4+6)/3 = 4; next 0.5*10 + 0.5*4 = 7; next 0.5*8 + 0.5*7 = 7.5
        let out = batch(Ema::new(3).unwrap(), [2.0, 4.0, 6.0, 10.0, 8.0]);
        assert_eq!(out, vec![None, None, Some(4.0), Some(7.0), Some(7.5)]);
        assert_eq!(
            batch(Ema::new(1).unwrap(), [3.0, 9.0]),
            vec![Some(3.0), Some(9.0)]
        );
    }

    #[test]
    fn wma_hand_cases() {
        // N=3 weights 1,2,3 over 6: (1*1 + 2*2 + 3*3)/6 = 14/6; (1*2 + 2*3 + 3*4)/6 = 20/6
        let out = batch(Wma::new(3).unwrap(), [1.0, 2.0, 3.0, 4.0]);
        assert!(close(out[2], Some(14.0 / 6.0)));
        assert!(close(out[3], Some(20.0 / 6.0)));
        assert_eq!(out[1], None);
    }

    #[test]
    fn warmup_is_period() {
        assert_eq!(Sma::new(5).unwrap().warmup(), 5);
        assert_eq!(Ema::new(5).unwrap().warmup(), 5);
        assert_eq!(Wma::new(5).unwrap().warmup(), 5);
    }

    proptest! {
        #[test]
        fn sma_matches_naive(xs in proptest::collection::vec(-1e6f64..1e6, 0..60), n in 1usize..10) {
            check_contract(|| Sma::new(n).unwrap(), &xs, true);
            let got = batch(Sma::new(n).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_sma(&xs, n)) { prop_assert!(close(*g, w)); }
        }

        #[test]
        fn ema_matches_naive(xs in proptest::collection::vec(-1e6f64..1e6, 0..60), n in 1usize..10) {
            check_contract(|| Ema::new(n).unwrap(), &xs, true);
            let got = batch(Ema::new(n).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_ema(&xs, n)) { prop_assert!(close(*g, w)); }
        }

        #[test]
        fn wma_matches_naive(xs in proptest::collection::vec(-1e6f64..1e6, 0..60), n in 1usize..10) {
            check_contract(|| Wma::new(n).unwrap(), &xs, true);
            let got = batch(Wma::new(n).unwrap(), xs.iter().copied());
            for (g, w) in got.iter().zip(naive_wma(&xs, n)) { prop_assert!(close(*g, w)); }
        }
    }
}
