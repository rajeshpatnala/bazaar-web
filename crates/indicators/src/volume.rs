//! On-balance volume, volume SMA and volume spike ratio.

use crate::bands::{ParamError, Window, finite_bar, period};
use crate::{Indicator, Ohlcv};

/// On-balance volume: running sum adding V when close rises, subtracting when it falls,
/// unchanged on equal. The first bar yields 0 (warm-up 1). If the running sum would stop being
/// finite the bar is skipped (state unchanged) and `None` is returned.
#[derive(Debug, Clone, Default)]
pub struct Obv {
    prev_close: Option<f64>,
    sum: f64,
}

impl Obv {
    /// New OBV starting at 0.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prev_close: None,
            sum: 0.0,
        }
    }
}

impl Indicator for Obv {
    type Input = Ohlcv;
    type Output = f64;
    fn warmup(&self) -> usize {
        1
    }
    fn update(&mut self, x: Ohlcv) -> Option<f64> {
        if !finite_bar(&x) {
            return None;
        }
        let delta = match self.prev_close {
            Some(pc) if x.c > pc => x.v,
            Some(pc) if x.c < pc => -x.v,
            _ => 0.0,
        };
        self.prev_close = Some(x.c);
        let next = self.sum + delta;
        if !next.is_finite() {
            return None;
        }
        self.sum = next;
        Some(next)
    }
    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Simple moving average of volume. Warm-up N bars.
#[derive(Debug, Clone)]
pub struct VolumeSma {
    w: Window,
}

impl VolumeSma {
    /// Average over `n` bars.
    ///
    /// # Errors
    /// `n == 0`.
    pub fn new(n: usize) -> Result<Self, ParamError> {
        Ok(Self {
            w: Window::new(period(n, "n")?),
        })
    }
}

impl Indicator for VolumeSma {
    type Input = Ohlcv;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.w.n()
    }
    fn update(&mut self, x: Ohlcv) -> Option<f64> {
        if !finite_bar(&x) {
            return None;
        }
        self.w.push(x.v).then(|| self.w.mean())
    }
    fn reset(&mut self) {
        self.w.clear();
    }
}

/// Volume spike ratio `V / SMA(V, N)` where the SMA covers the previous N bars (current bar
/// excluded). Warm-up N+1 bars; `None` while the previous-N average is 0 (no volume).
#[derive(Debug, Clone)]
pub struct VolumeSpike {
    w: Window,
}

impl VolumeSpike {
    /// Ratio against the previous `n` bars.
    ///
    /// # Errors
    /// `n == 0`.
    pub fn new(n: usize) -> Result<Self, ParamError> {
        Ok(Self {
            w: Window::new(period(n, "n")?),
        })
    }
}

impl Indicator for VolumeSpike {
    type Input = Ohlcv;
    type Output = f64;
    fn warmup(&self) -> usize {
        self.w.n() + 1
    }
    fn update(&mut self, x: Ohlcv) -> Option<f64> {
        if !finite_bar(&x) {
            return None;
        }
        let avg = self.w.is_full().then(|| self.w.mean());
        self.w.push(x.v);
        avg.filter(|a| *a > 0.0).map(|a| x.v / a)
    }
    fn reset(&mut self) {
        self.w.clear();
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

    fn cv(v: &[(f64, f64)]) -> Vec<Ohlcv> {
        v.iter()
            .enumerate()
            .map(|(i, &(c, vol))| bar(i64::try_from(i).unwrap(), c, c, c, vol))
            .collect()
    }

    #[test]
    fn rejects_zero_period() {
        assert!(VolumeSma::new(0).is_err());
        assert!(VolumeSpike::new(0).is_err());
    }

    #[test]
    fn rejects_huge_period_and_skips_non_finite() {
        assert!(VolumeSma::new(crate::bands::MAX_PERIOD + 1).is_err());
        assert!(VolumeSpike::new(usize::MAX).is_err());
        let xs = cv(&[(1.0, 2.0), (2.0, f64::NAN), (3.0, 4.0)]);
        assert_eq!(
            batch(Obv::new(), xs.clone()),
            vec![Some(0.0), None, Some(4.0)]
        );
        assert_eq!(
            batch(VolumeSma::new(1).unwrap(), xs.clone()),
            vec![Some(2.0), None, Some(4.0)]
        );
        assert_eq!(
            batch(VolumeSpike::new(1).unwrap(), xs),
            vec![None, None, Some(2.0)]
        );
    }

    #[test]
    fn obv_hand_case() {
        // closes 10,11,10,10,12 with volumes 5,3,4,6,2: 0, +3, -4 -> -1, equal -> -1, +2 -> 1
        let xs = cv(&[
            (10.0, 5.0),
            (11.0, 3.0),
            (10.0, 4.0),
            (10.0, 6.0),
            (12.0, 2.0),
        ]);
        let out = batch(Obv::new(), xs);
        assert_eq!(
            out,
            vec![Some(0.0), Some(3.0), Some(-1.0), Some(-1.0), Some(1.0)]
        );
    }

    #[test]
    fn obv_non_finite_sum_is_skipped() {
        let xs = cv(&[(1.0, 0.0), (2.0, f64::MAX), (3.0, f64::MAX), (4.0, 1.0)]);
        let out = batch(Obv::new(), xs);
        assert_eq!(out[1], Some(f64::MAX));
        assert_eq!(out[2], None);
        assert_eq!(out[3], Some(f64::MAX + 1.0));
    }

    #[test]
    fn volume_sma_hand_case() {
        // N=2 over 4, 6, 10: (4+6)/2 = 5, (6+10)/2 = 8
        let out = batch(
            VolumeSma::new(2).unwrap(),
            cv(&[(1.0, 4.0), (1.0, 6.0), (1.0, 10.0)]),
        );
        assert_eq!(out, vec![None, Some(5.0), Some(8.0)]);
    }

    #[test]
    fn spike_hand_case_and_zero_volume() {
        // N=2: bar2 = 9 / avg(4,6)=5 -> 1.8; bar3 = 5 / avg(6,9)=7.5 -> 2/3 (current excluded)
        let out = batch(
            VolumeSpike::new(2).unwrap(),
            cv(&[(1.0, 4.0), (1.0, 6.0), (1.0, 9.0), (1.0, 5.0)]),
        );
        assert_eq!(out[..2], [None, None]);
        near(out[2].unwrap(), 1.8);
        near(out[3].unwrap(), 2.0 / 3.0);
        let z = batch(
            VolumeSpike::new(2).unwrap(),
            cv(&[(1.0, 0.0), (1.0, 0.0), (1.0, 5.0)]),
        );
        assert_eq!(z[2], None);
    }

    proptest! {
        #[test]
        fn volume_indicators_match_naive(v in proptest::collection::vec((1.0f64..100.0, 0.0f64..1e6), 0..40), n in 1usize..8) {
            let xs = cv(&v);
            testkit::check_contract(Obv::new, &xs, true);
            testkit::check_contract(|| VolumeSma::new(n).unwrap(), &xs, true);
            testkit::check_contract(|| VolumeSpike::new(n).unwrap(), &xs, false);
            let obv = batch(Obv::new(), xs.clone());
            let sma = batch(VolumeSma::new(n).unwrap(), xs.clone());
            let spike = batch(VolumeSpike::new(n).unwrap(), xs.clone());
            let mut acc = 0.0;
            for i in 0..xs.len() {
                if i > 0 {
                    if xs[i].c > xs[i - 1].c { acc += xs[i].v; } else if xs[i].c < xs[i - 1].c { acc -= xs[i].v; }
                }
                near(obv[i].unwrap(), acc);
                if i + 1 >= n {
                    let m = xs[i + 1 - n..=i].iter().map(|b| b.v).sum::<f64>() / nf(n);
                    near(sma[i].unwrap(), m);
                } else { prop_assert!(sma[i].is_none()); }
                if i >= n {
                    let m = xs[i - n..i].iter().map(|b| b.v).sum::<f64>() / nf(n);
                    match spike[i] {
                        Some(s) => near(s, xs[i].v / m),
                        None => prop_assert!(m <= 0.0),
                    }
                } else { prop_assert!(spike[i].is_none()); }
            }
        }
    }
}
