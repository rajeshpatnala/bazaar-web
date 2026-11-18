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
#[cfg(test)]
mod testkit;
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

    /// Test indicator: simple moving average over a window (a real windowed indicator).
    struct Sma {
        n: usize,
        buf: Vec<f64>,
    }
    impl Indicator for Sma {
        type Input = f64;
        type Output = f64;
        fn warmup(&self) -> usize {
            self.n
        }
        fn update(&mut self, x: f64) -> Option<f64> {
            self.buf.push(x);
            if self.buf.len() > self.n {
                self.buf.remove(0);
            }
            (self.buf.len() == self.n)
                .then(|| self.buf.iter().sum::<f64>() / f64::from(u32::try_from(self.n).unwrap()))
        }
        fn reset(&mut self) {
            self.buf.clear();
        }
    }

    fn sma(n: usize) -> Sma {
        Sma { n, buf: Vec::new() }
    }

    #[test]
    fn batch_matches_streaming_hand_case() {
        // SMA(2): (1+2)/2 = 1.5, (2+4)/2 = 3
        let out = batch(sma(2), [1.0, 2.0, 4.0]);
        assert_eq!(out, vec![None, Some(1.5), Some(3.0)]);
    }

    #[test]
    fn reset_restarts_warmup() {
        let mut s = sma(2);
        s.update(1.0);
        s.update(1.0);
        s.reset();
        assert_eq!(s.update(5.0), None);
        assert_eq!(s.warmup(), 2);
    }

    proptest! {
        #[test]
        fn windowed_indicator_obeys_contract(
            xs in proptest::collection::vec(-1e6f64..1e6, 0..50),
            n in 1usize..8,
        ) {
            testkit::check_contract(|| sma(n), &xs, true);
        }
    }
}
