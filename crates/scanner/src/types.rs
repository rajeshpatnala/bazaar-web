//! `Recommendation` and its parts (the field list is `bz-quant` "Scanner scoring and Recommendation").
//! Rupee amounts are `Decimal`; statistics are `f64`, the score a `u8`. No `Default`: a missing
//! field is an error from [`Recommendation::new`].

use bazaar_regime::RegimeState;
use bazaar_risk::verdict::{GateVerdict, OverrideLevel, RuleId};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

/// What the user is advised to consider. Delivery equity is long-only today: `Sell` would be an
/// intraday short (MIS), which the gate, sizing and journal do not cover yet, so no scanner
/// step may emit it until short selling is scoped.
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
    /// True only when the stats come from out-of-sample / walk-forward trades. Unknown is
    /// `false` (fail closed): in-sample numbers flatter the edge. Shown in the Why.
    pub out_of_sample: bool,
}

impl Evidence {
    /// Finite numbers in range.
    fn is_sane(&self) -> bool {
        let finite = self.profit_factor.is_finite()
            && self.avg_r.is_finite()
            && self.win_rate.is_finite()
            && self.max_drawdown_pct.is_finite();
        finite
            && (0.0..=1.0).contains(&self.win_rate)
            && self.profit_factor >= 0.0
            && self.max_drawdown_pct >= 0.0
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
        out_of_sample: bool,
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
    /// The subset of `overridable_failures` that can be overridden in paper mode only.
    pub paper_only: Vec<RuleId>,
    /// True only when the one failure is G16 and its sole reason is the missing approval (the
    /// verdict's typed marker, never parsed from text). `passed` stays false and `hard_failures`
    /// still lists G16. The idea is not approved: no order exists until the user approves it.
    pub approval_pending: bool,
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
        let mut paper = Vec::new();
        for r in v.failures() {
            if !r.is_input_missing() && r.level().is_overridable() {
                soft.push(r.rule());
                if r.level() == OverrideLevel::Paper {
                    paper.push(r.rule());
                }
            } else {
                hard.push(r.rule());
            }
        }
        Self {
            passed: v.passed(),
            hard_failures: hard,
            overridable_failures: soft,
            paper_only: paper,
            approval_pending: v.approval_pending_only(),
            evaluated_at,
            config_hash: v.config_hash().as_str().to_owned(),
        }
    }

    /// The pending flag is allowed only on a failed gate whose one failure is G16.
    fn pending_is_consistent(&self) -> bool {
        !self.approval_pending
            || (!self.passed
                && self.hard_failures == [RuleId::G16]
                && self.overridable_failures.is_empty()
                && self.paper_only.is_empty())
    }
}

/// Reward:risk on the worst fill, price only: `|first target - worst entry| / |worst entry - stop|`.
/// `None` when the stop equals the entry or the division overflows. Callers and
/// [`Recommendation::new`] use this one formula, so a stated value can be checked exactly. Callers
/// pass the unrounded value (a 2 dp copy is refused); the UI rounds for display.
#[must_use]
pub fn reward_risk(worst_entry: Decimal, stop: Decimal, first_target: Decimal) -> Option<Decimal> {
    let risk = (worst_entry - stop).abs();
    if risk.is_zero() {
        return None;
    }
    (first_target - worst_entry).abs().checked_div(risk)
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
    #[error("a target is not strictly further from the entry than the one before it")]
    TargetsNotMonotonic,
    #[error("risk in rupees is below quantity x the worst-fill entry-to-stop distance")]
    RiskBelowStopDistance,
    #[error("evidence statistics are not finite or out of range")]
    BadEvidence,
    #[error("gate summary `passed` disagrees with its failure lists")]
    GateInconsistent,
    #[error("risk in rupees must be above zero")]
    NoRisk,
    #[error("reward:risk must be above zero")]
    BadRewardRisk,
    #[error("reward:risk does not match (first target - worst entry) / (worst entry - stop)")]
    RewardRiskMismatch,
    #[error("`approval_pending` needs a failed gate whose only failure is G16")]
    ApprovalPendingInconsistent,
    #[error("paper-only failures are not exactly the Paper-level overridable failures")]
    PaperOnlyNotOverridable,
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
#[derive(Debug, Clone, PartialEq)]
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

impl Recommendation {
    /// Validate and build. `min_trades_evidence` and `low_evidence_cap` come from `[scanner]`:
    /// below the trade count (or without evidence, costs or out-of-sample) `low_evidence` is set, `Warning::LowEvidence` is added and
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
        if !p
            .targets
            .windows(2)
            .all(|w| if buy { w[1] > w[0] } else { w[1] < w[0] })
        {
            return Err(RecError::TargetsNotMonotonic);
        }
        if p.risk_rupees <= Decimal::ZERO {
            return Err(RecError::NoRisk);
        }
        // Worst fill: the far edge of the entry zone from the stop. Charges only push risk higher.
        let distance = if buy {
            hi - p.stop_loss
        } else {
            p.stop_loss - lo
        };
        if p.risk_rupees < Decimal::from(p.size.qty) * distance {
            return Err(RecError::RiskBelowStopDistance);
        }
        if p.reward_risk <= Decimal::ZERO {
            return Err(RecError::BadRewardRisk);
        }
        // Recomputed from the levels on the worst fill; a stated value is never trusted.
        let first = p.targets.first().copied().unwrap_or(hi);
        let far = if buy { hi } else { lo };
        if reward_risk(far, p.stop_loss, first) != Some(p.reward_risk) {
            return Err(RecError::RewardRiskMismatch);
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
        if p.backtest.as_ref().is_some_and(|b| !b.is_sane()) {
            return Err(RecError::BadEvidence);
        }
        let g = &p.gate;
        // Exactly the Paper-level overridable failures: omitting G01 would make it look Soft. The
        // override endpoint re-derives the level from the `RuleId` and never trusts this list.
        let expected: Vec<RuleId> = g
            .overridable_failures
            .iter()
            .copied()
            .filter(|r| r.override_level() == OverrideLevel::Paper)
            .collect();
        let (mut have, mut want) = (g.paper_only.clone(), expected);
        have.sort();
        want.sort();
        if have != want {
            return Err(RecError::PaperOnlyNotOverridable);
        }
        if !g.pending_is_consistent() {
            return Err(RecError::ApprovalPendingInconsistent);
        }
        if g.passed != (g.hard_failures.is_empty() && g.overridable_failures.is_empty()) {
            return Err(RecError::GateInconsistent);
        }
        if p.data_as_of > p.generated_at {
            return Err(RecError::DataFromFuture);
        }
        // Fail closed: no evidence, too few trades, no costs or not out-of-sample is low evidence,
        // whatever `min_trades_evidence` is (a 0 cannot clear it).
        let low_evidence = p
            .backtest
            .as_ref()
            .is_none_or(|b| b.trades < min_trades_evidence || !b.costs_applied || !b.out_of_sample);
        if low_evidence {
            p.confidence = p.confidence.min(low_evidence_cap);
            if !p.warnings.contains(&Warning::LowEvidence) {
                p.warnings.push(Warning::LowEvidence);
            }
        }
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
    /// `None` means no evidence: renderers print "no evidence", never zeros.
    #[must_use]
    pub const fn backtest(&self) -> Option<&Evidence> {
        self.p.backtest.as_ref()
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

#[cfg(test)]
mod tests;
