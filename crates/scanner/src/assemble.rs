//! Turns ranked, scored candidates into a [`ScanResult`]: sizes each primary idea, runs the one
//! `DecisionGate` on it, and builds the `Recommendation` (advice, never an order; nothing here can
//! carry an approval). An idea that cannot be sized or evaluated becomes a `BlockedIdea` in the
//! same rank position, never a silent drop. `max_results` applies after the gate, so blocked
//! ideas never use up result slots; the cut itself is silent by design (it removes only the
//! lowest-ranked passing ideas, nothing blocked is hidden).
//!
//! Each idea is gated alone against the same snapshot and cash: "passed" means it passes by
//! itself, not that all shown ideas are fine together. The gate re-runs at approval time.

use bazaar_core::config::ScannerConfig;
use bazaar_core::instrument::Instrument;
use bazaar_core::mode::ExecutionMode;
use bazaar_core::money::Qty;
use bazaar_regime::Fit;
use bazaar_risk::gate::{Gate, GateInputs};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::inputs::{DataInputs, EvidenceInputs};
use bazaar_risk::types::{OrderIntent, Side, Tri};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

use crate::gate_ctx::ScanGateContext;
use crate::ports::GateContextSource;
use crate::result::{BlockReason, BlockedIdea, ScanResult};
use crate::runner::Hit;
use crate::score::{Scored, primary};
use crate::sizing::{Levels, SizedIdea, round_levels, size_idea};
use crate::types::{
    Action, GateSummary, Horizon, InstrumentRef, Recommendation, RecommendationParts, Warning,
    WhyFactor,
};
use crate::universe::DropReason;

/// Everything `assemble` reads besides the scored candidates.
pub struct AssembleCx<'a> {
    pub cfg: &'a ScannerConfig,
    /// The one gate (its limits and mode are the ones the context was built from).
    pub gate: &'a Gate<'a>,
    pub source: &'a dyn GateContextSource,
    /// Generation time from the injected `Clock`.
    pub now: DateTime<Tz>,
    /// From `ScanOutput::families_total` (enabled strategies' families, never counted from hits).
    pub families_total: u32,
}

impl std::fmt::Debug for AssembleCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssembleCx").finish_non_exhaustive()
    }
}

fn iref(i: &Instrument) -> InstrumentRef {
    InstrumentRef {
        instrument_token: i.instrument_token,
        tradingsymbol: i.tradingsymbol.clone(),
        name: i.name.clone(),
    }
}

/// A blocked idea shows the strategy's levels as given (nothing was rounded or sized).
fn blocked_idea(s: &Scored, reason: BlockReason) -> BlockedIdea {
    let (zone, stop, targets) = primary(&s.candidate.hits).map_or(
        ((Decimal::ZERO, Decimal::ZERO), Decimal::ZERO, Vec::new()),
        |h| {
            (
                (h.signal.entry_low, h.signal.entry_high),
                h.signal.stop,
                h.signal.targets.clone(),
            )
        },
    );
    BlockedIdea {
        instrument: iref(&s.candidate.instrument),
        entry_zone: zone,
        stop_loss: stop,
        targets,
        reason,
    }
}

/// Outside `paper` a Paper-level failure cannot be overridden, so it counts as hard there
/// (RISK-162-9): the idea is non-actionable instead of looking overridable.
fn mode_aware(mut g: GateSummary, mode: ExecutionMode) -> GateSummary {
    if mode != ExecutionMode::Paper && !g.paper_only.is_empty() {
        let moved = std::mem::take(&mut g.paper_only);
        g.overridable_failures.retain(|r| !moved.contains(r));
        g.hard_failures.extend(moved);
    }
    g
}

fn why(
    scored: &Scored,
    hit: &Hit,
    lv: &Levels,
    sized: &SizedIdea,
    cx: &AssembleCx<'_>,
    data: &DataInputs,
) -> Vec<WhyFactor> {
    let cand = &scored.candidate;
    let agreeing = agreeing_families(scored);
    let ev = scored.evidence.as_ref();
    vec![
        WhyFactor::Risk {
            risk_rupees: sized.risk_rupees,
            limit_rupees: sized.budget,
            qty: sized.size.qty,
        },
        WhyFactor::Reward {
            target: lv.targets.first().copied().unwrap_or(lv.entry_high),
            reward_risk: sized.reward_risk,
        },
        WhyFactor::Confluence {
            families_agreeing: agreeing,
            families_total: cx.families_total,
            names: agreeing_ids(scored),
        },
        WhyFactor::RegimeFit {
            market_trend: format!("{:?}", cand.market_regime.trend),
            stock_trend: format!("{:?}", cand.stock_regime.trend),
            vol: format!("{:?}", cand.stock_regime.vol),
            penalised: matches!(hit.fit, Fit::Penalised(_)),
        },
        WhyFactor::Evidence {
            trades: ev.map_or(0, |e| e.trades),
            profit_factor: ev.map(|e| e.profit_factor),
            costs_applied: ev.is_some_and(|e| e.costs_applied),
            out_of_sample: ev.is_some_and(|e| e.out_of_sample),
        },
        WhyFactor::Liquidity {
            median_traded_value: cand.median_traded_value,
        },
        WhyFactor::Invalidation {
            stop: lv.stop,
            max_hold_bars: hit.signal.exit_plan.time_stop_bars,
        },
        // What the gate saw for this instrument: an Unknown list is "not checked".
        match &data.ban_list {
            Tri::Known(b) => WhyFactor::Ban {
                checked: true,
                list_date: Some(b.fetched_at.date_naive().to_string()),
            },
            Tri::Unknown => WhyFactor::Ban {
                checked: false,
                list_date: None,
            },
        },
        WhyFactor::Events {
            checked: !scored.warnings.contains(&Warning::EventsNotChecked),
            notes: scored
                .warnings
                .iter()
                .filter_map(|w| match w {
                    Warning::EventNearby(n) => Some(n.clone()),
                    _ => None,
                })
                .collect(),
        },
    ]
}

fn agreeing_families(s: &Scored) -> u32 {
    let mut fams: Vec<&str> = s.candidate.hits.iter().map(|x| x.family.as_str()).collect();
    fams.sort_unstable();
    fams.dedup();
    u32::try_from(fams.len()).unwrap_or(u32::MAX)
}

/// The scanner's own facts replace the source's for evidence and event flags; a fact it cannot
/// vouch for stays `Unknown`, which the gate refuses. Evidence counts only when out of sample and
/// net of costs (as `Recommendation::new` does), else G01 sees nothing.
fn overlay(mut data: DataInputs, s: &Scored) -> DataInputs {
    data.evidence = match s
        .evidence
        .as_ref()
        .filter(|e| e.costs_applied && e.out_of_sample)
    {
        Some(e) => EvidenceInputs {
            oos_trades: Tri::Known(e.trades),
            profit_factor: Decimal::from_f64(e.profit_factor).map_or(Tri::Unknown, Tri::Known),
            expectancy_r: Decimal::from_f64(e.avg_r).map_or(Tri::Unknown, Tri::Known),
            walk_forward_pass: data.evidence.walk_forward_pass,
        },
        None => EvidenceInputs {
            oos_trades: Tri::Unknown,
            profit_factor: Tri::Unknown,
            expectancy_r: Tri::Unknown,
            walk_forward_pass: data.evidence.walk_forward_pass,
        },
    };
    if s.warnings.contains(&Warning::EventsNotChecked) {
        data.events.results_window = Tri::Unknown;
        data.events.policy_day = Tri::Unknown;
    } else if s
        .warnings
        .iter()
        .any(|w| matches!(w, Warning::EventNearby(_)))
    {
        data.events.results_window = Tri::Known(true);
    } else {
        data.events.results_window = Tri::Known(false);
    }
    data
}

fn agreeing_ids(s: &Scored) -> Vec<String> {
    let mut v: Vec<String> = s
        .candidate
        .hits
        .iter()
        .flat_map(|h| std::iter::once(h.signal.strategy_id).chain(h.merged_ids.iter().copied()))
        .map(str::to_owned)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Size, gate and build one idea. Every price below comes from `SizedIdea.levels` (the rounded
/// zone), never from the signal (RISK-163-1).
fn idea(
    s: &Scored,
    h: &Hit,
    cx: &AssembleCx<'_>,
    ctx: &ScanGateContext,
) -> Result<Recommendation, BlockReason> {
    let inst = &s.candidate.instrument;
    let sig = &h.signal;
    let given = Levels {
        entry_low: sig.entry_low,
        entry_high: sig.entry_high,
        stop: sig.stop,
        targets: sig.targets.clone(),
    };
    let agreeing = agreeing_families(s);
    if cx.families_total == 0 || agreeing > cx.families_total {
        return Err(BlockReason::Sizing(format!(
            "families_total {} cannot hold {agreeing} agreeing families",
            cx.families_total
        )));
    }
    let rounded = round_levels(&given, inst.tick_size)?;
    // Round trip (QUAN-165-1): the buy at the zone top and the stop-out sell at the rounded stop.
    let cost = |q: u32| match (
        ctx.cost().cost(Side::Buy, q, rounded.entry_high),
        ctx.cost().cost(Side::Sell, q, rounded.stop),
    ) {
        (Tri::Known(a), Tri::Known(b)) => a.checked_add(b).map_or(Tri::Unknown, Tri::Known),
        _ => Tri::Unknown,
    };
    let sized = size_idea(inst, &given, ctx.snapshot(), ctx.limits(), &cost)?;
    let l = &sized.levels;
    let intent = OrderIntent {
        symbol: inst.tradingsymbol.clone(),
        side: Side::Buy,
        qty: Qty(sized.size.qty),
        price: l.entry_high,
        stop: Some(l.stop),
        new_stop: None,
        lot_size: inst.lot_size,
        tick_size: inst.tick_size,
    };
    // X03 judges the need for the sized quantity: a delivery buy needs its full notional plus the
    // charges allowance in cash. `available` stays as the context gave it (Unknown blocks).
    let mut data = overlay(
        cx.source
            .data_for(inst, h)
            .map_err(|e| BlockReason::GateInputsUnavailable(e.to_string()))?,
        s,
    );
    data.margin.qty = sized.size.qty;
    data.margin.required = sized
        .notional
        .checked_add(sized.allowance)
        .map_or(Tri::Unknown, Tri::Known);
    let inputs = GateInputs {
        cost: ctx.cost(),
        data: &data,
        instrument: Some(inst),
        defined_max_loss: None,
        mode: ctx.mode(),
        entry_zone: Tri::Known(EntryZone {
            low: l.entry_low,
            high: l.entry_high,
        }),
        live_price: ctx
            .reference_price(inst.instrument_token)
            .map_or(Tri::Unknown, Tri::Known),
        exposure: ctx.exposure(),
    };
    let ev = cx
        .gate
        .evaluate(&intent, ctx.snapshot(), &inputs)
        .map_err(|e| BlockReason::GateReject(e.to_string()))?;
    if ev.ctx().limits != *ctx.limits() {
        return Err(BlockReason::LimitsChanged);
    }
    // One clock for both stamps; the mode is the one the rules ran under.
    let gate = mode_aware(
        GateSummary::from_verdict(ev.verdict(), cx.now),
        ev.ctx().mode,
    );
    let bar_secs = u64::try_from(sig.signal_bar_time.timestamp())
        .map_err(|_| BlockReason::Sizing("the signal bar time is before 1970".to_owned()))?;
    let parts = RecommendationParts {
        // Names the instrument and the bar only: not an approval key (approval binds to the order
        // digest) and the same on every scan of that bar, whatever the strategy or levels.
        id: (u128::from(inst.instrument_token) << 64) | u128::from(bar_secs),
        generated_at: cx.now,
        data_as_of: sig.signal_bar_time,
        instrument: iref(inst),
        action: Action::Buy,
        entry_zone: (l.entry_low, l.entry_high),
        stop_loss: l.stop,
        targets: l.targets.clone(),
        horizon: Horizon::Swing,
        size: sized.size,
        risk_rupees: sized.risk_rupees,
        reward_risk: sized.reward_risk,
        agreeing_strategies: agreeing_ids(s),
        confidence: s.score,
        regime: s.candidate.stock_regime.clone(),
        backtest: s.evidence.clone(),
        why: why(s, h, l, &sized, cx, &data),
        warnings: s.warnings.clone(),
        gate,
    };
    Recommendation::new(parts, cx.cfg.min_trades_evidence, cx.cfg.low_evidence_cap)
        .map_err(|e| BlockReason::Sizing(format!("the recommendation was refused: {e}")))
}

/// `scored` must already be ranked. The context is read once; blocked ideas keep their rank
/// position in `blocked`, hard gate failures are blocked, and only then are `ideas` cut to
/// `max_results`.
#[must_use]
pub fn assemble(
    scored: &[Scored],
    dropped: Vec<(String, DropReason)>,
    cx: &AssembleCx<'_>,
) -> ScanResult {
    let ctx = cx.source.context();
    let (mut ideas, mut blocked) = (Vec::new(), Vec::new());
    for s in scored {
        let out = match (&ctx, primary(&s.candidate.hits)) {
            (Err(e), _) => Err(BlockReason::GateInputsUnavailable(e.to_string())),
            (Ok(_), None) => Err(BlockReason::Sizing(
                "the candidate has no signal".to_owned(),
            )),
            (Ok(c), Some(h)) => idea(s, h, cx, c),
        };
        match out {
            Ok(r) if r.gate().hard_failures.is_empty() => ideas.push(r),
            Ok(r) => {
                let reason = BlockReason::GateHardFail(r.gate().hard_failures.clone());
                blocked.push(BlockedIdea {
                    instrument: r.instrument().clone(),
                    entry_zone: r.entry_zone(),
                    stop_loss: r.stop_loss(),
                    targets: r.targets().to_vec(),
                    reason,
                });
            }
            Err(reason) => blocked.push(blocked_idea(s, reason)),
        }
    }
    ideas.truncate(usize::try_from(cx.cfg.max_results).unwrap_or(usize::MAX));
    ScanResult::new(ideas, blocked, dropped)
}

#[cfg(test)]
mod tests;
