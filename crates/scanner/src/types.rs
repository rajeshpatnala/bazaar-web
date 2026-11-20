//! `Recommendation` and its parts (the field list is `bz-quant` "Scanner scoring and Recommendation").
//! Rupee amounts are `Decimal`; statistics are `f64`, the score a `u8`. No `Default`: a missing
//! field is an error from [`Recommendation::new`].

use bazaar_regime::RegimeState;
use bazaar_risk::verdict::{GateVerdict, RuleId};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

/// What the user is advised to consider. Long-only equity for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    Buy,
    Sell,
    NoTrade,
}

/// Holding period. Daily-bar swing only today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Horizon {
    Swing,
}

/// The instrument a recommendation is about (from the instrument master).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentRef {
    pub instrument_token: u64,
    pub tradingsymbol: String,
    pub name: String,
}

/// Quantity from `risk::sizing`, never computed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionSize {
    pub qty: u32,
    pub lots: u32,
}

/// Backtest statistics behind a recommendation.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    pub trades: u32,
    pub win_rate: f64,
    pub profit_factor: f64,
    /// Average result per trade in R (risk units).
    pub avg_r: f64,
    /// Worst peak-to-trough fall, percent.
    pub max_drawdown_pct: f64,
    /// Plain-text test period, e.g. "2021-01-01 to 2025-12-31".
    pub period: String,
    /// True when charges and slippage were deducted. Shown in the Why.
    pub costs_applied: bool,
}

impl Evidence {
    /// The neutral shrink prior (profit factor 1.0, expectancy 0, zero trades): claims no edge.
    #[must_use]
    pub const fn neutral_prior() -> Self {
        Self {
            trades: 0,
            win_rate: 0.0,
            profit_factor: 1.0,
            avg_r: 0.0,
            max_drawdown_pct: 0.0,
            period: String::new(),
            costs_applied: false,
        }
    }
}

/// One structured reason; rendered to plain English later (risk in rupees first).
#[derive(Debug, Clone, PartialEq)]
pub enum WhyFactor {
    Risk {
        risk_rupees: Decimal,
        limit_rupees: Decimal,
        qty: u32,
    },
    Reward {
        target: Decimal,
        reward_risk: Decimal,
    },
    Confluence {
        families_agreeing: u32,
        families_total: u32,
        names: Vec<String>,
    },
    RegimeFit {
        market_trend: String,
        stock_trend: String,
        vol: String,
        penalised: bool,
    },
    Evidence {
        trades: u32,
        profit_factor: Option<f64>,
        costs_applied: bool,
        low_evidence: bool,
    },
    Liquidity {
        median_traded_value: Decimal,
    },
    Invalidation {
        stop: Decimal,
        max_hold_bars: u32,
    },
    Ban {
        checked: bool,
        list_date: Option<String>,
    },
    Events {
        checked: bool,
        notes: Vec<String>,
    },
}

/// Something the user must see next to the idea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    EventNearby(String),
    /// The events calendar is empty or a placeholder: dates were not checked.
    EventsNotChecked,
    StaleData(String),
    ConflictingSignals(String),
    LowEvidence,
}

/// Compact view of a `GateVerdict`. It cannot approve anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSummary {
    pub passed: bool,
    pub hard_failures: Vec<RuleId>,
    pub overridable_failures: Vec<RuleId>,
    pub evaluated_at: DateTime<Tz>,
    pub config_hash: String,
}

impl GateSummary {
    /// Summarise a verdict. The verdict carries no clock, so the caller passes `evaluated_at`
    /// (from the injected `Clock`). A failure that is not an overridable breach counts as hard.
    #[must_use]
    pub fn from_verdict(v: &GateVerdict, evaluated_at: DateTime<Tz>) -> Self {
        let mut hard = Vec::new();
        let mut soft = Vec::new();
        for r in v.failures() {
            if !r.is_input_missing() && r.level().is_overridable() {
                soft.push(r.rule());
            } else {
                hard.push(r.rule());
            }
        }
        Self {
            passed: v.passed(),
            hard_failures: hard,
            overridable_failures: soft,
            evaluated_at,
            config_hash: v.config_hash().as_str().to_owned(),
        }
    }
}

/// Why a `Recommendation` could not be built.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecError {
    #[error("entry zone is empty or inverted")]
    BadEntryZone,
    #[error("stop is on the wrong side of the entry zone")]
    BadStop,
    #[error("a recommendation needs at least one target")]
    NoTargets,
    #[error("a target is on the wrong side of the entry zone")]
    BadTarget,
    #[error("risk in rupees must be above zero")]
    NoRisk,
    #[error("reward:risk must be above zero")]
    BadRewardRisk,
    #[error("quantity must be above zero")]
    NoQuantity,
    #[error("at least one agreeing strategy is required")]
    NoStrategies,
    #[error("confidence must be 0 to 100")]
    BadConfidence,
    #[error("the Why panel is empty")]
    NoWhy,
    #[error("data_as_of is after generated_at")]
    DataFromFuture,
    #[error("a recommendation needs Buy or Sell, not NoTrade")]
    NoAction,
}

/// Everything a recommendation needs, in one place. `Recommendation::new` checks it.
#[derive(Debug, Clone)]
pub struct RecommendationParts {
    pub id: u128,
    pub generated_at: DateTime<Tz>,
    pub data_as_of: DateTime<Tz>,
    pub instrument: InstrumentRef,
    pub action: Action,
    pub entry_zone: (Decimal, Decimal),
    pub stop_loss: Decimal,
    pub targets: Vec<Decimal>,
    pub horizon: Horizon,
    pub size: PositionSize,
    pub risk_rupees: Decimal,
    pub reward_risk: Decimal,
    pub agreeing_strategies: Vec<String>,
    pub confidence: u8,
    pub regime: RegimeState,
    /// `None` is "no evidence": zero trades, so `low_evidence`.
    pub backtest: Option<Evidence>,
    pub why: Vec<WhyFactor>,
    pub warnings: Vec<Warning>,
    pub gate: GateSummary,
}

/// One advised idea. Advice, not an order; `confidence` is a score, not a probability.
#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    p: RecommendationParts,
    low_evidence: bool,
}

impl PartialEq for RecommendationParts {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
            && self.generated_at == o.generated_at
            && self.data_as_of == o.data_as_of
            && self.instrument == o.instrument
            && self.action == o.action
            && self.entry_zone == o.entry_zone
            && self.stop_loss == o.stop_loss
            && self.targets == o.targets
            && self.size == o.size
            && self.risk_rupees == o.risk_rupees
            && self.reward_risk == o.reward_risk
            && self.agreeing_strategies == o.agreeing_strategies
            && self.confidence == o.confidence
            && self.regime == o.regime
            && self.backtest == o.backtest
            && self.why == o.why
            && self.warnings == o.warnings
            && self.gate == o.gate
    }
}

impl Recommendation {
    /// Validate and build. `min_trades_evidence` and `low_evidence_cap` come from `[scanner]`:
    /// below the trade count `low_evidence` is set, `Warning::LowEvidence` is added and
    /// `confidence` is capped, so ranking by `confidence` never favours unproven names.
    pub fn new(
        mut p: RecommendationParts,
        min_trades_evidence: u32,
        low_evidence_cap: u8,
    ) -> Result<Self, RecError> {
        let (lo, hi) = p.entry_zone;
        if lo <= Decimal::ZERO || lo > hi {
            return Err(RecError::BadEntryZone);
        }
        let buy = match p.action {
            Action::Buy => true,
            Action::Sell => false,
            Action::NoTrade => return Err(RecError::NoAction),
        };
        let bad_stop = if buy {
            p.stop_loss >= lo
        } else {
            p.stop_loss <= hi
        };
        if bad_stop || p.stop_loss <= Decimal::ZERO {
            return Err(RecError::BadStop);
        }
        if p.targets.is_empty() {
            return Err(RecError::NoTargets);
        }
        if p.targets
            .iter()
            .any(|t| if buy { *t <= hi } else { *t >= lo })
        {
            return Err(RecError::BadTarget);
        }
        if p.risk_rupees <= Decimal::ZERO {
            return Err(RecError::NoRisk);
        }
        if p.reward_risk <= Decimal::ZERO {
            return Err(RecError::BadRewardRisk);
        }
        if p.size.qty == 0 {
            return Err(RecError::NoQuantity);
        }
        if p.agreeing_strategies.is_empty() {
            return Err(RecError::NoStrategies);
        }
        if p.confidence > 100 {
            return Err(RecError::BadConfidence);
        }
        if p.why.is_empty() {
            return Err(RecError::NoWhy);
        }
        if p.data_as_of > p.generated_at {
            return Err(RecError::DataFromFuture);
        }
        let backtest = p.backtest.take().unwrap_or_else(Evidence::neutral_prior);
        let low_evidence = backtest.trades < min_trades_evidence;
        if low_evidence {
            p.confidence = p.confidence.min(low_evidence_cap);
            if !p.warnings.contains(&Warning::LowEvidence) {
                p.warnings.push(Warning::LowEvidence);
            }
        }
        p.backtest = Some(backtest);
        Ok(Self { p, low_evidence })
    }

    #[must_use]
    pub const fn id(&self) -> u128 {
        self.p.id
    }
    #[must_use]
    pub const fn generated_at(&self) -> DateTime<Tz> {
        self.p.generated_at
    }
    #[must_use]
    pub const fn data_as_of(&self) -> DateTime<Tz> {
        self.p.data_as_of
    }
    #[must_use]
    pub const fn instrument(&self) -> &InstrumentRef {
        &self.p.instrument
    }
    #[must_use]
    pub const fn action(&self) -> Action {
        self.p.action
    }
    #[must_use]
    pub const fn entry_zone(&self) -> (Decimal, Decimal) {
        self.p.entry_zone
    }
    #[must_use]
    pub const fn stop_loss(&self) -> Decimal {
        self.p.stop_loss
    }
    #[must_use]
    pub fn targets(&self) -> &[Decimal] {
        &self.p.targets
    }
    #[must_use]
    pub const fn horizon(&self) -> Horizon {
        self.p.horizon
    }
    #[must_use]
    pub const fn size(&self) -> PositionSize {
        self.p.size
    }
    #[must_use]
    pub const fn risk_rupees(&self) -> Decimal {
        self.p.risk_rupees
    }
    #[must_use]
    pub const fn reward_risk(&self) -> Decimal {
        self.p.reward_risk
    }
    #[must_use]
    pub fn agreeing_strategies(&self) -> &[String] {
        &self.p.agreeing_strategies
    }
    /// Score 0-100 (capped when `low_evidence`), not a probability.
    #[must_use]
    pub const fn confidence(&self) -> u8 {
        self.p.confidence
    }
    #[must_use]
    pub const fn regime(&self) -> &RegimeState {
        &self.p.regime
    }
    /// Always present: "no evidence" is the neutral prior with zero trades.
    #[must_use]
    pub fn backtest(&self) -> &Evidence {
        // `new` always fills it; the fallback keeps this panic-free.
        self.p.backtest.as_ref().unwrap_or(&NEUTRAL)
    }
    #[must_use]
    pub const fn low_evidence(&self) -> bool {
        self.low_evidence
    }
    #[must_use]
    pub fn why(&self) -> &[WhyFactor] {
        &self.p.why
    }
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.p.warnings
    }
    #[must_use]
    pub const fn gate(&self) -> &GateSummary {
        &self.p.gate
    }
}

static NEUTRAL: Evidence = Evidence::neutral_prior();

#[cfg(test)]
mod tests;
