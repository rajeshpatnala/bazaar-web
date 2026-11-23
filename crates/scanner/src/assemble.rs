//! Turns ranked, scored candidates into a [`ScanResult`]: sizes each primary idea, runs the one
//! `DecisionGate` on it, and builds the `Recommendation` (advice, never an order; nothing here can
//! carry an approval). An idea that cannot be sized or evaluated becomes a `BlockedIdea` in the
//! same rank position, never a silent drop. `max_results` applies after the gate, so blocked
//! ideas never use up result slots.

use bazaar_core::config::ScannerConfig;
use bazaar_core::instrument::Instrument;
use bazaar_core::mode::ExecutionMode;
use bazaar_core::money::Qty;
use bazaar_regime::Fit;
use bazaar_risk::gate::{Gate, GateInputs};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::types::{OrderIntent, Side, Tri};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

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
) -> Vec<WhyFactor> {
    let cand = &scored.candidate;
    let mut fams: Vec<&str> = cand.hits.iter().map(|x| x.family.as_str()).collect();
    fams.sort_unstable();
    fams.dedup();
    let agreeing = u32::try_from(fams.len()).unwrap_or(u32::MAX);
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
            families_total: cx.families_total.max(agreeing),
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
        // A banned or unlisted-ban candidate was dropped by the runner before scoring.
        WhyFactor::Ban {
            checked: true,
            list_date: None,
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
    let price = round_levels(&given, inst.tick_size)?.entry_high;
    let cost = |q: u32| ctx.cost().cost(Side::Buy, q, price);
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
    let mut data = ctx.data().clone();
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
    let gate = mode_aware(
        GateSummary::from_verdict(ev.verdict(), bazaar_core::clock::Clock::now_ist(ev.clock())),
        cx.gate.mode(),
    );
    let parts = RecommendationParts {
        id: (u128::from(inst.instrument_token) << 64)
            | u128::from(sig.signal_bar_time.timestamp().unsigned_abs()),
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
        why: why(s, h, l, &sized, cx),
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
