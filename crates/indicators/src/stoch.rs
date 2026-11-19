//! Stochastic oscillator.

use crate::bands::{ParamError, Window, finite_bar, period};
use crate::{Indicator, Ohlcv};

/// Stochastic output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StochOut {
    /// %K (raw when `smooth_k == 1`, otherwise slow %K), 0 to 100.
    pub k: f64,
    /// %D: SMA(`smooth_d`) of %K.
    pub d: f64,
}

/// Stochastic(N, `smooth_k`, `smooth_d`): raw %K = `100 * (C - LL_N) / (HH_N - LL_N)` (50 when the
/// range is 0); %K = SMA(`smooth_k`) of raw; %D = SMA(`smooth_d`) of %K. First `Some` at bar index
/// `N-1 + smooth_k-1 + smooth_d-1`, so warm-up is `N + smooth_k + smooth_d - 2` bars.
/// Raw %K is clamped to [0, 100], so a malformed bar (close outside its own window range) cannot
/// push the oscillator out of range; a bar whose range overflows yields `None`.
#[derive(Debug, Clone)]
pub struct Stochastic {
    hi: Window,
    lo: Window,
    raw: Window,
    k: Window,
}

impl Stochastic {
    /// Lookback `n`, %K smoothing `smooth_k`, %D smoothing `smooth_d` (classic slow: 14, 3, 3).
    ///
    /// # Errors
    /// Any period 0.
    pub fn new(n: usize, smooth_k: usize, smooth_d: usize) -> Result<Self, ParamError> {
        let n = period(n, "n")?;
        Ok(Self {
            hi: Window::new(n),
            lo: Window::new(n),
            raw: Window::new(period(smooth_k, "smooth_k")?),
            k: Window::new(period(smooth_d, "smooth_d")?),
        })
    }
}

impl Indicator for Stochastic {
    type Input = Ohlcv;
    type Output = StochOut;
    fn warmup(&self) -> usize {
        self.hi.n() + self.raw.n() + self.k.n() - 2
    }
    fn update(&mut self, x: Ohlcv) -> Option<StochOut> {
        if !finite_bar(&x) {
            return None;
        }
        let a = self.hi.push(x.h);
        let b = self.lo.push(x.l);
        if !(a && b) {
            return None;
        }
        let (hh, ll) = (self.hi.max(), self.lo.min());
        let range = hh - ll;
        let raw = if range > 0.0 {
            100.0 * (x.c - ll) / range
        } else {
            50.0
        };
        if !raw.is_finite() {
            return None;
        }
        let raw = raw.clamp(0.0, 100.0);
        if !self.raw.push(raw) {
            return None;
        }
        let k = self.raw.mean();
        if !self.k.push(k) {
            return None;
        }
        Some(StochOut {
            k,
            d: self.k.mean(),
        })
    }
    fn reset(&mut self) {
        self.hi.clear();
        self.lo.clear();
        self.raw.clear();
        self.k.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bands::{fixtures::bar, nf};
    use crate::{batch, testkit};
    use proptest::prelude::*;

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-8, "{a} vs {b}");
    }

    #[test]
    fn rejects_zero_periods() {
        assert!(Stochastic::new(0, 1, 1).is_err());
        assert_eq!(
            Stochastic::new(3, 0, 1).unwrap_err(),
            ParamError::ZeroPeriod("smooth_k")
        );
        assert_eq!(
            Stochastic::new(3, 1, 0).unwrap_err(),
            ParamError::ZeroPeriod("smooth_d")
        );
    }

    #[test]
    fn fast_hand_case() {
        // N=3, no smoothing. bar2: HH 12, LL 6, C 9 -> 100*3/6 = 50.
        // bar3: highs 11,12,13 lows 7,6,8 (HH 13, LL 6), C 13 -> 100*7/7 = 100.
        let xs = [
            bar(0, 10.0, 8.0, 9.0, 1.0),
            bar(1, 11.0, 7.0, 8.0, 1.0),
            bar(2, 12.0, 6.0, 9.0, 1.0),
            bar(3, 13.0, 8.0, 13.0, 1.0),
        ];
        let out = batch(Stochastic::new(3, 1, 1).unwrap(), xs);
        assert!(out[0].is_none() && out[1].is_none());
        near(out[2].unwrap().k, 50.0);
        near(out[3].unwrap().k, 100.0);
        near(out[3].unwrap().d, 100.0);
    }

    #[test]
    fn slow_hand_case() {
        // N=2 smooth_k=2 smooth_d=2: warm-up 2+2+2-2 = 4 bars.
        // raw: bar1 (H 10,12; L 8,9; C 12): HH 12, LL 8 -> 100; bar2 (H 12,11; L 9,7; C 8): HH 12 LL 7 -> 20;
        // bar3 (H 11,13; L 7,10; C 13): HH 13 LL 7 -> 100.
        // K: bar2 = (100+20)/2 = 60, bar3 = (20+100)/2 = 60; D at bar3 = 60.
        let xs = [
            bar(0, 10.0, 8.0, 9.0, 1.0),
            bar(1, 12.0, 9.0, 12.0, 1.0),
            bar(2, 11.0, 7.0, 8.0, 1.0),
            bar(3, 13.0, 10.0, 13.0, 1.0),
        ];
        let s = Stochastic::new(2, 2, 2).unwrap();
        assert_eq!(s.warmup(), 4);
        let out = batch(s, xs);
        assert!(out[..3].iter().all(Option::is_none));
        near(out[3].unwrap().k, 60.0);
        near(out[3].unwrap().d, 60.0);
    }

    #[test]
    fn slow_hand_case_k_differs_from_d() {
        // Same bars plus bar4 (H 12, L 9, C 10): window bars 3-4 HH 13, LL 9 -> raw 100*1/4 = 25.
        // K4 = (100+25)/2 = 62.5; D4 = (K3 + K4)/2 = (60+62.5)/2 = 61.25.
        let xs = [
            bar(0, 10.0, 8.0, 9.0, 1.0),
            bar(1, 12.0, 9.0, 12.0, 1.0),
            bar(2, 11.0, 7.0, 8.0, 1.0),
            bar(3, 13.0, 10.0, 13.0, 1.0),
            bar(4, 12.0, 9.0, 10.0, 1.0),
        ];
        let out = batch(Stochastic::new(2, 2, 2).unwrap(), xs);
        near(out[4].unwrap().k, 62.5);
        near(out[4].unwrap().d, 61.25);
    }

    #[test]
    fn malformed_bars_are_clamped_or_skipped() {
        let out = batch(
            Stochastic::new(1, 1, 1).unwrap(),
            [bar(0, 5.0, 4.0, 9.0, 1.0), bar(1, 5.0, 4.0, -3.0, 1.0)],
        );
        assert_eq!(out[0], Some(StochOut { k: 100.0, d: 100.0 }));
        assert_eq!(out[1], Some(StochOut { k: 0.0, d: 0.0 }));
        let huge = bar(0, f64::MAX, -f64::MAX, 0.0, 1.0);
        assert_eq!(batch(Stochastic::new(1, 1, 1).unwrap(), [huge])[0], None);
    }

    #[test]
    fn lookahead_stochastic() {
        let xs: Vec<Ohlcv> = (0..30)
            .map(|i| {
                let c = 100.0 + f64::from(i % 7) * 1.5;
                bar(i64::from(i), c + 1.0, c - 1.0, c, 1.0)
            })
            .collect();
        testkit::check_contract(|| Stochastic::new(5, 3, 3).unwrap(), &xs, true);
    }

    #[test]
    fn rejects_huge_period_and_skips_non_finite() {
        assert!(Stochastic::new(crate::bands::MAX_PERIOD + 1, 1, 1).is_err());
        assert!(Stochastic::new(3, usize::MAX, 1).is_err());
        assert!(Stochastic::new(3, 1, usize::MAX).is_err());
        let bad = Ohlcv {
            c: f64::INFINITY,
            ..bar(0, 5.0, 4.0, 4.5, 1.0)
        };
        let out = batch(
            Stochastic::new(1, 1, 1).unwrap(),
            [bad, bar(1, 5.0, 4.0, 4.5, 1.0)],
        );
        assert_eq!(out[0], None);
        near(out[1].unwrap().k, 50.0);
    }

    #[test]
    fn flat_range_is_fifty() {
        let xs = [bar(0, 5.0, 5.0, 5.0, 0.0); 3];
        let out = batch(Stochastic::new(2, 1, 1).unwrap(), xs);
        assert_eq!(out[2], Some(StochOut { k: 50.0, d: 50.0 }));
    }

    fn naive(xs: &[Ohlcv], n: usize, sk: usize, sd: usize) -> Vec<Option<StochOut>> {
        let raw = |i: usize| {
            let w = &xs[i + 1 - n..=i];
            let hh = w.iter().map(|b| b.h).fold(f64::MIN, f64::max);
            let ll = w.iter().map(|b| b.l).fold(f64::MAX, f64::min);
            if hh > ll {
                100.0 * (xs[i].c - ll) / (hh - ll)
            } else {
                50.0
            }
        };
        let kf = |i: usize| (i + 1 - sk..=i).map(raw).sum::<f64>() / nf(sk);
        (0..xs.len())
            .map(|i| {
                let first = n - 1 + sk - 1 + sd - 1;
                (i >= first).then(|| StochOut {
                    k: kf(i),
                    d: (i + 1 - sd..=i).map(kf).sum::<f64>() / nf(sd),
                })
            })
            .collect()
    }

    proptest! {
        #[test]
        fn matches_naive(
            v in proptest::collection::vec((10.0f64..200.0, 0.0f64..5.0, 0.0f64..5.0), 0..40),
            n in 1usize..7, sk in 1usize..4, sd in 1usize..4,
        ) {
            let xs: Vec<Ohlcv> = v.iter().enumerate()
                .map(|(i, &(c, u, d))| bar(i64::try_from(i).unwrap(), c + u, c - d, c, 1.0)).collect();
            testkit::check_contract(|| Stochastic::new(n, sk, sd).unwrap(), &xs, true);
            let got = batch(Stochastic::new(n, sk, sd).unwrap(), xs.clone());
            for (g, w) in got.iter().zip(naive(&xs, n, sk, sd)) {
                match (g, w) {
                    (Some(g), Some(w)) => { near(g.k, w.k); near(g.d, w.d); prop_assert!((-1e-9..=100.0 + 1e-9).contains(&g.k)); }
                    (None, None) => {}
                    _ => prop_assert!(false, "warm-up mismatch"),
                }
            }
        }
    }
}
