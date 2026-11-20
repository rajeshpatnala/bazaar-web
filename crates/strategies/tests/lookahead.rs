//! AC 4 across all three strategies, built from the shipped config: later bars change nothing, and
//! a regime dated after the decision bar is refused.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use bazaar_core::dataview::DataView;
use bazaar_strategies::{ContextError, OpenPosition};
use common::{CASES, World, candles, on_bar, strategy};

#[test]
fn lookahead_later_bars_change_nothing_for_every_strategy() {
    for c in &CASES {
        let full = candles(c.csv, true);
        let mut s = strategy(c.id);
        let pos = OpenPosition {
            instrument_token: 7,
            entry_price: full[c.k].close(),
            stop: full[c.k].low(),
            opened_at: full[c.k].close_time(),
            bars_held: 2,
        };
        let mut fired = 0;
        for end in 0..full.len() {
            let cut = &full[..=end];
            let w = World::for_case(c, &full[end]);
            let (a, b) = (
                on_bar(s.as_mut(), cut, end, &w),
                on_bar(s.as_mut(), &full, end, &w),
            );
            assert_eq!(a, b, "{} on_bar at bar {end}", c.id);
            fired += usize::from(a.is_some());
            let mut run = |bars: &[bazaar_core::candle::Candle]| {
                s.manage(&DataView::ending_at(bars, end).unwrap(), &w.ctx(), &pos)
            };
            assert_eq!(run(cut), run(&full), "{} manage at bar {end}", c.id);
        }
        assert_eq!(fired, 1, "{} fires on exactly one fixture bar", c.id);
    }
}

#[test]
fn a_regime_dated_after_the_decision_bar_is_refused() {
    for c in &CASES {
        let bars = candles(c.csv, true);
        let view = DataView::ending_at(&bars, c.k).unwrap();
        let mut s = strategy(c.id);
        let mut later = World::for_case(c, &bars[c.k]);
        let next_day = bars[c.k].close_time() + chrono::Duration::days(1);
        (later.market.as_of, later.stock.as_of) = (next_day, next_day);
        assert!(matches!(
            later.ctx().check_closed_bar(&view),
            Err(ContextError::RegimeFromTheFuture(_))
        ));
        assert!(matches!(
            World::for_case(c, &bars[c.k - 1])
                .ctx()
                .check_closed_bar(&view),
            Err(ContextError::RegimeStale(_))
        ));
        assert!(on_bar(s.as_mut(), &bars, c.k, &later).is_none(), "{}", c.id);
        // An earlier regime is refused too.
        let earlier = World::for_case(c, &bars[c.k - 1]);
        assert!(
            on_bar(s.as_mut(), &bars, c.k, &earlier).is_none(),
            "{}",
            c.id
        );
        // Control: the aligned regime fires.
        let aligned = World::for_case(c, &bars[c.k]);
        assert!(
            on_bar(s.as_mut(), &bars, c.k, &aligned).is_some(),
            "{}",
            c.id
        );
    }
}

#[test]
fn an_unadjusted_series_never_fires() {
    for c in &CASES {
        let raw = candles(c.csv, false);
        let w = World::for_case(c, &raw[c.k]);
        let mut s = strategy(c.id);
        assert!((0..raw.len()).all(|i| on_bar(s.as_mut(), &raw, i, &w).is_none()));
    }
}
