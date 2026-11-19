//! Regime labels and the state returned by the classifier.

use chrono::DateTime;
use chrono_tz::Tz;

/// Trend label. `Mixed` = signals disagree; `Unknown` = not enough bars or missing input. Both mean no trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trend {
    Up,
    Down,
    Range,
    Mixed,
    Unknown,
}

/// Size of recent daily swings against this series' own last year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VolState {
    Low,
    Normal,
    High,
    Unknown,
}

/// Three-valued flag: a missing input is `Unknown`, never a guessed `False`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tri {
    True,
    False,
    Unknown,
}

/// One plain-English line for the Why panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub label: String,
    pub value: String,
    pub meaning: String,
}

impl Fact {
    /// Build a Why-panel line.
    #[must_use]
    pub fn new(label: &str, value: impl Into<String>, meaning: impl Into<String>) -> Self {
        Self {
            label: label.to_owned(),
            value: value.into(),
            meaning: meaning.into(),
        }
    }
}

/// Result of classifying one series at its last closed bar.
#[derive(Debug, Clone, PartialEq)]
pub struct RegimeState {
    pub trend: Trend,
    pub vol: VolState,
    /// Market series only; `Unknown` on a stock series or when inputs are missing.
    pub breakdown: Tri,
    /// IST close time of the last bar used.
    pub as_of: DateTime<Tz>,
    pub ema_mid: Option<f64>,
    pub ema_slow: Option<f64>,
    pub adx: Option<f64>,
    pub atr_pct: Option<f64>,
    pub atr_pct_rank: Option<f64>,
    pub breadth_pct_above_50dma: Option<f64>,
    pub india_vix: Option<f64>,
    pub vix_percentile: Option<f64>,
    pub reasons: Vec<Fact>,
}

impl RegimeState {
    /// State for a series that cannot be judged yet: every label `Unknown`, no values, one reason.
    #[must_use]
    pub fn unknown(as_of: DateTime<Tz>, reason: Fact) -> Self {
        Self {
            trend: Trend::Unknown,
            vol: VolState::Unknown,
            breakdown: Tri::Unknown,
            as_of,
            ema_mid: None,
            ema_slow: None,
            adx: None,
            atr_pct: None,
            atr_pct_rank: None,
            breadth_pct_above_50dma: None,
            india_vix: None,
            vix_percentile: None,
            reasons: vec![reason],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn unknown_state_has_no_labels_values_or_guesses() {
        let t = chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 1, 1, 15, 30, 0)
            .unwrap();
        let s = RegimeState::unknown(t, Fact::new("why", "3 of 265", "too short"));
        assert_eq!(
            (s.trend, s.vol, s.breakdown),
            (Trend::Unknown, VolState::Unknown, Tri::Unknown)
        );
        assert!(s.ema_mid.is_none() && s.adx.is_none() && s.india_vix.is_none());
        assert_eq!((s.as_of, s.reasons.len()), (t, 1));
    }
}
