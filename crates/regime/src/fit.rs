//! `RegimeFit`: does a strategy's declared regime cells match the market and stock labels, and the
//! plain-English label for a state (spec `docs/strategy/F7-regime-definitions.md` section 5).
//!
//! Unknown, Mixed and a market Breakdown always suppress: "unsure means no trade". A label
//! describes recent behaviour; it does not predict the next bar.

use crate::{Fact, RegimeState, Trend, Tri, VolState};

/// Outcome of a fit check.
#[derive(Debug, Clone, PartialEq)]
pub enum Fit {
    /// The cell is declared and strong: no penalty. The fact is the Why-panel line ("fits").
    Allowed(Fact),
    /// Declared but weaker (for example Up x High): the scanner lowers the score.
    Penalised(Fact),
    /// No signal: the reason is a plain-English line for the Why panel.
    Suppressed(Fact),
}

/// A strategy's declared regime cells. Held by F8 strategies; filled from their own config.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegimeFit {
    /// Market trends the strategy can work in (Up, Range or Down; Mixed/Unknown never match).
    pub market_trends: Vec<Trend>,
    /// Stock (trend, vol) cells the strategy can work in.
    pub stock_cells: Vec<(Trend, VolState)>,
    /// Subset of `stock_cells` that is allowed but weaker. The penalty size is not here: F8 reads
    /// it from `strategies.toml` (no magic number in code).
    pub weak_cells: Vec<(Trend, VolState)>,
}

fn no(label: &str, value: String, meaning: &str) -> Fit {
    Fit::Suppressed(Fact::new(label, value, meaning))
}

/// A bad `RegimeFit` declaration.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegimeFitError {
    #[error("market_trends must not contain Mixed or Unknown")]
    MarketTrendUnsettled,
    #[error("stock_cells must not contain Mixed or Unknown")]
    StockCellUnsettled,
    #[error("weak_cells must also be in stock_cells")]
    WeakNotDeclared,
}

impl RegimeFit {
    /// Check a declaration at config load: `weak_cells` must be declared in `stock_cells`, and no
    /// Mixed/Unknown trend or Unknown vol may be declared. A bad declaration only ever suppresses,
    /// so this is how its author hears about it.
    ///
    /// # Errors
    /// The first problem found.
    pub fn check(&self) -> Result<(), RegimeFitError> {
        let unsettled = |t: Trend| matches!(t, Trend::Unknown | Trend::Mixed);
        if self.market_trends.iter().any(|t| unsettled(*t)) {
            return Err(RegimeFitError::MarketTrendUnsettled);
        }
        if self
            .stock_cells
            .iter()
            .any(|(t, v)| unsettled(*t) || *v == VolState::Unknown)
        {
            return Err(RegimeFitError::StockCellUnsettled);
        }
        if self
            .weak_cells
            .iter()
            .any(|c| !self.stock_cells.contains(c))
        {
            return Err(RegimeFitError::WeakNotDeclared);
        }
        Ok(())
    }

    /// Check the market and stock states against the declared cells. Never panics; any label that is
    /// not settled suppresses.
    #[must_use]
    pub fn allows(&self, market: &RegimeState, stock: &RegimeState) -> Fit {
        for (who, s) in [("Market", market), ("Stock", stock)] {
            if matches!(s.trend, Trend::Unknown | Trend::Mixed) || s.vol == VolState::Unknown {
                return no(
                    who,
                    trend_label(s),
                    "Not settled: no trade ideas from this label.",
                );
            }
        }
        // Fail closed: only a definite `False` passes. For a settled market trend `classify` always
        // gives True or False; `Unknown` means the stress check was not made.
        if market.breakdown != Tri::False {
            let meaning = if market.breakdown == Tri::True {
                "Market under stress: no new buys."
            } else {
                "Market stress check not available: no new buys."
            };
            return no("Market", label(market), meaning);
        }
        if !self.market_trends.contains(&market.trend) {
            return no(
                "Market",
                label(market),
                "This idea does not fit the market's trend.",
            );
        }
        // A stock run normally has `Unknown` breakdown (it is judged for the market only), so only
        // a definite `True` suppresses here.
        if stock.breakdown == Tri::True {
            return no(
                "Stock",
                trend_label(stock),
                "Stock is flagged as crashing: no new buys.",
            );
        }
        let cell = (stock.trend, stock.vol);
        if !self.stock_cells.contains(&cell) {
            return no(
                "Stock",
                trend_label(stock),
                "This idea does not fit the stock's trend and swings.",
            );
        }
        if self.weak_cells.contains(&cell) {
            return Fit::Penalised(Fact::new(
                "Fit",
                format!("Market: {}; Stock: {}", label(market), trend_label(stock)),
                "Fits, but a weaker setting: the score is lowered.",
            ));
        }
        Fit::Allowed(Fact::new(
            "Fit",
            format!("Market: {}; Stock: {}", label(market), trend_label(stock)),
            "Fits this idea.",
        ))
    }
}

/// Plain-English label for a state (templated; snapshot-tested).
#[must_use]
pub fn label(s: &RegimeState) -> String {
    if s.breakdown == Tri::True {
        return "Market under stress: no new buys".to_owned();
    }
    trend_label(s)
}

/// Trend and swing wording only (no stress text), safe for a stock line.
fn trend_label(s: &RegimeState) -> String {
    let word = match s.vol {
        VolState::Low => "calm swings",
        VolState::Normal => "normal swings",
        VolState::High => "wild swings",
        VolState::Unknown => "swings not clear yet",
    };
    match s.trend {
        Trend::Unknown => "Not enough data to judge".to_owned(),
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
            (Range, VolState::Unknown, "Sideways, swings not clear yet"),
            (Down, VolState::Unknown, "Downtrend, swings not clear yet"),
            (Mixed, Normal, "Unclear: signals disagree"),
            (Unknown, VolState::Unknown, "Not enough data to judge"),
        ];
        for (t, v, want) in cases {
            assert_eq!(label(&st(t, v, f)), want, "{t:?} x {v:?}");
        }
        assert_eq!(
            label(&st(Down, High, Tri::True)),
            "Market under stress: no new buys"
        );
    }

    #[test]
    fn stock_labels_use_no_market_wording_and_breakdown_wins_on_any_trend() {
        // A stock with an Unknown trend reads "Not enough data to judge", no "market".
        let s = st(Trend::Unknown, VolState::Unknown, Tri::Unknown);
        assert!(!label(&s).contains("market"));
        for t in [Trend::Up, Trend::Range, Trend::Down] {
            assert_eq!(
                label(&st(t, VolState::High, Tri::True)),
                "Market under stress: no new buys"
            );
        }
    }

    #[test]
    fn unknown_market_breakdown_with_settled_trend_fails_closed() {
        let m = st(Trend::Up, VolState::Normal, Tri::Unknown);
        let ok = st(Trend::Up, VolState::Normal, Tri::Unknown);
        let Fit::Suppressed(r) = fit().allows(&m, &ok) else {
            panic!("unknown breakdown must suppress");
        };
        assert!(r.meaning.contains("not available"));
    }

    #[test]
    fn stock_crash_flag_suppresses_and_pass_names_both_labels() {
        let m = st(Trend::Up, VolState::Normal, Tri::False);
        let mut crash = st(Trend::Up, VolState::Normal, Tri::True);
        let Fit::Suppressed(r) = fit().allows(&m, &crash) else {
            panic!("stock crash must suppress");
        };
        assert!(!r.value.contains("Market") && r.meaning.contains("crashing"));
        crash.breakdown = Tri::Unknown; // the normal stock value
        let Fit::Allowed(f) = fit().allows(&m, &crash) else {
            panic!("allowed");
        };
        assert_eq!(
            f.value,
            "Market: Uptrend, normal swings; Stock: Uptrend, normal swings"
        );
    }

    #[test]
    fn bad_declarations_are_reported() {
        assert_eq!(fit().check(), Ok(()));
        let mut f = fit();
        f.weak_cells.push((Trend::Range, VolState::Low));
        assert_eq!(f.check(), Err(RegimeFitError::WeakNotDeclared));
        let mut f = fit();
        f.market_trends.push(Trend::Mixed);
        assert!(f.check().is_err());
        let mut f = fit();
        f.stock_cells.push((Trend::Up, VolState::Unknown));
        assert!(f.check().is_err());
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
        assert!(matches!(fit().allows(&m, &ok), Fit::Allowed(_)));
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
