//! `RegimeFit`: does a strategy's declared regime cells match the market and stock labels, and the
//! plain-English label for a state (spec `docs/strategy/F7-regime-definitions.md` section 5).
//!
//! Unknown, Mixed and a market Breakdown always suppress: "unsure means no trade". A label
//! describes recent behaviour; it does not predict the next bar.

use crate::{Fact, RegimeState, Trend, Tri, VolState};

/// Outcome of a fit check.
#[derive(Debug, Clone, PartialEq)]
pub enum Fit {
    /// The cell is declared and strong: no penalty.
    Allowed,
    /// Declared but weaker (for example Up x High): the scanner lowers the score.
    Penalised(Fact),
    /// No signal: the reason is a plain-English line for the Why panel.
    Suppressed(Fact),
}

/// A strategy's declared regime cells. Held by F8 strategies; filled from their own config.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegimeFit {
    /// Market trends the strategy can work in (Up, Range or Down; never Mixed/Unknown).
    pub market_trends: Vec<Trend>,
    /// Stock (trend, vol) cells the strategy can work in.
    pub stock_cells: Vec<(Trend, VolState)>,
    /// Subset of `stock_cells` that is allowed but weaker.
    pub weak_cells: Vec<(Trend, VolState)>,
}

fn no(label: &str, value: String, meaning: &str) -> Fit {
    Fit::Suppressed(Fact::new(label, value, meaning))
}

impl RegimeFit {
    /// Check the market and stock states against the declared cells. Never panics; any label that is
    /// not settled suppresses.
    #[must_use]
    pub fn allows(&self, market: &RegimeState, stock: &RegimeState) -> Fit {
        if market.breakdown == Tri::True {
            return no("Market", label(market), "Market under stress: no new buys.");
        }
        for (who, s) in [("Market", market), ("Stock", stock)] {
            if matches!(s.trend, Trend::Unknown | Trend::Mixed) || s.vol == VolState::Unknown {
                return no(
                    who,
                    label(s),
                    "Not settled: no trade ideas from this label.",
                );
            }
        }
        if !self.market_trends.contains(&market.trend) {
            return no(
                "Market",
                label(market),
                "This idea does not fit the market's trend.",
            );
        }
        let cell = (stock.trend, stock.vol);
        if !self.stock_cells.contains(&cell) {
            return no(
                "Stock",
                label(stock),
                "This idea does not fit the stock's trend and swings.",
            );
        }
        if self.weak_cells.contains(&cell) {
            return Fit::Penalised(Fact::new(
                "Stock",
                label(stock),
                "Fits, but a weaker setting: the score is lowered.",
            ));
        }
        Fit::Allowed
    }
}

/// Plain-English label for a state (templated; snapshot-tested).
#[must_use]
pub fn label(s: &RegimeState) -> String {
    if s.breakdown == Tri::True {
        return "Market under stress: no new buys".to_owned();
    }
    let word = match s.vol {
        VolState::Low => "calm swings",
        VolState::Normal => "normal swings",
        VolState::High => "wild swings",
        VolState::Unknown => "swings not clear yet",
    };
    match s.trend {
        Trend::Unknown => "Not enough data to judge the market".to_owned(),
        Trend::Mixed => "Unclear: signals disagree".to_owned(),
        Trend::Up => format!("Uptrend, {word}"),
        Trend::Down => format!("Downtrend, {word}"),
        Trend::Range => format!("Sideways, {word}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    fn st(trend: Trend, vol: VolState, breakdown: Tri) -> RegimeState {
        let as_of = Kolkata.with_ymd_and_hms(2026, 1, 1, 15, 30, 0).unwrap();
        let mut s = RegimeState::unknown(as_of, Fact::new("x", "y", "z"));
        s.trend = trend;
        s.vol = vol;
        s.breakdown = breakdown;
        s
    }

    #[test]
    fn labels_for_every_cell() {
        use Trend::{Down, Mixed, Range, Unknown, Up};
        use VolState::{High, Low, Normal};
        let f = Tri::False;
        let cases = [
            (Up, Low, "Uptrend, calm swings"),
            (Up, Normal, "Uptrend, normal swings"),
            (Up, High, "Uptrend, wild swings"),
            (Range, Low, "Sideways, calm swings"),
            (Range, Normal, "Sideways, normal swings"),
            (Range, High, "Sideways, wild swings"),
            (Down, Low, "Downtrend, calm swings"),
            (Down, Normal, "Downtrend, normal swings"),
            (Down, High, "Downtrend, wild swings"),
            (Up, VolState::Unknown, "Uptrend, swings not clear yet"),
            (Mixed, Normal, "Unclear: signals disagree"),
            (
                Unknown,
                VolState::Unknown,
                "Not enough data to judge the market",
            ),
        ];
        for (t, v, want) in cases {
            assert_eq!(label(&st(t, v, f)), want, "{t:?} x {v:?}");
        }
        assert_eq!(
            label(&st(Down, High, Tri::True)),
            "Market under stress: no new buys"
        );
    }

    fn fit() -> RegimeFit {
        RegimeFit {
            market_trends: vec![Trend::Up, Trend::Range],
            stock_cells: vec![(Trend::Up, VolState::Normal), (Trend::Up, VolState::High)],
            weak_cells: vec![(Trend::Up, VolState::High)],
        }
    }

    #[test]
    fn allowed_penalised_and_each_suppression() {
        let m = st(Trend::Up, VolState::Normal, Tri::False);
        let ok = st(Trend::Up, VolState::Normal, Tri::Unknown);
        assert_eq!(fit().allows(&m, &ok), Fit::Allowed);
        let weak = st(Trend::Up, VolState::High, Tri::Unknown);
        assert!(matches!(fit().allows(&m, &weak), Fit::Penalised(_)));
        // Stock cell not declared.
        let other = st(Trend::Down, VolState::Normal, Tri::Unknown);
        assert!(matches!(fit().allows(&m, &other), Fit::Suppressed(_)));
        // Market trend not declared.
        let down = st(Trend::Down, VolState::Normal, Tri::False);
        assert!(matches!(fit().allows(&down, &ok), Fit::Suppressed(_)));
        // Hard suppressions: Unknown, Mixed, unknown vol, market Breakdown.
        for bad in [
            st(Trend::Unknown, VolState::Unknown, Tri::Unknown),
            st(Trend::Mixed, VolState::Normal, Tri::Unknown),
            st(Trend::Up, VolState::Unknown, Tri::Unknown),
        ] {
            assert!(matches!(fit().allows(&m, &bad), Fit::Suppressed(_)));
            assert!(matches!(fit().allows(&bad, &ok), Fit::Suppressed(_)));
        }
        let stress = st(Trend::Up, VolState::Normal, Tri::True);
        let Fit::Suppressed(r) = fit().allows(&stress, &ok) else {
            panic!("breakdown must suppress");
        };
        assert!(r.meaning.contains("no new buys"));
    }

    #[test]
    fn empty_declaration_allows_nothing() {
        let m = st(Trend::Up, VolState::Normal, Tri::False);
        assert!(matches!(
            RegimeFit::default().allows(&m, &m),
            Fit::Suppressed(_)
        ));
    }
}
