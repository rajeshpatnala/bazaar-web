//! The real `GatePort`: maps an `execution::OrderIntent` onto the risk crate's `DecisionGate`.
//!
//! Everything the gate needs besides the intent and the quote comes from a sync [`GateContext`]
//! (snapshot, data inputs, instrument master row, stop). Lot and tick sizes are taken from the
//! instrument row, never defaulted. Fail closed: any missing input is a `Block`.
//!
//! Decisions worth knowing:
//! - The quote age the gate checks (G15) is taken from the quote the adapter was handed, never
//!   from the context: an unparsable timestamp or no quote is `Unknown`.
//! - The drift limit applies to exits too (an exit approved at a price that has since moved is
//!   re-proposed, not sent blind); it is the only entry-protection control kept on exits.
//! - A `reduce_only` order must fit inside the held quantity net of exits already pending.

use std::sync::Arc;

use bazaar_core::clock::Clock;
use bazaar_core::config::ExecutionSection;
use bazaar_core::instrument::Instrument;
use bazaar_core::money::Qty;
use bazaar_core::ports::QuoteSnapshot;
use bazaar_risk::audit::{AuditError, AuditRecord, AuditSink};
use bazaar_risk::baseline::LimitsBaseline;
use bazaar_risk::config::GateConfig;
use bazaar_risk::gate::{CostModel, Decision, Gate, GateInputs, ModeEvidence};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::DataInputs;
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::{OrderIntent as RiskIntent, Tri};
use chrono::DateTime;
use rust_decimal::Decimal;

use crate::pipeline::{GateOutcome, GatePort};
use crate::types::{OrderIntent, OrderKind};

/// Basis points in one whole.
const BPS: i64 = 10_000;

/// Owned inputs for one gate run. A context that cannot supply them returns `None` (Block).
#[derive(Debug, Clone)]
pub struct GateInputsOwned {
    pub snapshot: PortfolioSnapshot,
    /// `quote_at` here is ignored: the adapter sets it from the fetched quote.
    pub data: DataInputs,
    pub mode: ModeEvidence,
    pub entry_zone: Tri<EntryZone>,
    pub exposure: Option<ExposureInputs>,
    pub defined_max_loss: Option<Decimal>,
    /// Instrument-master row for the intent's symbol; `None` blocks.
    pub instrument: Option<Instrument>,
    /// Protective stop for an entry (the recommendation's); exits carry none.
    pub stop: Option<Decimal>,
    /// The price the user was shown; the fresh quote's drift is measured from it. `None` blocks
    /// an entry (a `reduce_only` exit without it skips the drift check).
    pub proposed_price: Option<Decimal>,
    /// Quantity of other `reduce_only` intents for this symbol already pending, approved or sent
    /// (netted against the held quantity). `None` = unknown, which blocks `reduce_only`.
    pub pending_exit_qty: Option<Decimal>,
}

/// Sync source of everything around an intent. Called inside `spawn_blocking`.
pub trait GateContext: Send + Sync + 'static {
    fn inputs(&self, intent: &OrderIntent) -> Option<GateInputsOwned>;
}

/// An audit sink that always refuses: this adapter never requests an override.
struct NoOverrides;
impl AuditSink for NoOverrides {
    fn write(&self, _: &AuditRecord) -> Result<(), AuditError> {
        Err(AuditError::new("overrides are not used by the pipeline"))
    }
}

/// `GatePort` over the real `bazaar_risk::gate::Gate`.
pub struct RiskGateAdapter<C: GateContext> {
    config: Arc<GateConfig>,
    baseline: Arc<dyn LimitsBaseline + Send + Sync>,
    clock: Arc<dyn Clock>,
    cost: Arc<dyn CostModel>,
    ctx: C,
    /// `execution.max_quote_drift_bps`.
    max_quote_drift_bps: u32,
    /// `execution.kill_allow_reduce_only`.
    kill_allow_reduce_only: bool,
}

impl<C: GateContext> std::fmt::Debug for RiskGateAdapter<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RiskGateAdapter")
            .field("max_quote_drift_bps", &self.max_quote_drift_bps)
            .field("kill_allow_reduce_only", &self.kill_allow_reduce_only)
            .finish_non_exhaustive()
    }
}

impl<C: GateContext> RiskGateAdapter<C> {
    /// Drift limit and the kill-exit switch come from the typed, loader-validated section.
    #[must_use]
    pub fn new(
        config: Arc<GateConfig>,
        baseline: Arc<dyn LimitsBaseline + Send + Sync>,
        clock: Arc<dyn Clock>,
        cost: Arc<dyn CostModel>,
        ctx: C,
        exec: &ExecutionSection,
    ) -> Self {
        Self {
            config,
            baseline,
            clock,
            cost,
            ctx,
            max_quote_drift_bps: exec.max_quote_drift_bps,
            kill_allow_reduce_only: exec.kill_allow_reduce_only,
        }
    }
}

fn block(why: &str) -> GateOutcome {
    GateOutcome::Block(why.to_owned())
}

/// Pure mapping to the risk crate's intent. Lot and tick come from the instrument row; a market
/// order is priced at `market_price` (the quote); `None` for a market order is `None` out.
#[must_use]
pub fn to_risk_intent(
    intent: &OrderIntent,
    inst: &Instrument,
    market_price: Option<Decimal>,
    stop: Option<Decimal>,
) -> Option<RiskIntent> {
    let price = match (intent.kind, market_price) {
        (OrderKind::Limit(p), _) => p,
        (OrderKind::Market, Some(m)) => m,
        (OrderKind::Market, None) => return None,
    };
    Some(RiskIntent {
        symbol: intent.symbol.clone(),
        side: intent.side,
        qty: Qty(intent.qty),
        price,
        stop: if intent.reduce_only { None } else { stop },
        new_stop: None,
        lot_size: inst.lot_size,
        tick_size: inst.tick_size,
    })
}

/// Drift of `now` from `reference` in bps; `None` when the reference is not positive or the
/// arithmetic overflows (the caller treats that as drifted).
fn drift_bps(reference: Decimal, now: Decimal) -> Option<Decimal> {
    if reference <= Decimal::ZERO {
        return None;
    }
    (now - reference)
        .abs()
        .checked_mul(Decimal::from(BPS))?
        .checked_div(reference)
}

/// Time of the quote as the gate's G15 needs it; unparsable is `Unknown`.
fn quote_time(q: Option<&QuoteSnapshot>) -> Tri<DateTime<chrono_tz::Tz>> {
    q.and_then(|q| DateTime::parse_from_rfc3339(&q.at_ist_rfc3339).ok())
        .map_or(Tri::Unknown, |t| {
            Tri::Known(t.with_timezone(&chrono_tz::Asia::Kolkata))
        })
}

impl<C: GateContext> GatePort for RiskGateAdapter<C> {
    fn gate(&self, intent: &OrderIntent, quote: Option<&QuoteSnapshot>) -> GateOutcome {
        let Some(mut inp) = self.ctx.inputs(intent) else {
            return block("gate inputs unavailable");
        };
        inp.data.quote_at = quote_time(quote);
        let last = quote.map(|q| q.last);
        let held = inp.snapshot.position(&intent.symbol);
        let killed = inp.snapshot.kill_switch != Tri::Known(false);
        if intent.reduce_only {
            // RISK-109-3: reduce_only must really shrink a live position, net of exits already
            // pending, killed or not. Anything else is an entry in disguise.
            let Some(pending) = inp.pending_exit_qty else {
                return block("reduce_only: pending exit quantity unknown");
            };
            let shrinks = held.is_some_and(|p| {
                p.side != intent.side && Decimal::from(intent.qty) + pending <= Decimal::from(p.qty)
            });
            if !shrinks {
                return block("reduce_only: no live position to reduce");
            }
            if killed && !self.kill_allow_reduce_only {
                return block("kill switch engaged: exits are not allowed by configuration");
            }
        } else if killed {
            return block("kill switch engaged: only reduce_only exits are allowed");
        } else if last.is_none() {
            // RISK-109-7: no quote blocks an entry.
            return block("no quote");
        } else if inp.proposed_price.is_none() {
            return block("no proposed price to measure quote drift from");
        }
        let Some(inst) = inp.instrument.as_ref() else {
            return block("instrument not in the master");
        };
        let Some(risk) = to_risk_intent(intent, inst, last, inp.stop) else {
            return block("no quote for a market order");
        };
        let gate_inputs = GateInputs {
            cost: &*self.cost,
            data: &inp.data,
            instrument: Some(inst),
            defined_max_loss: inp.defined_max_loss,
            mode: &inp.mode,
            entry_zone: inp.entry_zone,
            live_price: last.map_or(Tri::Unknown, Tri::Known),
            exposure: inp.exposure.as_ref(),
        };
        let gate = Gate::new(&self.config, &*self.baseline, &*self.clock);
        let minted = match gate.decide(&risk, &inp.snapshot, &gate_inputs, None, &NoOverrides) {
            Ok(Decision::Minted(m)) => m,
            Ok(Decision::Refused(eval)) => {
                let why: Vec<String> = eval
                    .verdict()
                    .failures()
                    .map(|r| format!("{}: {}", r.rule().as_str(), r.message()))
                    .collect();
                return GateOutcome::Block(why.join("; "));
            }
            Err(_) => return block("gate could not size or mint the order"),
        };
        if minted.qty() != intent.qty {
            return block("gate sized the order differently");
        }
        let risk_inr = minted.risk_rupees();
        if let (Some(reference), Some(last)) = (inp.proposed_price, last) {
            let drifted = drift_bps(reference, last)
                .is_none_or(|d| d > Decimal::from(self.max_quote_drift_bps));
            if drifted {
                return GateOutcome::Drifted { new_risk: risk_inr };
            }
        }
        GateOutcome::Pass { risk_inr }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{IntentId, Product, Side};
    use bazaar_core::instrument::{Exchange, InstrumentType};
    use bazaar_risk::baseline::BaselineError;
    use bazaar_risk::config::Baseline;
    use bazaar_risk::rules::inputs::{EventFlags, EvidenceInputs, MarginInputs};
    use bazaar_risk::snapshot::{Mark, Position};
    use chrono::{DateTime, Duration, NaiveDate, TimeZone};
    use chrono_tz::{Asia::Kolkata, Tz};
    use std::collections::BTreeMap;

    const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }
    fn now() -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap()
    }
    struct At;
    impl Clock for At {
        fn now_ist(&self) -> DateTime<Tz> {
            now()
        }
    }
    struct Store;
    impl LimitsBaseline for Store {
        fn read(&self) -> Result<Baseline, BaselineError> {
            Ok(Baseline::none())
        }
    }
    #[derive(Debug)]
    struct Cost;
    impl CostModel for Cost {
        fn cost(&self, _: bazaar_risk::types::Side, _: u32, _: Decimal) -> Tri<Decimal> {
            Tri::Known(d("20"))
        }
    }

    /// Context with knobs the tests turn.
    struct Ctx {
        held: Vec<Position>,
        killed: bool,
        instrument: bool,
        proposed: Option<Decimal>,
        none: bool,
        pending: Option<Decimal>,
        kill_unknown: bool,
        lot: u32,
    }
    impl Ctx {
        fn new() -> Self {
            Self {
                held: vec![],
                killed: false,
                instrument: true,
                proposed: Some(d("50")),
                none: false,
                pending: Some(Decimal::ZERO),
                kill_unknown: false,
                lot: 1,
            }
        }
    }
    fn long_abc(qty: u32) -> Position {
        Position {
            symbol: "ABC".into(),
            side: Side::Buy,
            qty,
            stop: Tri::Known(d("40")),
            risk_price: Tri::Known(d("50")),
            avg_cost: Tri::Known(d("50")),
        }
    }
    impl GateContext for Ctx {
        fn inputs(&self, _: &OrderIntent) -> Option<GateInputsOwned> {
            if self.none {
                return None;
            }
            let end = now().date_naive();
            let series = (0..100u32)
                .map(|i| {
                    (
                        end - Duration::days(i64::from(99 - i)),
                        Decimal::new(i64::from(i % 7) - 3, 3),
                    )
                })
                .collect();
            let mut s = PortfolioSnapshot::unknown();
            s.as_of = Tri::Known(now());
            s.positions = Tri::Known(self.held.clone());
            s.consecutive_losses = Tri::Known(0);
            s.day_pnl = Tri::Known(Decimal::ZERO);
            s.week_pnl = Tri::Known(Decimal::ZERO);
            s.week_start = Tri::Known(NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
            s.entries_today = Tri::Known(0);
            s.orders_last_minute = Tri::Known(0);
            s.orders_today = Tri::Known(0);
            s.overrides_today = Tri::Known(0);
            s.overrides_week = Tri::Known(0);
            s.kill_switch = if self.kill_unknown {
                Tri::Unknown
            } else {
                Tri::Known(self.killed)
            };
            s.charges_buffer = Tri::Known(d("20"));
            s.starting_cash = Tri::Known(d("250000"));
            s.realised_pnl = Tri::Known(Decimal::ZERO);
            s.equity_mtm = Tri::Known(d("250000"));
            s.marks = Tri::Known(
                self.held
                    .iter()
                    .map(|p| Mark {
                        symbol: p.symbol.clone(),
                        price: d("50"),
                    })
                    .collect(),
            );
            s.last_losing_exits = Tri::Known(vec![]);
            Some(GateInputsOwned {
                snapshot: s,
                data: DataInputs {
                    inputs_as_of: Tri::Known(now()),
                    evidence: EvidenceInputs {
                        oos_trades: Tri::Known(100),
                        profit_factor: Tri::Known(d("2")),
                        expectancy_r: Tri::Known(d("0.3")),
                        walk_forward_pass: Tri::Known(true),
                    },
                    ban_list: Tri::Unknown,
                    stock_fno_underlying: Tri::Known(None),
                    events: EventFlags {
                        results_window: Tri::Known(false),
                        policy_day: Tri::Known(false),
                        option_on_own_expiry: Tri::Known(false),
                    },
                    quote_at: Tri::Known(now()),
                    session_open: Tri::Known(true),
                    margin: MarginInputs {
                        qty: 10,
                        required: Tri::Known(d("5000")),
                        available: Tri::Known(d("250000")),
                    },
                },
                mode: ModeEvidence {
                    approval: None,
                    compliance_fresh: Tri::Known(false),
                    readiness_report: Tri::Known(false),
                    owner_opt_in: Tri::Known(false),
                    live: Tri::Known(false),
                },
                entry_zone: Tri::Known(EntryZone {
                    low: d("49.5"),
                    high: d("50.1"),
                }),
                exposure: Some(ExposureInputs {
                    sectors: BTreeMap::from([("ABC".to_string(), "IT".to_string())]),
                    returns: BTreeMap::from([("ABC".to_string(), series)]),
                }),
                defined_max_loss: None,
                instrument: self.instrument.then(|| Instrument {
                    instrument_token: 1,
                    exchange_token: 1,
                    tradingsymbol: "ABC".into(),
                    name: "ABC".into(),
                    exchange: Exchange::Nse,
                    instrument_type: InstrumentType::Equity,
                    segment: "NSE".into(),
                    expiry: None,
                    strike: None,
                    tick_size: d("0.05"),
                    lot_size: self.lot,
                }),
                stop: Some(d("40")),
                proposed_price: self.proposed,
                pending_exit_qty: self.pending,
            })
        }
    }

    fn exec_section() -> ExecutionSection {
        let r: bazaar_core::config::RiskConfig = bazaar_core::config::parse_str(TEXT).unwrap();
        r.execution
    }
    fn adapter(c: Ctx) -> RiskGateAdapter<Ctx> {
        adapter_with(c, |_| {})
    }
    fn adapter_with(c: Ctx, f: impl FnOnce(&mut ExecutionSection)) -> RiskGateAdapter<Ctx> {
        let mut e = exec_section();
        f(&mut e);
        RiskGateAdapter::new(
            Arc::new(GateConfig::from_toml_str(TEXT).unwrap()),
            Arc::new(Store),
            Arc::new(At),
            Arc::new(Cost),
            c,
            &e,
        )
    }
    fn intent(side: Side, qty: u32, reduce_only: bool) -> OrderIntent {
        OrderIntent {
            id: IntentId(1),
            symbol: "ABC".into(),
            side,
            qty,
            kind: OrderKind::Limit(d("50")),
            product: Product::Mis,
            reduce_only,
            created_at: now(),
        }
    }
    fn quote(last: &str) -> QuoteSnapshot {
        QuoteSnapshot {
            last: d(last),
            lower_circuit: None,
            upper_circuit: None,
            bar_volume: None,
            at_ist_rfc3339: now().to_rfc3339(),
        }
    }
    fn is_block(o: &GateOutcome) -> bool {
        matches!(o, GateOutcome::Block(_))
    }

    #[test]
    fn entry_passes_with_risk_inr() {
        let a = adapter(Ctx::new());
        let out = a.gate(&intent(Side::Buy, 10, false), Some(&quote("50")));
        // 10 * |50-40| + 20 cost
        assert_eq!(out, GateOutcome::Pass { risk_inr: d("120") }, "{out:?}");
    }

    #[test]
    fn none_quote_blocks_entries() {
        let a = adapter(Ctx::new());
        assert!(is_block(&a.gate(&intent(Side::Buy, 10, false), None)));
    }

    #[test]
    fn drift_beyond_limit_is_drifted_and_within_passes() {
        let a = adapter(Ctx::new());
        // 50 bps of 50 is 0.25: 49.75 is on the limit (pass), 49.70 is beyond.
        let ok = a.gate(&intent(Side::Buy, 10, false), Some(&quote("49.75")));
        let far = a.gate(&intent(Side::Buy, 10, false), Some(&quote("49.70")));
        assert!(matches!(ok, GateOutcome::Pass { .. }), "{ok:?}");
        assert!(matches!(far, GateOutcome::Drifted { .. }), "{far:?}");
    }

    #[test]
    fn reduce_only_without_position_is_blocked_killed_or_not() {
        for killed in [false, true] {
            let mut c = Ctx::new();
            c.killed = killed;
            let a = adapter(c);
            let out = a.gate(&intent(Side::Sell, 5, true), Some(&quote("50")));
            assert!(is_block(&out), "killed={killed}: {out:?}");
        }
    }

    #[test]
    fn reduce_only_that_adds_exposure_is_blocked() {
        let mut c = Ctx::new();
        c.held = vec![long_abc(5)];
        let a = adapter(c);
        assert!(is_block(
            &a.gate(&intent(Side::Buy, 5, true), Some(&quote("50")))
        ));
        assert!(is_block(
            &a.gate(&intent(Side::Sell, 6, true), Some(&quote("50")))
        ));
    }

    #[test]
    fn exit_is_allowed_while_killed_but_entry_is_not() {
        let mut c = Ctx::new();
        c.held = vec![long_abc(5)];
        c.killed = true;
        let a = adapter(c);
        let exit = a.gate(&intent(Side::Sell, 5, true), Some(&quote("50")));
        assert_eq!(
            exit,
            GateOutcome::Pass {
                risk_inr: Decimal::ZERO
            },
            "{exit:?}"
        );
        assert!(is_block(
            &a.gate(&intent(Side::Buy, 10, false), Some(&quote("50")))
        ));
    }

    #[test]
    fn missing_inputs_or_instrument_block() {
        let mut c = Ctx::new();
        c.none = true;
        assert!(is_block(
            &adapter(c).gate(&intent(Side::Buy, 10, false), Some(&quote("50")))
        ));
        let mut c = Ctx::new();
        c.instrument = false;
        assert!(is_block(
            &adapter(c).gate(&intent(Side::Buy, 10, false), Some(&quote("50")))
        ));
    }

    #[test]
    fn mapping_takes_lot_and_tick_from_the_instrument_and_prices_market_at_quote() {
        let mut c = Ctx::new();
        c.lot = 75;
        let inst = c
            .inputs(&intent(Side::Buy, 75, false))
            .unwrap()
            .instrument
            .unwrap();
        let mut i = intent(Side::Buy, 75, false);
        i.kind = OrderKind::Market;
        let r = to_risk_intent(&i, &inst, Some(d("51")), Some(d("45"))).unwrap();
        assert_eq!((r.lot_size, r.tick_size), (75, d("0.05")));
        assert_eq!((r.price, r.stop, r.qty), (d("51"), Some(d("45")), Qty(75)));
        i.reduce_only = true;
        assert_eq!(
            to_risk_intent(&i, &inst, Some(d("51")), Some(d("45")))
                .unwrap()
                .stop,
            None
        );
    }

    #[test]
    fn qty_off_the_lot_is_refused_by_the_gate() {
        let mut c = Ctx::new();
        c.lot = 75;
        let out = adapter(c).gate(&intent(Side::Buy, 10, false), Some(&quote("50")));
        assert!(is_block(&out), "{out:?}");
    }

    #[test]
    fn drift_arithmetic() {
        assert_eq!(drift_bps(d("50"), d("50.5")), Some(d("100")));
        assert_eq!(drift_bps(Decimal::ZERO, d("1")), None);
    }

    fn held_ctx() -> Ctx {
        let mut c = Ctx::new();
        c.held = vec![long_abc(5)];
        c
    }

    #[test]
    fn two_exits_cannot_together_exceed_the_held_quantity() {
        // RISK-134-3: 5 held; one exit of 5 is already pending, a second of 5 (or even 1) blocks.
        let mut c = held_ctx();
        c.pending = Some(Decimal::from(5));
        let a = adapter(c);
        for qty in [5, 1] {
            let out = a.gate(&intent(Side::Sell, qty, true), Some(&quote("50")));
            assert!(is_block(&out), "qty {qty}: {out:?}");
        }
        let mut c = held_ctx();
        c.pending = Some(Decimal::from(4));
        let a = adapter(c);
        assert!(matches!(
            a.gate(&intent(Side::Sell, 1, true), Some(&quote("50"))),
            GateOutcome::Pass { .. }
        ));
        assert!(is_block(
            &a.gate(&intent(Side::Sell, 2, true), Some(&quote("50")))
        ));
    }

    #[test]
    fn unknown_pending_exit_qty_blocks_reduce_only() {
        let mut c = held_ctx();
        c.pending = None;
        let out = adapter(c).gate(&intent(Side::Sell, 1, true), Some(&quote("50")));
        assert!(is_block(&out), "{out:?}");
    }

    #[test]
    fn kill_allow_reduce_only_decides_whether_an_exit_passes_while_killed() {
        for (allow, killed, unknown) in [
            (true, true, false),
            (false, true, false),
            (true, false, true),
            (false, false, true),
        ] {
            let mut c = held_ctx();
            c.killed = killed;
            c.kill_unknown = unknown;
            let a = adapter_with(c, |e| e.kill_allow_reduce_only = allow);
            let out = a.gate(&intent(Side::Sell, 5, true), Some(&quote("50")));
            assert_eq!(
                !is_block(&out),
                allow,
                "allow={allow} killed={killed} unknown={unknown}: {out:?}"
            );
        }
        // Not killed: allowed whatever the key says.
        let a = adapter_with(held_ctx(), |e| e.kill_allow_reduce_only = false);
        assert!(!is_block(
            &a.gate(&intent(Side::Sell, 5, true), Some(&quote("50")))
        ));
    }

    #[test]
    fn unknown_kill_state_blocks_an_entry() {
        let mut c = Ctx::new();
        c.kill_unknown = true;
        assert!(is_block(
            &adapter(c).gate(&intent(Side::Buy, 10, false), Some(&quote("50")))
        ));
    }

    #[test]
    fn quote_age_comes_from_the_fetched_quote() {
        // RISK-134-1: the context says the quote is fresh; the quote itself is 90 s old, or
        // its time does not parse.
        let a = adapter(Ctx::new());
        let mut old = quote("50");
        old.at_ist_rfc3339 = (now() - Duration::seconds(90)).to_rfc3339();
        let out = a.gate(&intent(Side::Buy, 10, false), Some(&old));
        assert!(
            matches!(&out, GateOutcome::Block(r) if r.contains("G15")),
            "{out:?}"
        );
        let mut bad = quote("50");
        bad.at_ist_rfc3339 = "not a time".into();
        let out = a.gate(&intent(Side::Buy, 10, false), Some(&bad));
        assert!(
            matches!(&out, GateOutcome::Block(r) if r.contains("G15")),
            "{out:?}"
        );
    }

    #[test]
    fn entry_without_a_proposed_price_blocks_but_an_exit_does_not_need_one() {
        let mut c = Ctx::new();
        c.proposed = None;
        assert!(is_block(
            &adapter(c).gate(&intent(Side::Buy, 10, false), Some(&quote("50")))
        ));
        let mut c = held_ctx();
        c.proposed = None;
        let out = adapter(c).gate(&intent(Side::Sell, 5, true), Some(&quote("50")));
        assert!(matches!(out, GateOutcome::Pass { .. }), "{out:?}");
    }

    #[test]
    fn non_positive_drift_reference_is_drifted() {
        let mut c = Ctx::new();
        c.proposed = Some(Decimal::ZERO);
        let out = adapter(c).gate(&intent(Side::Buy, 10, false), Some(&quote("50")));
        assert!(matches!(out, GateOutcome::Drifted { .. }), "{out:?}");
    }

    #[test]
    fn drift_applies_to_exits_too() {
        // RISK-134-4: kept on purpose; pinned here.
        let mut c = held_ctx();
        c.proposed = Some(d("50"));
        let out = adapter(c).gate(&intent(Side::Sell, 5, true), Some(&quote("49.70")));
        assert!(matches!(out, GateOutcome::Drifted { .. }), "{out:?}");
    }

    #[test]
    fn limit_exit_with_no_quote_reaches_the_gate_and_passes_with_zero_risk() {
        // TS-134-1: the price is the user's limit; an exit is not trapped by a missing quote.
        let out = adapter(held_ctx()).gate(&intent(Side::Sell, 5, true), None);
        assert_eq!(
            out,
            GateOutcome::Pass {
                risk_inr: Decimal::ZERO
            },
            "{out:?}"
        );
    }

    #[test]
    fn market_exit_with_no_quote_blocks() {
        // TS-134-4.
        let mut i = intent(Side::Sell, 5, true);
        i.kind = OrderKind::Market;
        let out = adapter(held_ctx()).gate(&i, None);
        assert!(is_block(&out), "{out:?}");
    }

    #[test]
    fn over_risk_entry_blocks_with_the_rule_id_in_the_reason() {
        // TS-134-3: far over the per-trade and notional limits.
        let out = adapter(Ctx::new()).gate(&intent(Side::Buy, 1000, false), Some(&quote("50")));
        assert!(
            matches!(&out, GateOutcome::Block(r) if r.contains("G0") || r.contains("G1")),
            "{out:?}"
        );
    }
}
