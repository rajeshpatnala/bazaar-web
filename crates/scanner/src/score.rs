//! Score and rank candidates (F11 step 3). The score is a 0-100 rule-fit number, never a chance
//! of profit. f64 lives only inside this module; money is read once (`Decimal` to f64 for the
//! liquidity ratio) and the result leaves as a `u8`. The ranking sorts on integers only.
//!
//! Score = 100 x (`w_conf` x confluence + `w_regime` x `regime_fit` + `w_liq` x liquidity + `w_evid` x
//! evidence) minus the event penalty, clamped to 0..=100. A `low_evidence` name is capped at
//! `low_evidence_cap` for display, and the ranking is structural: every proven name sorts above
//! every low-evidence one (rule R1), then score descending, tradingsymbol ascending, instrument
//! token. "Proven" means at least `min_trades_evidence` trades, out of sample, net of costs, with
//! profit factor above 1 and positive average R. Evidence with a non-finite metric (a zero-loss
//! profit factor is infinite) is unusable and counts as none. Confluence is 0 when fewer than two
//! families are enabled (one family cannot agree with anything). Every weight and the floor are
//! UNVERIFIED proposals (`docs/strategy/F11-scanner-defaults.md`).
//!
//! Deferred: `EVENT_WINDOW_DAYS` and `PF_FULL_SCORE` stay code constants for now; they move to
//! `[scanner]` (window 1..=30, full-score PF > 1) with the owner's confirmation, tracked in
//! `docs/features/F11-scanner.md`. Step 4 must derive `families_total` from the enabled
//! strategies' families in config, never from the hits.

use bazaar_core::config::{EventKind, ScannerConfig, ScannerWeights};
use bazaar_regime::Fit;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::ports::{EventCalendar, EvidenceSource};
use crate::runner::{Candidate, Hit};
use crate::types::{Evidence, Warning};

/// Calendar days ahead in which an event costs score points. A scoring constant, UNVERIFIED;
/// the hard event blocks live in the gate (G06/G07), this only grades near misses.
pub const EVENT_WINDOW_DAYS: u32 = 5;
/// Profit factor at which the evidence quality reaches 1 (1.0 or below scores 0).
const PF_FULL_SCORE: f64 = 2.0;

/// Everything scoring reads besides the candidates. `as_of` is the decision bar's close: the
/// evidence source may only count trades closed before it (no look-ahead).
pub struct ScoreContext<'a> {
    pub cfg: &'a ScannerConfig,
    pub evidence: &'a dyn EvidenceSource,
    pub events: &'a dyn EventCalendar,
    pub decision_date: NaiveDate,
    pub as_of: DateTime<Tz>,
    /// Method families enabled in this run (the confluence denominator).
    pub families_total: u32,
}

impl std::fmt::Debug for ScoreContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreContext").finish_non_exhaustive()
    }
}

/// The four components in 0..=1 and the event penalty in points; kept so the Why can show them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Components {
    pub confluence: f64,
    pub regime_fit: f64,
    pub liquidity: f64,
    pub evidence: f64,
    pub event_penalty: f64,
}

/// A candidate with its score.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub candidate: Candidate,
    pub components: Components,
    /// Score before the low-evidence cap.
    pub raw_score: u8,
    /// The score ranked on: `raw_score`, capped when `low_evidence`.
    pub score: u8,
    /// The primary hit (best fit, then reward:risk) lacks adequate evidence.
    pub low_evidence: bool,
    /// Usable evidence behind the primary hit (`None` = none, or unusable: in sample, gross,
    /// non-finite).
    pub evidence: Option<Evidence>,
    /// Warnings known at scoring time (events near, calendar not checked, low evidence).
    pub warnings: Vec<Warning>,
}

/// Evidence counts only when out of sample, net of costs and finite.
fn usable(e: &Evidence) -> bool {
    e.out_of_sample
        && e.costs_applied
        && e.profit_factor.is_finite()
        && e.profit_factor >= 0.0
        && e.win_rate.is_finite()
        && (0.0..=1.0).contains(&e.win_rate)
        && e.avg_r.is_finite()
        && e.max_drawdown_pct.is_finite()
}

/// Profit factor mapped to 0..=1: 1.0 (break-even) is 0, `PF_FULL_SCORE` is 1.
fn quality(pf: f64) -> f64 {
    ((pf - 1.0) / (PF_FULL_SCORE - 1.0)).clamp(0.0, 1.0)
}

/// Evidence component of one hit: own profit factor shrunk toward the strategy-wide one by
/// `n / (n + k)`. The prior is the strategy-wide factor only with enough trades, else a neutral
/// 1.0 (never an optimistic number). No usable evidence is the floor, 0.
fn evidence_component(own: Option<&Evidence>, wide: Option<&Evidence>, cfg: &ScannerConfig) -> f64 {
    let Some(own) = own.filter(|e| usable(e)) else {
        return 0.0;
    };
    if own.avg_r <= 0.0 {
        return 0.0; // no positive expectancy: no evidence credit
    }
    let prior = wide
        .filter(|w| usable(w) && w.trades >= cfg.min_trades_evidence)
        .map_or(1.0, |w| w.profit_factor);
    let n = f64::from(own.trades);
    let weight = n / (n + f64::from(cfg.shrink_k));
    quality(weight.mul_add(own.profit_factor, (1.0 - weight) * prior))
}

/// Enough trades, out of sample, net of costs, and the edge is positive (profit factor above 1,
/// average R above 0). Thin or losing evidence is low evidence.
fn proven(e: Option<&Evidence>, cfg: &ScannerConfig) -> bool {
    e.is_some_and(|e| {
        usable(e) && e.trades >= cfg.min_trades_evidence && e.profit_factor > 1.0 && e.avg_r > 0.0
    })
}

/// Absolute log scale: 0 at the floor, 1 at `liquidity_full_score_multiple` times the floor.
fn liquidity_component(median: Decimal, cfg: &ScannerConfig) -> f64 {
    let floor = cfg.min_median_traded_value;
    let mult = cfg.liquidity_full_score_multiple;
    if floor <= Decimal::ZERO || !mult.is_finite() || mult <= 1.0 {
        return 0.0;
    }
    let ratio = median
        .checked_div(floor)
        .and_then(|r| r.to_f64())
        .unwrap_or(0.0);
    if ratio <= 1.0 {
        return 0.0;
    }
    (ratio.ln() / mult.ln()).clamp(0.0, 1.0)
}

/// Mean regime component: 1 for an allowed cell, `penalty_weight` (0..=1) for a penalised one.
fn regime_component(hits: &[Hit]) -> f64 {
    if hits.is_empty() {
        return 0.0;
    }
    let sum: f64 = hits
        .iter()
        .map(|h| match h.fit {
            Fit::Penalised(_) => h.penalty_weight.clamp(0.0, 1.0),
            _ => 1.0,
        })
        .sum();
    sum / f64::from(u32::try_from(hits.len()).unwrap_or(u32::MAX))
}

/// The hit the recommendation will be built on: an allowed fit first, then higher reference
/// reward:risk, then lower strategy id.
pub(crate) fn primary(hits: &[Hit]) -> Option<&Hit> {
    hits.iter().min_by(|a, b| {
        matches!(a.fit, Fit::Penalised(_))
            .cmp(&matches!(b.fit, Fit::Penalised(_)))
            .then(b.signal.reference_rr.cmp(&a.signal.reference_rr))
            .then(a.signal.strategy_id.cmp(b.signal.strategy_id))
    })
}

/// Plain-English label for an event kind (never the Debug repr). The note is config text and
/// must be escaped when rendered (step 4 acceptance, SEC-118-1).
fn event_label(k: EventKind) -> &'static str {
    match k {
        EventKind::Results => "results",
        EventKind::RbiPolicy => "RBI policy",
        EventKind::Budget => "budget",
        EventKind::Expiry => "expiry",
        EventKind::ExDate => "ex-date",
        EventKind::IndexRebalance => "index rebalance",
        EventKind::SpecialSession => "special session",
    }
}

/// Penalty points and warnings from events inside the window. An empty calendar proves nothing:
/// no bonus, and a warning that dates were not checked.
fn event_penalty(c: &Candidate, cx: &ScoreContext<'_>) -> (f64, Vec<Warning>) {
    if cx.events.is_empty() {
        return (0.0, vec![Warning::EventsNotChecked]);
    }
    let events = cx.events.upcoming(
        &c.instrument.tradingsymbol,
        cx.decision_date,
        EVENT_WINDOW_DAYS,
    );
    let mut nearest: Option<i64> = None;
    let mut warnings = Vec::new();
    for e in &events {
        let days = (e.date - cx.decision_date).num_days();
        if !(0..=i64::from(EVENT_WINDOW_DAYS)).contains(&days) {
            continue;
        }
        nearest = Some(nearest.map_or(days, |n| n.min(days)));
        warnings.push(Warning::EventNearby(format!(
            "{} on {}: {}",
            event_label(e.kind),
            e.date,
            e.note
        )));
    }
    let max = cx.cfg.event_penalty_max_points.clamp(0.0, 100.0);
    let pts = nearest.map_or(0.0, |d| {
        let d = f64::from(i32::try_from(d).unwrap_or(0));
        max * (1.0 - d / (f64::from(EVENT_WINDOW_DAYS) + 1.0))
    });
    (pts, warnings)
}

/// The single f64 to `u8` boundary: clamp to 0..=100 and round.
fn to_score(x: f64) -> u8 {
    if !x.is_finite() {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100 and finite-checked"
    )]
    let v = x.clamp(0.0, 100.0).round() as u8;
    v
}

fn weighted(w: &ScannerWeights, c: &Components) -> f64 {
    100.0
        * (w.confluence * c.confluence
            + w.regime_fit * c.regime_fit
            + w.liquidity * c.liquidity
            + w.evidence * c.evidence)
        - c.event_penalty
}

/// Score one candidate. Uses only the candidate (built from bars up to the closed decision bar)
/// and sources asked with `as_of`.
#[must_use]
pub fn score(c: Candidate, cx: &ScoreContext<'_>) -> Scored {
    let cfg = cx.cfg;
    let mut fams: Vec<&str> = c.hits.iter().map(|h| h.family.as_str()).collect();
    fams.sort_unstable();
    fams.dedup();
    let agreeing = u32::try_from(fams.len()).unwrap_or(u32::MAX);
    let total = cx.families_total.max(agreeing).max(1);
    // One enabled family gives every name full confluence for nothing: score it 0.
    let confluence = if cx.families_total < 2 {
        0.0
    } else {
        f64::from(agreeing) / f64::from(total)
    };
    let (own, wide) = primary(&c.hits).map_or((None, None), |h| {
        let id = h.signal.strategy_id;
        (
            cx.evidence
                .for_instrument(id, c.instrument.instrument_token, cx.as_of),
            cx.evidence.for_strategy(id, cx.as_of),
        )
    });
    let (penalty, mut warnings) = event_penalty(&c, cx);
    let components = Components {
        confluence,
        regime_fit: regime_component(&c.hits),
        liquidity: liquidity_component(c.median_traded_value, cfg),
        evidence: evidence_component(own.as_ref(), wide.as_ref(), cfg),
        event_penalty: penalty,
    };
    let raw_score = to_score(weighted(&cfg.weights, &components));
    let low_evidence = !proven(own.as_ref(), cfg);
    let score = if low_evidence {
        warnings.push(Warning::LowEvidence);
        raw_score.min(cfg.low_evidence_cap)
    } else {
        raw_score
    };
    Scored {
        candidate: c,
        components,
        raw_score,
        score,
        low_evidence,
        evidence: own.filter(usable),
        warnings,
    }
}

/// Rank: proven above low-evidence (rule R1, whatever the scores), then score (capped)
/// descending, tradingsymbol ascending, instrument token. The order depends on nothing else, so
/// the same inputs give the same list.
pub fn rank(scored: &mut [Scored]) {
    scored.sort_by(|a, b| {
        let (ia, ib) = (&a.candidate.instrument, &b.candidate.instrument);
        a.low_evidence
            .cmp(&b.low_evidence)
            .then(b.score.cmp(&a.score))
            .then_with(|| ia.tradingsymbol.cmp(&ib.tradingsymbol))
            .then_with(|| ia.instrument_token.cmp(&ib.instrument_token))
    });
}

/// Score and rank all candidates, best first. Not truncated: `max_results` applies after the
/// gate (step 4), because blocked ideas must stay visible and not use up slots.
#[must_use]
pub fn score_and_rank(candidates: Vec<Candidate>, cx: &ScoreContext<'_>) -> Vec<Scored> {
    let mut v: Vec<Scored> = candidates.into_iter().map(|c| score(c, cx)).collect();
    rank(&mut v);
    v
}

#[cfg(test)]
mod tests;
