//! `DataView`: a read-only window of closed bars that ends at the decision bar.
//!
//! The only way to get one is from a slice of already-closed candles and an end index, so code that
//! holds a view cannot reach later bars (no look-ahead). Only the engine (backtest, scanner) builds
//! views; strategies and indicators receive them.

use crate::candle::Candle;

/// Closed bars, oldest first, ending at the decision bar. The engine that builds a view must pass
/// bars of one `Interval` in strictly increasing `close_time` order (not checked here; the F8
/// testkit asserts it): a mis-sorted slice would give wrong `lookback` windows.
#[derive(Debug, Clone, Copy)]
pub struct DataView<'a> {
    bars: &'a [Candle],
}

impl<'a> DataView<'a> {
    /// View of `all[..=end]`; `None` when `end` is out of range. Later bars are not reachable.
    #[must_use]
    pub fn ending_at(all: &'a [Candle], end: usize) -> Option<Self> {
        all.get(..=end).map(|bars| Self { bars })
    }

    /// The decision bar (the last closed bar); `None` on an empty view (never empty via `ending_at`).
    #[must_use]
    pub fn last(&self) -> Option<&'a Candle> {
        self.bars.last()
    }

    /// The most recent `n` bars, oldest first (fewer when the view is shorter).
    #[must_use]
    pub fn lookback(&self, n: usize) -> &'a [Candle] {
        self.bars
            .get(self.bars.len().saturating_sub(n)..)
            .unwrap_or(&[])
    }

    /// All bars in the view, oldest first.
    #[must_use]
    pub const fn bars(&self) -> &'a [Candle] {
        self.bars
    }

    /// Number of bars in the view.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bars.len()
    }

    /// True when the view holds no bars.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candle::Interval;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;
    use rust_decimal::Decimal;

    fn bars(n: u32) -> Vec<Candle> {
        (1..=n)
            .map(|d| {
                let t = Kolkata.with_ymd_and_hms(2026, 1, d, 15, 30, 0).unwrap();
                let p = Decimal::from(100 + d);
                Candle::new(t, Interval::Day, [p, p, p, p], 10, true).unwrap()
            })
            .collect()
    }

    #[test]
    fn view_ends_at_the_decision_bar_and_hides_later_bars() {
        let all = bars(5);
        let v = DataView::ending_at(&all, 2).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v.last(), all.get(2));
        assert_eq!(v.lookback(2), &all[1..3]);
        assert_eq!(v.lookback(99).len(), 3);
        assert_eq!(v.bars().len(), 3);
        assert!(!v.is_empty());
    }

    #[test]
    fn end_past_the_data_is_none() {
        assert!(DataView::ending_at(&bars(3), 3).is_none());
        assert!(DataView::ending_at(&[], 0).is_none());
    }
}
