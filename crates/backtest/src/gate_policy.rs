//! The backtest's `EntryPolicy`: every entry goes through the real F6 `DecisionGate`.
//!
//! The same `Gate`, `GateConfig` (`config/risk.toml`) and sizing code that paper and live use judge
//! each sized signal at the close of its decision bar. The gate clock is frozen at that bar's
//! `close_time` (IST), never the wall clock. The portfolio snapshot is rebuilt from the trades the
//! engine has closed so far ([`EntryPolicy::observe`]); the book is flat whenever the engine asks
//! (it holds one position at a time). There is no override path: `Gate::decide` gets `None` for the
//! override request and a sink that refuses every write. A rejection is never hidden: it is
//! recorded with its rule ids and reasons ([`GatePolicy::rejections`]) and counted as `skipped`.
//!
//! What the caller must supply ([`GateEnv`]) is what a backtest cannot derive from bars: the
//! evidence record, the symbol's sector and the event calendar. Unknown means reject (fail closed).
//! Modelled as: session open (the order is staged for the next session), no kill switch, paper
//! mode evidence, margin need = notional against equity (no intraday leverage). Not modelled: held
//! positions in other symbols (single-instrument runs), so the G05 correlation window is empty.

use std::collections::BTreeMap;

use bazaar_core::candle::Candle;
use bazaar_core::clock::Clock;
use bazaar_core::money::{Price, Qty};
use bazaar_risk::audit::{AuditError, AuditRecord, AuditSink};
use bazaar_risk::baseline::MemoryBaseline;
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_risk::gate::{
    CostModel as GateCostModel, Decision, Evaluation, Gate, GateInputs, ModeEvidence,
};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::sizing::Sized;
use bazaar_risk::snapshot::{LosingExit, PortfolioSnapshot};
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::RuleId;
use bazaar_strategies::Signal;
use chrono::{DateTime, Datelike, Days, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::engine::{EntryPolicy, EntryVerdict, RunSpec, fill_params, round_trip_cost, snap};
use crate::fill::trade_date;
use crate::types::{BacktestError, Trade};

/// Date-dependent event answers for the symbol (results window, policy day, own-expiry option).
pub type EventsFor<'a> = &'a dyn Fn(NaiveDate) -> EventFlags;

/// Inputs a backtest cannot derive from bars. No `Default`: every field is a decision.
pub struct GateEnv<'a> {
    /// Cost-adjusted out-of-sample evidence (G01). `Unknown` fields reject every entry.
    pub evidence: EvidenceInputs,
    /// Sector of the run's symbol (G05).
    pub sector: String,
    pub events: EventsFor<'a>,
}

impl std::fmt::Debug for GateEnv<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GateEnv")
            .field("sector", &self.sector)
            .finish_non_exhaustive()
    }
}

/// One refused entry: when, which rules failed, and why (verdict text or the gate's error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub at: DateTime<Tz>,
    /// Failed rules in evaluation order; empty when the gate could not produce a verdict.
    pub rules: Vec<RuleId>,
    pub reasons: Vec<String>,
}

impl Rejection {
    fn error(at: DateTime<Tz>, why: impl Into<String>) -> Self {
        Self {
            at,
            rules: Vec::new(),
            reasons: vec![why.into()],
        }
    }
}

/// The failed rules of a refused verdict, each with observed value, limit and the gate's reason.
fn refusal(at: DateTime<Tz>, eval: &Evaluation) -> Rejection {
    let failed: Vec<_> = eval.verdict().failures().collect();
    Rejection {
        at,
        rules: failed.iter().map(|r| r.rule()).collect(),
        reasons: failed
            .iter()
            .map(|r| {
                format!(
                    "{}: {} (limit {}); {}",
                    r.rule(),
                    r.observed(),
                    r.limit(),
                    r.message()
                )
            })
            .collect(),
    }
}

/// A trade reduced to what the snapshot needs.
struct Closed {
    symbol: String,
    entry_day: NaiveDate,
    exit_day: NaiveDate,
    exit_at: DateTime<Tz>,
    net: Decimal,
}

/// The gate clock: frozen at the decision bar's close.
struct SimClock(DateTime<Tz>);

impl Clock for SimClock {
    fn now_ist(&self) -> DateTime<Tz> {
        self.0
    }
}

/// Refuses every audit write: a backtest has no override, so nothing may reach the sink.
struct NoOverrideSink;

impl AuditSink for NoOverrideSink {
    fn write(&self, _record: &AuditRecord) -> Result<(), AuditError> {
        Err(AuditError::new("backtests run without overrides"))
    }
}

/// The gate's cost model over the backtest's: entry and stop-exit legs of a long entry, charges
/// plus slippage, the same estimate the engine sized with. Anything else is `Unknown`.
#[derive(Debug)]
struct GateCost<'a, 'b> {
    spec: &'b RunSpec<'a>,
    stop: Price,
    token: u32,
    date: NaiveDate,
}

impl GateCostModel for GateCost<'_, '_> {
    fn cost(&self, side: Side, qty: u32, price: Decimal) -> Tri<Decimal> {
        if side != Side::Buy {
            return Tri::Unknown;
        }
        let Ok(entry) = snap(self.spec.instrument.tick_size, price) else {
            return Tri::Unknown;
        };
        let fp = fill_params(self.spec);
        round_trip_cost(
            self.spec,
            &fp,
            (entry, self.stop),
            qty,
            (self.token, self.date),
        )
        .map_or(Tri::Unknown, Tri::Known)
    }
}

/// The real-gate entry policy. One per run.
pub struct GatePolicy<'a> {
    spec: &'a RunSpec<'a>,
    cfg: &'a GateConfig,
    env: GateEnv<'a>,
    closed: Vec<Closed>,
    rejections: Vec<Rejection>,
    approved: u32,
}

impl std::fmt::Debug for GatePolicy<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatePolicy")
            .field("approved", &self.approved)
            .field("rejections", &self.rejections.len())
            .finish_non_exhaustive()
    }
}

impl<'a> GatePolicy<'a> {
    /// # Errors
    /// `Invalid` when the run's `EffectiveLimits` are not the gate config's own (sizing and the
    /// gate would then judge against different limits).
    pub fn new(
        spec: &'a RunSpec<'a>,
        cfg: &'a GateConfig,
        env: GateEnv<'a>,
    ) -> Result<Self, BacktestError> {
        if cfg.effective(&Baseline::none()) != *spec.limits {
            return Err(BacktestError::Invalid(
                "run limits differ from the gate config".into(),
            ));
        }
        Ok(Self {
            spec,
            cfg,
            env,
            closed: Vec::new(),
            rejections: Vec::new(),
            approved: 0,
        })
    }

    /// Every refused entry so far, in order.
    #[must_use]
    pub fn rejections(&self) -> &[Rejection] {
        &self.rejections
    }

    /// Entries the gate passed.
    #[must_use]
    pub const fn approved(&self) -> u32 {
        self.approved
    }

    fn snapshot(&self, now: DateTime<Tz>, equity: Decimal) -> PortfolioSnapshot {
        let today = trade_date(now);
        let week_start = today
            .checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_monday())))
            .map_or(Tri::Unknown, Tri::Known);
        let sum = |keep: &dyn Fn(&Closed) -> bool| {
            self.closed
                .iter()
                .filter(|c| keep(c))
                .try_fold(Decimal::ZERO, |a, c| a.checked_add(c.net))
                .map_or(Tri::Unknown, Tri::Known)
        };
        let count = |n: usize| u32::try_from(n).map_or(Tri::Unknown, Tri::Known);
        let entries = self.closed.iter().filter(|c| c.entry_day == today).count();
        let exits = self.closed.iter().filter(|c| c.exit_day == today).count();
        let streak = self
            .closed
            .iter()
            .rev()
            .filter(|c| c.exit_day == today)
            .take_while(|c| c.net < Decimal::ZERO)
            .count();
        let mut last: BTreeMap<&str, DateTime<Tz>> = BTreeMap::new();
        for c in self.closed.iter().filter(|c| c.net < Decimal::ZERO) {
            last.insert(&c.symbol, c.exit_at);
        }
        let mut s = PortfolioSnapshot::unknown();
        s.as_of = Tri::Known(now);
        s.positions = Tri::Known(Vec::new());
        s.consecutive_losses = count(streak);
        s.day_pnl = sum(&|c| c.exit_day == today);
        s.week_pnl = match week_start {
            Tri::Known(w) => sum(&|c| c.exit_day >= w && c.exit_day <= today),
            Tri::Unknown => Tri::Unknown,
        };
        s.week_start = week_start;
        s.entries_today = count(entries);
        s.orders_last_minute = Tri::Known(0);
        s.orders_today = count(entries.saturating_add(exits));
        s.overrides_today = Tri::Known(0);
        s.overrides_week = Tri::Known(0);
        s.kill_switch = Tri::Known(false);
        s.starting_cash = Tri::Known(self.spec.capital);
        s.realised_pnl = sum(&|_| true);
        s.equity_mtm = Tri::Known(equity);
        s.marks = Tri::Known(Vec::new());
        s.last_losing_exits = Tri::Known(
            last.into_iter()
                .map(|(symbol, at)| LosingExit {
                    symbol: symbol.to_owned(),
                    at,
                })
                .collect(),
        );
        s
    }

    fn judge(&self, signal: &Signal, sized: &Sized, equity: Decimal) -> Result<(), Rejection> {
        let (spec, now) = (self.spec, signal.signal_bar_time);
        let fail = |why: String| Rejection::error(now, why);
        let tick = spec.instrument.tick_size;
        let idx = spec.bars.partition_point(|b| b.close_time() <= now);
        let bar = idx
            .checked_sub(1)
            .and_then(|i| spec.bars.get(i))
            .filter(|b| b.close_time() == now)
            .ok_or_else(|| fail("the decision bar is not in the run's bars".into()))?;
        let fill_day = trade_date(spec.bars.get(idx).map_or(now, Candle::close_time));
        let token = u32::try_from(spec.instrument.instrument_token)
            .map_err(|_| fail("instrument token too large".into()))?;
        let entry = snap(tick, signal.entry_high).map_err(|e| fail(e.to_string()))?;
        let stop = snap(tick, signal.stop).map_err(|e| fail(e.to_string()))?;
        let cost = GateCost {
            spec,
            stop,
            token,
            date: fill_day,
        };
        let events = (self.env.events)(trade_date(now));
        let data = DataInputs {
            inputs_as_of: Tri::Known(now),
            evidence: self.env.evidence,
            ban_list: Tri::Unknown,
            stock_fno_underlying: Tri::Known(None),
            events,
            quote_at: Tri::Known(now),
            session_open: Tri::Known(true),
            margin: MarginInputs {
                qty: sized.qty,
                required: Tri::Known(sized.notional),
                available: Tri::Known(equity),
            },
        };
        let mode = ModeEvidence {
            approval: None,
            compliance_fresh: Tri::Known(false),
            readiness_report: Tri::Known(false),
            owner_opt_in: Tri::Known(false),
            live: Tri::Known(false),
        };
        let exposure = ExposureInputs {
            sectors: BTreeMap::from([(
                spec.instrument.tradingsymbol.clone(),
                self.env.sector.clone(),
            )]),
            returns: BTreeMap::new(),
        };
        let inputs = GateInputs {
            cost: &cost,
            data: &data,
            instrument: Some(spec.instrument),
            defined_max_loss: None,
            mode: &mode,
            entry_zone: Tri::Known(EntryZone {
                low: signal.entry_low,
                high: signal.entry_high,
            }),
            live_price: Tri::Known(bar.close()),
            exposure: Some(&exposure),
        };
        let intent = OrderIntent {
            symbol: spec.instrument.tradingsymbol.clone(),
            side: Side::Buy,
            qty: Qty(sized.qty),
            price: entry.value(),
            stop: Some(stop.value()),
            new_stop: None,
            lot_size: spec.instrument.lot_size,
            tick_size: tick,
        };
        let (clock, store) = (SimClock(now), MemoryBaseline::new());
        let gate = Gate::new(self.cfg, &store, &clock);
        match gate.decide(
            &intent,
            &self.snapshot(now, equity),
            &inputs,
            None,
            &NoOverrideSink,
        ) {
            Ok(Decision::Minted(_)) => Ok(()),
            Ok(Decision::Refused(eval)) => Err(refusal(now, &eval)),
            Err(e) => Err(fail(e.to_string())),
        }
    }
}

impl EntryPolicy for GatePolicy<'_> {
    fn observe(&mut self, closed: &[Trade]) {
        let seen = self.closed.len();
        for t in closed.iter().skip(seen) {
            self.closed.push(Closed {
                symbol: t.entry.leg.symbol.clone(),
                entry_day: trade_date(t.entry.bar_time),
                exit_day: trade_date(t.exit.bar_time),
                exit_at: t.exit.bar_time,
                net: t.net_pnl,
            });
        }
    }

    fn decide(&mut self, signal: &Signal, sized: &Sized, equity: Decimal) -> EntryVerdict {
        match self.judge(signal, sized, equity) {
            Ok(()) => {
                self.approved = self.approved.saturating_add(1);
                EntryVerdict::Approve
            }
            Err(r) => {
                let why = r.reasons.join(" | ");
                self.rejections.push(r);
                EntryVerdict::Reject(why)
            }
        }
    }
}

#[cfg(test)]
mod tests;
