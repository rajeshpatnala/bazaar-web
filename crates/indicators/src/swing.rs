//! Swing high/low (fractal pivots) with an explicit confirmation lag.

use crate::bands::{ParamError, finite_bar, period};
use crate::{Indicator, Ohlcv};
use std::collections::VecDeque;

/// A confirmed pivot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pivot {
    /// Index of the pivot bar among accepted (finite) bars, counted from 0 since the last reset.
    pub bar: usize,
    /// Pivot price (the bar's high or low).
    pub price: f64,
    /// Close time of the pivot bar, Unix milliseconds.
    pub ts_ms: i64,
}

/// Swing output for one bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwingOut {
    /// Swing high confirmed on this bar (its pivot bar is `n` bars earlier).
    pub new_high: Option<Pivot>,
    /// Swing low confirmed on this bar.
    pub new_low: Option<Pivot>,
    /// Most recent confirmed swing high so far.
    pub last_high: Option<Pivot>,
    /// Most recent confirmed swing low so far.
    pub last_low: Option<Pivot>,
}

/// Swing detector: bar t is a swing high when its high is strictly greater than the highs of the
/// `n` bars before and the `n` bars after it (swing low: strictly lower lows). Ties are not
/// pivots. The pivot at bar t needs bar t+n to close, so it is first reported on bar t+n (the
/// confirmation lag is modelled, never back-filled). Warm-up is `2n + 1` bars. Non-finite bars
/// are skipped.
#[derive(Debug, Clone)]
pub struct Swing {
    n: usize,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    times: VecDeque<i64>,
    count: usize,
    last_high: Option<Pivot>,
    last_low: Option<Pivot>,
}

impl Swing {
    /// Pivot strength `n` bars on each side.
    ///
    /// # Errors
    /// `n == 0` or above the maximum period.
    pub fn new(n: usize) -> Result<Self, ParamError> {
        let n = period(n, "n")?;
        Ok(Self {
            n,
            highs: VecDeque::new(),
            lows: VecDeque::new(),
            times: VecDeque::new(),
            count: 0,
            last_high: None,
            last_low: None,
        })
    }
}

impl Indicator for Swing {
    type Input = Ohlcv;
    type Output = SwingOut;
    fn warmup(&self) -> usize {
        2 * self.n + 1
    }
    fn update(&mut self, x: Ohlcv) -> Option<SwingOut> {
        if !finite_bar(&x) {
            return None;
        }
        let w = 2 * self.n + 1;
        self.highs.push_back(x.h);
        self.lows.push_back(x.l);
        self.times.push_back(x.ts_ms);
        if self.highs.len() > w {
            self.highs.pop_front();
            self.lows.pop_front();
            self.times.pop_front();
        }
        self.count += 1;
        if self.highs.len() < w {
            return None;
        }
        // The candidate is the window's centre bar, n bars old.
        let centre = self.n;
        let idx = self.count - 1 - self.n;
        let (ch, cl, ts_ms) = (self.highs[centre], self.lows[centre], self.times[centre]);
        let is_high = self
            .highs
            .iter()
            .enumerate()
            .all(|(i, h)| i == centre || ch > *h);
        let is_low = self
            .lows
            .iter()
            .enumerate()
            .all(|(i, l)| i == centre || cl < *l);
        let new_high = is_high.then_some(Pivot {
            bar: idx,
            price: ch,
            ts_ms,
        });
        let new_low = is_low.then_some(Pivot {
            bar: idx,
            price: cl,
            ts_ms,
        });
        self.last_high = new_high.or(self.last_high);
        self.last_low = new_low.or(self.last_low);
        Some(SwingOut {
            new_high,
            new_low,
            last_high: self.last_high,
            last_low: self.last_low,
        })
    }
    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.times.clear();
        self.count = 0;
        self.last_high = None;
        self.last_low = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bands::fixtures::bar;
    use crate::{batch, testkit};
    use proptest::prelude::*;

    fn micro() -> Vec<Ohlcv> {
        include_str!("../../../fixtures/indicators/swing_micro.csv")
            .lines()
            .skip(1)
            .map(|line| {
                let f: Vec<f64> = line.split(',').map(|s| s.parse().unwrap()).collect();
                let mid = f64::midpoint(f[1], f[2]);
                bar(
                    line.split(',').next().unwrap().parse().unwrap(),
                    f[1],
                    f[2],
                    mid,
                    1.0,
                )
            })
            .collect()
    }

    fn pivot(bar: usize, price: f64) -> Pivot {
        Pivot {
            bar,
            price,
            ts_ms: i64::try_from(bar).unwrap() * 60_000,
        }
    }

    #[test]
    fn rejects_bad_period() {
        assert_eq!(Swing::new(0).unwrap_err(), ParamError::ZeroPeriod("n"));
        assert!(Swing::new(usize::MAX).is_err());
        assert_eq!(Swing::new(2).unwrap().warmup(), 5);
    }

    #[test]
    fn micro_case_confirmation_lag() {
        // n=2. highs 5,6,9,7,6,8,10,7,6; lows 3,4,2,3,4,3,5,4,5 (every low <= high).
        // Bar 2 (high 9 > 5,6,7,6; low 2 < 3,4,3,4) is a pivot of both kinds, but only visible at
        // bar 4. Bar 5 low 3 ties bar 3 low 3 -> not a pivot. Bar 6 high 10 > 6,8,7,6 -> pivot
        // high, visible at bar 8. Bars 0-3 are warm-up.
        let out = batch(Swing::new(2).unwrap(), micro());
        assert!(out[..4].iter().all(Option::is_none));
        let a = out[4].unwrap();
        assert_eq!(a.new_high, Some(pivot(2, 9.0)));
        assert_eq!(a.new_low, Some(pivot(2, 2.0)));
        for o in &out[5..8] {
            let o = o.unwrap();
            assert_eq!((o.new_high, o.new_low), (None, None));
            assert_eq!(o.last_high, Some(pivot(2, 9.0)));
            assert_eq!(o.last_low, Some(pivot(2, 2.0)));
        }
        let b = out[8].unwrap();
        assert_eq!(b.new_high, Some(pivot(6, 10.0)));
        assert_eq!(b.new_low, None);
        assert_eq!(b.last_high, Some(pivot(6, 10.0)));
        assert_eq!(b.last_low, Some(pivot(2, 2.0)));
    }

    #[test]
    fn skips_non_finite_bars() {
        let mut xs = micro();
        xs.insert(
            3,
            Ohlcv {
                h: f64::NAN,
                ..bar(0, 1.0, 1.0, 1.0, 1.0)
            },
        );
        let out = batch(Swing::new(2).unwrap(), xs);
        assert_eq!(out[3], None);
        // the skipped bar is not counted: the pivot is still bar 2, now visible one input later
        assert_eq!(out[5].unwrap().new_high, Some(pivot(2, 9.0)));
    }

    #[test]
    fn lookahead_swing() {
        let xs: Vec<Ohlcv> = (0..40)
            .map(|i| {
                let c = 100.0 + 5.0 * (f64::from(i) / 3.0).sin();
                bar(i64::from(i), c + 1.0, c - 1.0, c, 1.0)
            })
            .collect();
        testkit::check_contract(|| Swing::new(3).unwrap(), &xs, true);
        // A pivot at bar p is first reported exactly on bar p+n, never earlier.
        let full = batch(Swing::new(3).unwrap(), xs);
        for (t, o) in full.iter().enumerate() {
            if let Some(o) = o {
                for p in [o.new_high, o.new_low].into_iter().flatten() {
                    assert_eq!(t, p.bar + 3);
                }
            }
        }
    }

    proptest! {
        #[test]
        fn matches_naive(v in proptest::collection::vec((0u8..6, 0u8..3), 0..40), n in 1usize..4) {
            // Small integer prices so that equal highs/lows (ties) occur often.
            let xs: Vec<Ohlcv> = v.iter().enumerate()
                .map(|(i, &(lo, span))| {
                    let (l, h) = (f64::from(lo), f64::from(lo + span));
                    bar(i64::try_from(i).unwrap(), h, l, f64::midpoint(h, l), 1.0)
                }).collect();
            let got = batch(Swing::new(n).unwrap(), xs.clone());
            let (mut last_h, mut last_l): (Option<Pivot>, Option<Pivot>) = (None, None);
            for (t, g) in got.iter().enumerate() {
                if t < 2 * n { prop_assert!(g.is_none()); continue; }
                let g = g.unwrap();
                let p = t - n;
                let win = &xs[p - n..=t];
                let hi = win.iter().enumerate().all(|(i, b)| i == n || xs[p].h > b.h);
                let lo = win.iter().enumerate().all(|(i, b)| i == n || xs[p].l < b.l);
                let want_h = hi.then(|| Pivot { bar: p, price: xs[p].h, ts_ms: xs[p].ts_ms });
                let want_l = lo.then(|| Pivot { bar: p, price: xs[p].l, ts_ms: xs[p].ts_ms });
                last_h = want_h.or(last_h);
                last_l = want_l.or(last_l);
                prop_assert_eq!(g.new_high, want_h);
                prop_assert_eq!(g.new_low, want_l);
                prop_assert_eq!(g.last_high, last_h);
                prop_assert_eq!(g.last_low, last_l);
            }
        }
    }
}
