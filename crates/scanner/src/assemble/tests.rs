use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use bazaar_core::clock::Clock;
use bazaar_core::config::{self, StrategiesConfig};
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_regime::{Fact, RegimeState};
use bazaar_risk::baseline::{BaselineError, LimitsBaseline};
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_risk::gate::{CostModel, ModeEvidence};
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::verdict::RuleId;
use bazaar_strategies::{ExitPlan, Signal};
use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Kolkata;

use super::*;
use crate::gate_ctx::ContextUnavailable;
use crate::result::ScanVerdict;
use crate::runner::Candidate;
use crate::score::Components;

const TEXT: &str = include_str!("../../../../fixtures/config/risk.toml");

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
struct Flat;
impl CostModel for Flat {
    fn cost(&self, _s: Side, _q: u32, _p: Decimal) -> Tri<Decimal> {
        Tri::Known(d("20"))
    }
}

fn snap() -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.as_of = Tri::Known(now());
    s.positions = Tri::Known(vec![]);
    s.consecutive_losses = Tri::Known(0);
    s.day_pnl = Tri::Known(Decimal::ZERO);
    s.week_pnl = Tri::Known(Decimal::ZERO);
    s.week_start = Tri::Known(NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
    s.entries_today = Tri::Known(0);
    s.orders_last_minute = Tri::Known(0);
    s.orders_today = Tri::Known(0);
    s.overrides_today = Tri::Known(0);
    s.overrides_week = Tri::Known(0);
    s.kill_switch = Tri::Known(false);
    s.charges_buffer = Tri::Known(d("20"));
    // Small enough that the 20% position cap stays inside the Rs 20,000 order-notional limit (G16).
    s.starting_cash = Tri::Known(d("100000"));
    s.realised_pnl = Tri::Known(Decimal::ZERO);
    s.equity_mtm = Tri::Known(d("100000"));
    s.marks = Tri::Known(vec![]);
    s.last_losing_exits = Tri::Known(vec![]);
    s
}

fn data(oos: u32) -> DataInputs {
    DataInputs {
        inputs_as_of: Tri::Known(now()),
        evidence: EvidenceInputs {
            oos_trades: Tri::Known(oos),
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
            qty: 0,
            required: Tri::Unknown,
            available: Tri::Known(d("250000")),
        },
    }
}

fn mode() -> ModeEvidence {
    ModeEvidence {
        approval: None,
        compliance_fresh: Tri::Known(false),
        readiness_report: Tri::Known(false),
        owner_opt_in: Tri::Known(false),
        live: Tri::Known(false),
    }
}

fn gate_cfg(text: &str) -> GateConfig {
    GateConfig::from_toml_str(text).unwrap()
}

/// The limits the gate will use: built with `risk::baseline::rule_ctx`, as the composition does.
fn limits_of(cfg: &GateConfig) -> bazaar_risk::config::EffectiveLimits {
    bazaar_risk::baseline::rule_ctx(cfg, &Store, now())
        .unwrap()
        .limits
}

fn context(cfg: &GateConfig, oos: u32) -> ScanGateContext {
    ScanGateContext::new(
        snap(),
        limits_of(cfg),
        Box::new(Flat),
        data(oos),
        mode(),
        BTreeMap::from([(1, d("500")), (2, d("500")), (3, d("500"))]),
        Some(exposure()),
    )
}

fn exposure() -> ExposureInputs {
    let end = now().date_naive();
    let series: Vec<(NaiveDate, Decimal)> = (0..100u32)
        .map(|i| {
            let day = end - chrono::Duration::days(i64::from(99 - i));
            (day, Decimal::new(i64::from(i % 7) - 3, 3))
        })
        .collect();
    let syms = ["AAA", "BBB", "CCC"];
    ExposureInputs {
        sectors: syms
            .iter()
            .map(|s| ((*s).to_owned(), "IT".to_owned()))
            .collect(),
        returns: syms
            .iter()
            .map(|s| ((*s).to_owned(), series.clone()))
            .collect(),
    }
}

/// Counts `context()` calls; builds a fresh context each time.
struct Source {
    calls: AtomicU32,
    make: Box<dyn Fn() -> Result<ScanGateContext, ContextUnavailable> + Send + Sync>,
}
impl GateContextSource for Source {
    fn context(&self) -> Result<ScanGateContext, ContextUnavailable> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.make)()
    }
}

fn source(
    f: impl Fn() -> Result<ScanGateContext, ContextUnavailable> + Send + Sync + 'static,
) -> Source {
    Source {
        calls: AtomicU32::new(0),
        make: Box::new(f),
    }
}

fn scanner_cfg(max_results: u32) -> ScannerConfig {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let c: StrategiesConfig = config::load(&dir).unwrap();
    let mut s = c.scanner.unwrap();
    s.max_results = max_results;
    s
}

fn fact() -> Fact {
    Fact::new("fit", "x", "y")
}

fn scored(sym: &str, token: u64, score: u8) -> Scored {
    let hit = Hit {
        family: "trend".into(),
        signal: Signal {
            strategy_id: "s1",
            instrument_token: token,
            strategy_version: 1,
            params_hash: "h".into(),
            signal_bar_time: now() - chrono::Duration::hours(20),
            entry_low: d("499.98"),
            entry_high: d("500.02"),
            stop: d("489.97"),
            targets: vec![d("520.03")],
            risk_per_share: d("10.05"),
            reference_rr: d("2"),
            exit_plan: ExitPlan {
                time_stop_bars: 10,
                rule_text: "t".into(),
            },
            facts: vec![],
            valid_until: now(),
        },
        fit: bazaar_regime::Fit::Allowed(fact()),
        penalty_weight: 0.5,
        merged_ids: vec![],
    };
    let r = RegimeState::unknown(now(), fact());
    Scored {
        candidate: Candidate {
            instrument: Instrument {
                instrument_token: token,
                exchange_token: token,
                tradingsymbol: sym.into(),
                name: sym.into(),
                exchange: Exchange::Nse,
                instrument_type: InstrumentType::Equity,
                segment: "NSE".into(),
                expiry: None,
                strike: None,
                tick_size: d("0.05"),
                lot_size: 1,
            },
            hits: vec![hit],
            market_regime: r.clone(),
            stock_regime: r,
            median_traded_value: d("1000000000"),
        },
        components: Components {
            confluence: 0.5,
            regime_fit: 1.0,
            liquidity: 1.0,
            evidence: 0.0,
            event_penalty: 0.0,
        },
        raw_score: score,
        score,
        low_evidence: true,
        evidence: None,
        warnings: vec![],
    }
}

fn run(cfg_text: &str, max_results: u32, src: &Source, items: &[Scored]) -> ScanResult {
    let gc = gate_cfg(cfg_text);
    let gate = Gate::new(&gc, &Store, &At);
    let sc = scanner_cfg(max_results);
    let cx = AssembleCx {
        cfg: &sc,
        gate: &gate,
        source: src,
        now: now(),
        families_total: 4,
    };
    assemble(items, vec![], &cx)
}

/// The rules of the first blocked idea when it is a hard gate failure, else empty.
fn hard_rules(r: &ScanResult) -> Vec<RuleId> {
    match r.blocked().first().map(|b| &b.reason) {
        Some(BlockReason::GateHardFail(v)) => v.clone(),
        _ => Vec::new(),
    }
}

fn ok_source(oos: u32) -> Source {
    let g = gate_cfg(TEXT);
    source(move || Ok(context(&g, oos)))
}

#[test]
fn a_clean_idea_is_built_from_the_rounded_levels_and_the_context_is_read_once() {
    let src = ok_source(50);
    let items = [
        scored("AAA", 1, 80),
        scored("BBB", 2, 70),
        scored("CCC", 3, 60),
    ];
    let r = run(TEXT, 5, &src, &items);
    assert_eq!(src.calls.load(Ordering::SeqCst), 1);
    assert_eq!(r.ideas().len(), 3, "{:?}", r.blocked());
    let a = &r.ideas()[0];
    // RISK-163-1: the zone, stop and target are the tick-rounded levels, not the signal's.
    assert_eq!(a.entry_zone(), (d("500"), d("500.05")));
    assert_eq!(a.stop_loss(), d("489.95"));
    assert_eq!(a.targets(), &[d("520")]);
    assert!(a.gate().passed);
    assert_eq!(a.gate().evaluated_at, now());
    assert_eq!(a.generated_at(), now());
    assert!(a.size().qty > 0);
    assert_eq!(a.reward_risk(), d("19.95") / d("10.10"));
    assert!(a.risk_rupees() >= Decimal::from(a.size().qty) * d("10.10"));
    assert!(a.gate().hard_failures.is_empty() && a.gate().paper_only.is_empty());
    assert_eq!(*r.verdict(), ScanVerdict::Setups { count: 3 });
}

#[test]
fn an_unavailable_context_blocks_every_idea_in_rank_order() {
    let src = source(|| Err(ContextUnavailable("snapshot store down".into())));
    let items = [scored("AAA", 1, 80), scored("BBB", 2, 70)];
    let r = run(TEXT, 5, &src, &items);
    assert_eq!(src.calls.load(Ordering::SeqCst), 1);
    assert_eq!(r.ideas().len(), 0);
    let syms: Vec<&str> = r
        .blocked()
        .iter()
        .map(|b| b.instrument.tradingsymbol.as_str())
        .collect();
    assert_eq!(syms, ["AAA", "BBB"]);
    assert!(
        r.blocked()
            .iter()
            .all(|b| b.reason == BlockReason::GateInputsUnavailable("snapshot store down".into()))
    );
    // The levels are shown as the strategy gave them.
    assert_eq!(r.blocked()[0].entry_zone, (d("499.98"), d("500.02")));
    assert!(
        matches!(r.verdict(), ScanVerdict::NoHighQualitySetups { reasons } if reasons.len() == 2)
    );
}

#[test]
fn a_gate_reject_is_a_block_reason_not_a_drop() {
    let src = ok_source(50);
    let items = [scored("abc", 1, 80), scored("BBB", 2, 70)];
    let r = run(TEXT, 5, &src, &items);
    assert_eq!(r.ideas().len(), 1);
    assert_eq!(r.blocked().len(), 1);
    assert!(matches!(&r.blocked()[0].reason, BlockReason::GateReject(m) if m.contains("abc")));
}

#[test]
fn limits_that_differ_from_the_gates_block_the_idea() {
    let g = gate_cfg(TEXT);
    let src = source(move || {
        let mut c = context(&g, 50);
        let mut l = c.limits().clone();
        l.max_risk_per_trade_pct += Decimal::ONE;
        c = ScanGateContext::new(
            snap(),
            l,
            Box::new(Flat),
            data(50),
            mode(),
            BTreeMap::new(),
            None,
        );
        Ok(c)
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(r.ideas().len(), 0);
    assert_eq!(r.blocked()[0].reason, BlockReason::LimitsChanged);
}

#[test]
fn max_results_applies_after_the_gate_and_blocked_do_not_use_slots() {
    let src = ok_source(50);
    // Rank 1 is non-canonical (gate reject); ranks 2 and 3 are clean.
    let items = [
        scored("abc", 1, 90),
        scored("BBB", 2, 80),
        scored("CCC", 3, 70),
    ];
    let r = run(TEXT, 1, &src, &items);
    assert_eq!(r.ideas().len(), 1);
    assert_eq!(r.ideas()[0].instrument().tradingsymbol, "BBB");
    assert_eq!(r.blocked().len(), 1);
    assert_eq!(r.blocked()[0].instrument.tradingsymbol, "abc");
}

#[test]
fn blocked_ideas_keep_their_rank_position_among_each_other() {
    let g = gate_cfg(TEXT);
    let src = source(move || {
        let mut c = context(&g, 50);
        // Unknown snapshot: every rule that needs it fails, hard.
        c = ScanGateContext::new(
            PortfolioSnapshot::unknown(),
            c.limits().clone(),
            Box::new(Flat),
            data(50),
            mode(),
            BTreeMap::new(),
            None,
        );
        Ok(c)
    });
    let items = [
        scored("AAA", 1, 80),
        scored("abc", 2, 70),
        scored("CCC", 3, 60),
    ];
    let r = run(TEXT, 5, &src, &items);
    assert_eq!(r.ideas().len(), 0);
    let syms: Vec<&str> = r
        .blocked()
        .iter()
        .map(|b| b.instrument.tradingsymbol.as_str())
        .collect();
    assert_eq!(syms, ["AAA", "abc", "CCC"]);
}

#[test]
fn a_hard_failure_blocks_and_lists_every_failing_rule() {
    let g = gate_cfg(TEXT);
    let src = source(move || {
        let mut s = snap();
        s.kill_switch = Tri::Known(true);
        s.entries_today = Tri::Unknown;
        Ok(ScanGateContext::new(
            s,
            limits_of(&g),
            Box::new(Flat),
            data(50),
            mode(),
            BTreeMap::from([(1, d("500"))]),
            None,
        ))
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(r.ideas().len(), 0);
    let rules = hard_rules(&r);
    assert!(rules.contains(&RuleId::G18), "{rules:?}");
    assert!(rules.len() >= 2, "every failing rule is listed: {rules:?}");
}

#[test]
fn a_paper_level_failure_is_overridable_in_paper_but_blocked_in_semi_auto() {
    // 5 out-of-sample trades: G01 (Paper level) fails.
    let paper = run(TEXT, 5, &ok_source(5), &[scored("AAA", 1, 80)]);
    let idea = &paper.ideas()[0];
    assert_eq!(idea.gate().paper_only, vec![RuleId::G01]);
    assert_eq!(idea.gate().overridable_failures, vec![RuleId::G01]);
    assert!(!idea.gate().passed);

    let semi = TEXT.replace(
        "execution_mode = \"paper\"",
        "execution_mode = \"semi_auto\"",
    );
    let g = gate_cfg(&semi);
    let src = source(move || Ok(context(&g, 5)));
    let r = run(&semi, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(r.ideas().len(), 0, "non-actionable outside paper");
    let rules = hard_rules(&r);
    assert!(rules.contains(&RuleId::G01), "{rules:?}");
}

#[test]
fn the_context_never_carries_an_approval() {
    let g = gate_cfg(TEXT);
    let mut m = mode();
    m.approval = Some(bazaar_risk::override_::IntentDigest::of(&OrderIntent {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty: Qty(1),
        price: d("1"),
        stop: Some(d("0.5")),
        new_stop: None,
        lot_size: 1,
        tick_size: d("0.05"),
    }));
    let c = ScanGateContext::new(
        snap(),
        limits_of(&g),
        Box::new(Flat),
        data(50),
        m,
        BTreeMap::new(),
        None,
    );
    assert!(c.mode().approval.is_none());
}

#[test]
fn an_unknown_reference_price_fails_g10_closed() {
    let g = gate_cfg(TEXT);
    let src = source(move || {
        Ok(ScanGateContext::new(
            snap(),
            limits_of(&g),
            Box::new(Flat),
            data(50),
            mode(),
            BTreeMap::new(),
            None,
        ))
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    let rules = hard_rules(&r);
    assert!(rules.contains(&RuleId::G10), "{rules:?}");
}
