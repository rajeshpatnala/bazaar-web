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
