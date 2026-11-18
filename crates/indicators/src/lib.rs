//! Pure indicator maths (streaming and batch); no I/O.
//!
//! One streaming implementation per indicator; `batch` is a fold over `update`. Candles come in as
//! `bazaar_core::candle::Candle` (closed bars only, so no look-ahead) and are converted to `f64`
//! through the single fallible adapter in `ohlcv`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod bands;
pub mod ma;
pub mod momentum;
mod ohlcv;
pub mod session;
pub mod stoch;
pub mod swing;
pub mod trend;
pub mod volume;

pub use ohlcv::{AdapterError, Ohlcv};

/// A streaming indicator. Feed one closed bar at a time.
pub trait Indicator {
    /// What one step consumes.
    type Input;
    /// What one step yields once warm.
    type Output;
    /// Bars needed before the first `Some`.
    fn warmup(&self) -> usize;
    /// Consume one bar; `None` during warm-up.
    fn update(&mut self, x: Self::Input) -> Option<Self::Output>;
    /// Forget all state.
    fn reset(&mut self);
}

/// Run `ind` over `xs`; equals calling `update` on each item in order.
pub fn batch<I: Indicator>(
    mut ind: I,
    xs: impl IntoIterator<Item = I::Input>,
) -> Vec<Option<I::Output>> {
    xs.into_iter().map(|x| ind.update(x)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Test indicator: running sum, warm after 2 bars.
    struct Sum2 {
        n: usize,
        sum: f64,
    }
    impl Indicator for Sum2 {
        type Input = f64;
        type Output = f64;
        fn warmup(&self) -> usize {
            2
        }
        fn update(&mut self, x: f64) -> Option<f64> {
            self.n += 1;
            self.sum += x;
            (self.n >= 2).then_some(self.sum)
        }
        fn reset(&mut self) {
            self.n = 0;
            self.sum = 0.0;
        }
    }

    fn fresh() -> Sum2 {
        Sum2 { n: 0, sum: 0.0 }
    }

    #[test]
    fn batch_matches_streaming_hand_case() {
        // 1, 1+2, 1+2+3
        let out = batch(fresh(), [1.0, 2.0, 3.0]);
        assert_eq!(out, vec![None, Some(3.0), Some(6.0)]);
    }

    #[test]
    fn reset_restarts_warmup() {
        let mut s = fresh();
        s.update(1.0);
        s.update(1.0);
        s.reset();
        assert_eq!(s.update(5.0), None);
        assert_eq!(s.warmup(), 2);
    }

    proptest! {
        #[test]
        fn batch_length_and_prefix_stable(
            xs in proptest::collection::vec(-1e6f64..1e6, 0..50),
            extra in -1e6f64..1e6,
        ) {
            let a = batch(fresh(), xs.clone());
            prop_assert_eq!(a.len(), xs.len());
            let mut longer = xs;
            longer.push(extra);
            let b = batch(fresh(), longer);
            prop_assert_eq!(&b[..a.len()], &a[..]);
        }
    }
}
