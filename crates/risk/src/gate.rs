//! The `DecisionGate` core: classify, then run every rule in `RuleId::EVALUATION_ORDER` and
//! return one explainable verdict. Pure: time comes from the injected `Clock` (read once), limits
//! from the baseline, state from the snapshot. Sizing and the `GatedIntent` mint are slice 11b.
//!
//! Fail closed: an unreadable baseline, an unclassifiable order, a non-canonical symbol or an
//! incomplete verdict is a [`GateReject`] and no verdict exists to override.

use crate::baseline::{BaselineError, LimitsBaseline, rule_ctx};
use crate::classify::{Action, Classification, ClassifyError, classify};
use crate::config::{ConfigHash, GateConfig, RuleCtx};
use crate::override_::IntentDigest;
use crate::rules::behaviour::{
    EntryZone, g08_cooloff, g09_stop_widening, g10_entry_zone, g11_averaging_down,
    g17_reentry_lockout, g18_entries_per_day,
};
use crate::rules::exposure::{self, ExposureInputs};
use crate::rules::inputs::{DataInputs, g01, g06, g07, g15, x01, x03};
use crate::rules::loss_limits::{g03, g19};
use crate::rules::portfolio::{g02, g04a, g04b, g14};
use crate::rules::structure::{ModeInputs, OptionKind, StructureInputs, g13, g16, x02};
use crate::snapshot::PortfolioSnapshot;
use crate::types::{OrderIntent, Side, Tri};
use crate::verdict::{GateVerdict, RuleId, RuleResult, VerdictError};
use bazaar_core::clock::Clock;
use bazaar_core::instrument::Instrument;
use bazaar_core::mode::ExecutionMode;
use chrono::{DateTime, Duration};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use std::fmt;

/// A G05 return series whose last date is further than this many calendar days before the
/// snapshot's IST date (or after it) is stale (RISK-52-6). Five covers a long weekend.
pub const MAX_RETURN_LAG_DAYS: i64 = 5;

/// True when the `live-trading` feature is compiled in. Read from the build, never from a caller.
#[must_use]
pub const fn live_trading_compiled() -> bool {
    cfg!(feature = "live-trading")
}

/// Mode evidence from the caller. There is no `live_trading_compiled` field (the gate reads the
/// build) and approval is a digest, so it only counts for the order it was given for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeEvidence {
    /// Digest of the order the user approved (`semi_auto`); `None` = no approval on record.
    pub approval: Option<IntentDigest>,
    pub compliance_fresh: Tri<bool>,
    pub readiness_report: Tri<bool>,
    pub owner_opt_in: Tri<bool>,
    /// The order would go to the live broker.
    pub live: Tri<bool>,
}

/// Everything the rules read besides the intent and the snapshot.
#[derive(Debug, Clone, Copy)]
pub struct GateInputs<'a> {
    pub data: &'a DataInputs,
    /// Instrument-master row; `OptionKind` is built from it only. `None` = unknown (G13 rejects).
    pub instrument: Option<&'a Instrument>,
    /// Payoff engine's whole-order maximum loss for a capped-loss option structure.
    pub defined_max_loss: Option<Decimal>,
    pub mode: &'a ModeEvidence,
    pub entry_zone: Tri<EntryZone>,
    pub live_price: Tri<Decimal>,
    pub exposure: Option<&'a ExposureInputs>,
}

/// Why no verdict was produced. Every variant is a Reject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateReject {
    /// The symbol is not canonical (upper-case A-Z, 0-9, `-`, `&`, `_`; 1 to 32 characters), or
    /// differs from the instrument's trading symbol.
    Symbol(String),
    Classify(ClassifyError),
    Baseline(BaselineError),
    Verdict(VerdictError),
}

impl fmt::Display for GateReject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symbol(s) => write!(
                f,
                "The symbol {s:?} is not a canonical trading symbol, so the order is refused."
            ),
            Self::Classify(e) => write!(f, "The order cannot be classified: {e}."),
            Self::Baseline(e) => write!(f, "{e}."),
            Self::Verdict(e) => write!(f, "The verdict could not be built: {e:?}."),
        }
    }
}

impl std::error::Error for GateReject {}

/// The clock an override must use with an [`Evaluation`]: frozen at the instant the gate read, so
/// `ctx.now` and the override time are one `Clock` call (CR-55-5).
#[derive(Debug, Clone, Copy)]
pub struct EvaluationClock(DateTime<Tz>);

impl Clock for EvaluationClock {
    fn now_ist(&self) -> DateTime<Tz> {
        self.0
    }
}

/// One gate run: the verdict plus what the mint (11b) and the override engine need.
#[derive(Debug)]
pub struct Evaluation {
    verdict: GateVerdict,
    ctx: RuleCtx,
    classification: Classification,
    ref_price: Decimal,
    risk_ref: Decimal,
    limits_digest: ConfigHash,
    clock: EvaluationClock,
}

impl Evaluation {
    /// The verdict, bound to the intent digest, with every rule in order.
    #[must_use]
    pub const fn verdict(&self) -> &GateVerdict {
        &self.verdict
    }
    #[must_use]
    pub const fn ctx(&self) -> &RuleCtx {
        &self.ctx
    }
    #[must_use]
    pub const fn classification(&self) -> &Classification {
        &self.classification
    }
    /// Notional reference price: `max(intent.price, live price)` (G14, G16, G05, G10).
    #[must_use]
    pub const fn ref_price(&self) -> Decimal {
        self.ref_price
    }
    /// Risk reference price, side-aware: `max` for a Buy, `min` for a Sell (a lower short fill
    /// widens the stop distance). Used by G02, G04b and, in 11b, sizing.
    #[must_use]
    pub const fn risk_ref(&self) -> Decimal {
        self.risk_ref
    }
    /// Digest of the `EffectiveLimits` the rules ran under (file combined with the baseline).
    /// Computed from the derived `Debug` text: stable within one build and pinned by a golden
    /// test, so any change to `EffectiveLimits` shows up in review. Not a cross-build format.
    #[must_use]
    pub const fn limits_digest(&self) -> &ConfigHash {
        &self.limits_digest
    }
    /// Clock frozen at `ctx.now`; pass it to `override_::evaluate`.
    #[must_use]
    pub const fn clock(&self) -> &EvaluationClock {
        &self.clock
    }
}

/// The gate. Holds no state of its own: config, baseline and clock are injected.
pub struct Gate<'a> {
    config: &'a GateConfig,
    baseline: &'a dyn LimitsBaseline,
    clock: &'a dyn Clock,
}

impl fmt::Debug for Gate<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gate")
            .field("config_hash", self.config.hash())
            .finish_non_exhaustive()
    }
}

fn canonical(symbol: &str) -> bool {
    !symbol.is_empty()
        && symbol.len() <= 32
        && symbol.bytes().all(|b| {
            b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b'-' | b'&' | b'_')
        })
}

/// G05 series freshness: every supplied series must end within [`MAX_RETURN_LAG_DAYS`] of the
/// snapshot's IST date and not after it. `None` means no staleness finding.
fn stale_returns(inputs: &ExposureInputs, as_of: Tri<DateTime<Tz>>) -> Option<RuleResult> {
    let fail = |why: String| {
        RuleResult::input_missing(
            RuleId::G05,
            "cannot confirm",
            format!("series ends within {MAX_RETURN_LAG_DAYS} days of the snapshot"),
            &format!(
                "Cannot confirm sector and correlation exposure: {why}. When unsure, the app says no. To pass: refresh the return series."
            ),
        )
    };
    let Tri::Known(at) = as_of else {
        return Some(fail("the snapshot time is unknown".into()));
    };
    let today = at.with_timezone(&Kolkata).date_naive();
    let Some(oldest_ok) = today.checked_sub_signed(Duration::days(MAX_RETURN_LAG_DAYS)) else {
        return Some(fail("the snapshot date is out of range".into()));
    };
    for (sym, series) in &inputs.returns {
        let Some(end) = series.iter().map(|p| p.0).max() else {
            continue; // an empty series is rejected by the rule itself
        };
        if end > today || end < oldest_ok {
            return Some(fail(format!(
                "the return series for {sym} ends on {end}, not within {MAX_RETURN_LAG_DAYS} days up to {today}"
            )));
        }
    }
    None
}

impl<'a> Gate<'a> {
    #[must_use]
    pub const fn new(
        config: &'a GateConfig,
        baseline: &'a dyn LimitsBaseline,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            config,
            baseline,
            clock,
        }
    }

    /// The mode in force (from the file only).
    #[must_use]
    pub const fn mode(&self) -> ExecutionMode {
        self.config.mode()
    }

    /// Run every rule for `intent` against `snapshot`. All failures are listed; none is skipped.
    pub fn evaluate(
        &self,
        intent: &OrderIntent,
        snapshot: &PortfolioSnapshot,
        inputs: &GateInputs<'_>,
    ) -> Result<Evaluation, GateReject> {
        let symbols_ok = canonical(&intent.symbol)
            && inputs
                .instrument
                .is_none_or(|i| i.tradingsymbol == intent.symbol);
        if !symbols_ok {
            return Err(GateReject::Symbol(intent.symbol.clone()));
        }
        // The single clock read: ctx.now and the override clock are the same instant.
        let now = self.clock.now_ist();
        let ctx = rule_ctx(self.config, self.baseline, now).map_err(GateReject::Baseline)?;
        let cls = classify(intent, snapshot).map_err(GateReject::Classify)?;
        // A non-canonical held row must not trap exits or stop moves: it rejects entries only.
        let bad_held = snapshot
            .positions
            .known()
            .is_some_and(|v| v.iter().any(|p| !canonical(&p.symbol)));
        if bad_held && cls.entry_qty > 0 {
            return Err(GateReject::Symbol(intent.symbol.clone()));
        }
        // Two reference prices, each computed once: notional (worst = higher) and risk (side-aware).
        let live = inputs.live_price.known().copied();
        let ref_price = live.map_or(intent.price, |l| intent.price.max(l));
        let risk_ref = live.map_or(intent.price, |l| match intent.side {
            Side::Buy => intent.price.max(l),
            Side::Sell => intent.price.min(l),
        });
        let option = inputs.instrument.map_or(Tri::Unknown, |i| {
            OptionKind::from_instrument(i, inputs.defined_max_loss)
        });
        let digest = IntentDigest::of(intent);
        let m = inputs.mode;
        let structure = StructureInputs {
            option,
            mode: ModeInputs {
                user_approval: m
                    .approval
                    .as_ref()
                    .map_or(Tri::Unknown, |d| Tri::Known(*d == digest)),
                compliance_fresh: m.compliance_fresh,
                readiness_report: m.readiness_report,
                owner_opt_in: m.owner_opt_in,
                live: m.live,
                live_trading_compiled: Tri::Known(live_trading_compiled()),
            },
        };
        let d = inputs.data;
        let entry = cls.action == Action::Entry && cls.entry_qty > 0;
        let g05 = match inputs.exposure {
            Some(e) if entry => stale_returns(e, snapshot.as_of),
            _ => None,
        }
        .unwrap_or_else(|| {
            exposure::evaluate(&ctx, intent, &cls, snapshot, inputs.exposure, ref_price)
        });
        let mut results = Vec::with_capacity(RuleId::EVALUATION_ORDER.len());
        for rule in RuleId::EVALUATION_ORDER {
            results.push(match rule {
                RuleId::G16 => g16(&ctx, intent, &cls, snapshot, &structure, ref_price),
                RuleId::G15 => g15(&ctx, &cls, snapshot, d),
                RuleId::G03 => g03(&ctx, &cls, snapshot),
                RuleId::G08 => g08_cooloff(&ctx, &cls, snapshot),
                RuleId::G19 => g19(&ctx, &cls, snapshot),
                RuleId::G18 => g18_entries_per_day(&ctx, &cls, snapshot),
                RuleId::G17 => g17_reentry_lockout(&ctx, intent, &cls, snapshot),
                RuleId::X01 => x01(d),
                RuleId::X02 => x02(intent, &cls),
                RuleId::X03 => x03(&cls, d),
                RuleId::G06 => g06(&ctx, &cls, d),
                RuleId::G07 => g07(&ctx, &cls, d),
                RuleId::G13 => g13(&ctx, intent, &cls, snapshot, &structure),
                // The RAW intent, on every path: exits and stop moves too.
                RuleId::G09 => g09_stop_widening(intent, snapshot),
                RuleId::G10 => g10_entry_zone(&ctx, &cls, inputs.entry_zone, inputs.live_price),
                RuleId::G11 => g11_averaging_down(intent, &cls, snapshot, inputs.live_price),
                RuleId::G02 => g02(&ctx, intent, &cls, snapshot, risk_ref),
                RuleId::G14 => g14(&ctx, intent, &cls, snapshot, ref_price),
                RuleId::G04a => g04a(&ctx, intent, &cls, snapshot),
                RuleId::G04b => g04b(&ctx, intent, &cls, snapshot, risk_ref),
                RuleId::G01 => g01(&ctx, &cls, d),
                RuleId::G05 => g05.clone(),
            });
        }
        let verdict = GateVerdict::new(ctx.config_hash.clone(), &results)
            .map_err(GateReject::Verdict)?
            .bound_to(digest);
        let limits_digest = ConfigHash::of_text(&format!("{:?}", ctx.limits));
        Ok(Evaluation {
            verdict,
            classification: cls,
            ref_price,
            risk_ref,
            limits_digest,
            clock: EvaluationClock(now),
            ctx,
        })
    }
}
