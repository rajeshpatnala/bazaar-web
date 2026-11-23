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
use crate::types::Evidence;

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

/// What a source might claim; the scanner's own evidence and event facts replace those parts.
fn data() -> DataInputs {
    DataInputs {
        inputs_as_of: Tri::Known(now()),
        evidence: EvidenceInputs {
            oos_trades: Tri::Known(999),
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

fn context(cfg: &GateConfig) -> ScanGateContext {
    ScanGateContext::new(
        snap(),
        limits_of(cfg),
        Box::new(Flat),
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

type DataFn = Box<dyn Fn(&Instrument) -> Result<DataInputs, ContextUnavailable> + Send + Sync>;

/// Counts `context()` calls; builds a fresh context each time.
struct Source {
    calls: AtomicU32,
    make: Box<dyn Fn() -> Result<ScanGateContext, ContextUnavailable> + Send + Sync>,
    data: DataFn,
}
impl GateContextSource for Source {
    fn context(&self) -> Result<ScanGateContext, ContextUnavailable> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.make)()
    }
    fn data_for(&self, i: &Instrument, _h: &Hit) -> Result<DataInputs, ContextUnavailable> {
        (self.data)(i)
    }
}

fn source(
    f: impl Fn() -> Result<ScanGateContext, ContextUnavailable> + Send + Sync + 'static,
) -> Source {
    Source {
        calls: AtomicU32::new(0),
        make: Box::new(f),
        data: Box::new(|_| Ok(data())),
    }
}

fn with_data(
    mut s: Source,
    f: impl Fn(&Instrument) -> Result<DataInputs, ContextUnavailable> + Send + Sync + 'static,
) -> Source {
    s.data = Box::new(f);
    s
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

fn evidence(trades: u32) -> Evidence {
    Evidence {
        trades,
        win_rate: 0.5,
        profit_factor: 2.0,
        avg_r: 0.3,
        max_drawdown_pct: 5.0,
        period: "2021 to 2025".into(),
        costs_applied: true,
        out_of_sample: true,
    }
}

fn with_trades(mut s: Scored, trades: u32) -> Scored {
    s.evidence = Some(evidence(trades));
    s.low_evidence = trades < 30;
    s
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
        low_evidence: false,
        evidence: Some(evidence(50)),
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

fn ok_source() -> Source {
    let g = gate_cfg(TEXT);
    source(move || Ok(context(&g)))
}

#[test]
fn a_clean_idea_is_built_from_the_rounded_levels_and_the_context_is_read_once() {
    let src = ok_source();
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
    let src = ok_source();
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
        let mut c = context(&g);
        let mut l = c.limits().clone();
        l.max_risk_per_trade_pct += Decimal::ONE;
        c = ScanGateContext::new(snap(), l, Box::new(Flat), mode(), BTreeMap::new(), None);
        Ok(c)
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(r.ideas().len(), 0);
    assert_eq!(r.blocked()[0].reason, BlockReason::LimitsChanged);
}

#[test]
fn max_results_applies_after_the_gate_and_blocked_do_not_use_slots() {
    let src = ok_source();
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
        let mut c = context(&g);
        // Unknown snapshot: every rule that needs it fails, hard.
        c = ScanGateContext::new(
            PortfolioSnapshot::unknown(),
            c.limits().clone(),
            Box::new(Flat),
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
            mode(),
            BTreeMap::from([(1, d("500"))]),
            None,
        ))
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(r.ideas().len(), 0);
    let rules = hard_rules(&r);
    let mut rules = rules;
    rules.sort();
    assert_eq!(
        rules,
        vec![RuleId::G05, RuleId::G16, RuleId::G18],
        "no exposure input, kill switch, unknown entries"
    );
}

#[test]
fn a_paper_level_failure_is_overridable_in_paper_but_blocked_in_semi_auto() {
    // 5 out-of-sample trades: G01 (Paper level) fails.
    let paper = run(
        TEXT,
        5,
        &ok_source(),
        &[with_trades(scored("AAA", 1, 80), 5)],
    );
    let idea = &paper.ideas()[0];
    assert_eq!(idea.gate().paper_only, vec![RuleId::G01]);
    assert_eq!(idea.gate().overridable_failures, vec![RuleId::G01]);
    assert!(!idea.gate().passed);

    let semi = TEXT.replace(
        "execution_mode = \"paper\"",
        "execution_mode = \"semi_auto\"",
    );
    let g = gate_cfg(&semi);
    let src = source(move || Ok(context(&g)));
    let r = run(&semi, 5, &src, &[with_trades(scored("AAA", 1, 80), 5)]);
    assert_eq!(r.ideas().len(), 0, "non-actionable outside paper");
    // G16 "approval not on record" fails every semi_auto idea until slice 8 (the scan never
    // supplies an approval); G01 moved from paper-only to hard outside paper.
    let mut rules = hard_rules(&r);
    rules.sort();
    assert_eq!(rules, vec![RuleId::G01, RuleId::G16]);
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
            mode(),
            BTreeMap::new(),
            None,
        ))
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    let rules = hard_rules(&r);
    assert!(rules.contains(&RuleId::G10), "{rules:?}");
}

fn paper_only_of(r: &ScanResult, i: usize) -> Vec<RuleId> {
    r.ideas()[i].gate().paper_only.clone()
}

#[test]
fn evidence_is_judged_per_idea_not_taken_from_the_source() {
    // The source claims 999 trades for everyone; only the idea's own evidence counts.
    let items = [scored("AAA", 1, 80), with_trades(scored("BBB", 2, 70), 5)];
    let r = run(TEXT, 5, &ok_source(), &items);
    assert_eq!(r.ideas().len(), 2, "{:?}", r.blocked());
    assert!(paper_only_of(&r, 0).is_empty(), "50 trades pass G01");
    assert_eq!(paper_only_of(&r, 1), vec![RuleId::G01], "5 trades fail G01");
}

#[test]
fn missing_or_unusable_evidence_fails_g01_and_blocks() {
    let mut none = scored("AAA", 1, 80);
    none.evidence = None;
    let mut in_sample = scored("BBB", 2, 70);
    in_sample.evidence.as_mut().unwrap().out_of_sample = false;
    let r = run(TEXT, 5, &ok_source(), &[none, in_sample]);
    assert_eq!(r.ideas().len(), 0);
    assert_eq!(r.blocked().len(), 2);
    for b in r.blocked() {
        assert!(
            matches!(&b.reason, BlockReason::GateHardFail(v) if v.contains(&RuleId::G01)),
            "{b:?}"
        );
    }
}

#[test]
fn an_event_warning_fails_g07_soft_and_unchecked_events_block() {
    let mut near = scored("AAA", 1, 80);
    near.warnings = vec![Warning::EventNearby("results tomorrow".into())];
    let clean = scored("BBB", 2, 70);
    let r = run(TEXT, 5, &ok_source(), &[near, clean]);
    assert_eq!(r.ideas().len(), 2, "{:?}", r.blocked());
    assert_eq!(r.ideas()[0].gate().overridable_failures, vec![RuleId::G07]);
    assert!(!r.ideas()[0].gate().passed);
    assert_eq!(
        r.ideas()[1].gate().overridable_failures,
        Vec::<RuleId>::new()
    );

    let mut unchecked = scored("CCC", 3, 60);
    unchecked.warnings = vec![Warning::EventsNotChecked];
    let r = run(TEXT, 5, &ok_source(), &[unchecked]);
    assert_eq!(r.ideas().len(), 0);
    assert!(hard_rules(&r).contains(&RuleId::G07), "{:?}", r.blocked());
}

#[test]
fn the_ban_factor_reports_what_the_gate_saw() {
    let ban_of = |r: &ScanResult| {
        r.ideas()[0]
            .why()
            .iter()
            .find_map(|f| match f {
                WhyFactor::Ban { checked, list_date } => Some((*checked, list_date.clone())),
                _ => None,
            })
            .unwrap()
    };
    let r = run(TEXT, 5, &ok_source(), &[scored("AAA", 1, 80)]);
    assert_eq!(ban_of(&r), (false, None), "an Unknown list is not checked");
    let src = with_data(ok_source(), |_| {
        let mut d = data();
        d.ban_list = Tri::Known(bazaar_risk::rules::inputs::BanList {
            banned_underlyings: vec![],
            fetched_at: now(),
        });
        Ok(d)
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(ban_of(&r), (true, Some("2026-11-19".to_owned())));
}

#[test]
fn a_data_failure_blocks_only_that_idea() {
    let src = with_data(ok_source(), |i| {
        if i.tradingsymbol == "BBB" {
            Err(ContextUnavailable("quote store down".into()))
        } else {
            Ok(data())
        }
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80), scored("BBB", 2, 70)]);
    assert_eq!(r.ideas().len(), 1);
    assert_eq!(
        r.blocked()[0].reason,
        BlockReason::GateInputsUnavailable("quote store down".into())
    );
}

#[derive(Debug)]
struct BySide;
impl CostModel for BySide {
    fn cost(&self, s: Side, _q: u32, p: Decimal) -> Tri<Decimal> {
        match s {
            Side::Buy => Tri::Known(d("30")),
            // The sell leg is priced at the rounded stop, not the entry.
            Side::Sell if p == d("489.95") => Tri::Known(d("70")),
            Side::Sell => Tri::Unknown,
        }
    }
}

fn by_side_source(available: Tri<Decimal>) -> Source {
    let g = gate_cfg(TEXT);
    let src = source(move || {
        Ok(ScanGateContext::new(
            snap(),
            limits_of(&g),
            Box::new(BySide),
            mode(),
            BTreeMap::from([(1, d("500"))]),
            Some(exposure()),
        ))
    });
    with_data(src, move |_| {
        let mut x = data();
        x.margin.available = available;
        Ok(x)
    })
}

#[test]
fn risk_includes_both_legs_and_x03_pins_the_margin_arithmetic() {
    let items = [scored("AAA", 1, 80)];
    let big = run(TEXT, 5, &by_side_source(Tri::Known(d("250000"))), &items);
    assert_eq!(big.ideas().len(), 1, "{:?}", big.blocked());
    let a = &big.ideas()[0];
    let qty = Decimal::from(a.size().qty);
    // QUAN-165-1: allowance = buy 30 + sell 70 (the stop-out exit at the rounded stop).
    assert_eq!(a.risk_rupees(), qty * d("10.10") + d("100"));
    let required = qty * d("500.05") + d("100");

    // X03 passes at exactly the need and blocks one paisa below it.
    let eq = run(TEXT, 5, &by_side_source(Tri::Known(required)), &items);
    assert_eq!(eq.ideas().len(), 1, "{:?}", eq.blocked());
    let low = run(
        TEXT,
        5,
        &by_side_source(Tri::Known(required - d("0.01"))),
        &items,
    );
    assert_eq!(low.ideas().len(), 0);
    assert_eq!(hard_rules(&low), vec![RuleId::X03]);
    let unk = run(TEXT, 5, &by_side_source(Tri::Unknown), &items);
    assert_eq!(hard_rules(&unk), vec![RuleId::X03]);

    // The Why carries the same money as the recommendation.
    let w = a.why();
    assert!(
        w.iter()
            .any(|f| matches!(f, WhyFactor::Risk { risk_rupees, qty: q, .. }
        if *risk_rupees == a.risk_rupees() && *q == a.size().qty))
    );
    assert!(
        w.iter()
            .any(|f| matches!(f, WhyFactor::Reward { target, .. } if *target == a.targets()[0]))
    );
    assert!(w.iter().any(|f| matches!(
        f,
        WhyFactor::Confluence {
            families_total: 4,
            families_agreeing: 1,
            ..
        }
    )));
    assert_eq!(a.data_as_of(), now() - chrono::Duration::hours(20));
}

#[test]
fn a_missing_exposure_input_blocks_on_g05() {
    let g = gate_cfg(TEXT);
    let src = source(move || {
        Ok(ScanGateContext::new(
            snap(),
            limits_of(&g),
            Box::new(Flat),
            mode(),
            BTreeMap::from([(1, d("500"))]),
            None,
        ))
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(hard_rules(&r), vec![RuleId::G05]);
}

#[test]
fn a_bad_families_total_blocks_instead_of_being_clamped() {
    let src = ok_source();
    let gc = gate_cfg(TEXT);
    let gate = Gate::new(&gc, &Store, &At);
    let sc = scanner_cfg(5);
    let cx = AssembleCx {
        cfg: &sc,
        gate: &gate,
        source: &src,
        now: now(),
        families_total: 0,
    };
    let r = assemble(&[scored("AAA", 1, 80)], vec![], &cx);
    assert!(
        matches!(&r.blocked()[0].reason, BlockReason::Sizing(m) if m.contains("families_total"))
    );
    // Two agreeing families against a total of one.
    let mut two = scored("AAA", 1, 80);
    let mut second = two.candidate.hits[0].clone();
    second.family = "momentum".into();
    two.candidate.hits.push(second);
    let cx = AssembleCx {
        cfg: &sc,
        gate: &gate,
        source: &src,
        now: now(),
        families_total: 1,
    };
    let r = assemble(&[two], vec![], &cx);
    assert!(
        matches!(&r.blocked()[0].reason, BlockReason::Sizing(m) if m.contains("families_total"))
    );
}

#[test]
fn an_idea_that_cannot_be_sized_or_has_no_signal_is_blocked_as_sizing() {
    // A stop above the entry cannot be sized.
    let mut bad = scored("AAA", 1, 80);
    bad.candidate.hits[0].signal.stop = d("510");
    let mut none = scored("BBB", 2, 70);
    none.candidate.hits.clear();
    let r = run(TEXT, 5, &ok_source(), &[bad, none]);
    assert_eq!(r.ideas().len(), 0);
    assert_eq!(r.blocked().len(), 2);
    assert!(
        matches!(&r.blocked()[1].reason, BlockReason::Sizing(m) if m == "the candidate has no signal")
    );
    assert!(
        !matches!(&r.blocked()[0].reason, BlockReason::GateHardFail(_)),
        "{:?}",
        r.blocked()[0]
    );
}

#[test]
fn both_stamps_use_the_scan_clock() {
    let r = run(TEXT, 5, &ok_source(), &[scored("AAA", 1, 80)]);
    assert_eq!(
        r.ideas()[0].generated_at(),
        r.ideas()[0].gate().evaluated_at
    );
}

#[test]
fn a_source_results_day_is_not_hidden_by_a_scanner_without_warnings() {
    let src = with_data(ok_source(), |_| {
        let mut d = data();
        d.events.results_window = Tri::Known(true);
        Ok(d)
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(r.ideas().len(), 1, "{:?}", r.blocked());
    assert_eq!(r.ideas()[0].gate().overridable_failures, vec![RuleId::G07]);
    assert!(!r.ideas()[0].gate().passed);
    let src = with_data(ok_source(), |_| {
        let mut d = data();
        d.events.policy_day = Tri::Known(true);
        Ok(d)
    });
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    assert_eq!(r.ideas()[0].gate().overridable_failures, vec![RuleId::G07]);
}

#[test]
fn long_source_errors_are_bounded_in_the_block_reason() {
    let src = with_data(ok_source(), |_| Err(ContextUnavailable("x".repeat(10_000))));
    let r = run(TEXT, 5, &src, &[scored("AAA", 1, 80)]);
    let BlockReason::GateInputsUnavailable(m) = &r.blocked()[0].reason else {
        unreachable!("{:?}", r.blocked()[0]);
    };
    assert_eq!(m.chars().count(), MAX_REASON_CHARS + 3);
}

struct E2eEvidence;
impl crate::ports::EvidenceSource for E2eEvidence {
    fn for_instrument(&self, _: &str, _: u64, _: DateTime<Tz>) -> Option<Evidence> {
        Some(evidence(50))
    }
    fn for_strategy(&self, _: &str, _: DateTime<Tz>) -> Option<Evidence> {
        Some(evidence(50))
    }
}

/// A real (non-empty, no rows) calendar: absence of events is then a checked fact.
struct E2eEvents;
impl crate::ports::EventCalendar for E2eEvents {
    fn upcoming(&self, _: &str, _: chrono::NaiveDate, _: u32) -> Vec<bazaar_core::config::Event> {
        Vec::new()
    }
    fn is_empty(&self) -> bool {
        false
    }
}

#[test]
fn fixture_scan_to_gate_to_why_end_to_end() {
    use crate::runner::tests as rt;
    let seen = rt::Seen::default();
    let env = rt::env();
    let mut strategies = vec![rt::fake("a", 2, bazaar_regime::Trend::Up, &seen)];
    // Costs (`Flat`, Rs 20) and evidence (`E2eEvidence`) are stubs: the cost model and the real
    // evidence stats are covered in the risk and backtest crates, not here.
    let mut uni = [
        rt::entry("AAA", rt::rising(rt::N, 1_000_000)),
        rt::entry("BBB", rt::rising(rt::N, 1_000_000)),
    ];
    // A distinct token, so the price map (token 1 only) would expose a token mix-up.
    uni[1].instrument.instrument_token = 2;
    let out = rt::scan(
        &env,
        &rt::fresh_ban(&["BBB"]),
        &mut strategies,
        &uni,
        &rt::rising(rt::N, 1),
    )
    .unwrap();
    // The banned name is dropped with a reason, never scored.
    assert_eq!(out.candidates.len(), 1);
    assert!(
        out.dropped
            .iter()
            .any(|(s, r)| s == "BBB" && *r == crate::universe::DropReason::Banned),
        "{:?}",
        out.dropped
    );
    // No look-ahead: the strategy saw bars up to the decision bar and none later.
    let decision = rt::day(rt::N - 1).date_naive();
    {
        let seen = seen.lock().unwrap();
        assert!(!seen.is_empty());
        assert!(seen.iter().all(|(_, day)| *day <= decision), "{seen:?}");
        assert_eq!(seen.last().map(|(_, day)| *day), Some(decision));
    }

    let sc = scanner_cfg(5);
    let bar = rt::day(rt::N - 1);
    let sx = crate::score::ScoreContext {
        cfg: &sc,
        evidence: &E2eEvidence,
        events: &E2eEvents,
        decision_date: bar.date_naive(),
        as_of: bar,
        families_total: out.families_total,
    };
    let ranked = crate::score::score_and_rank(out.candidates, &sx);
    let g = gate_cfg(TEXT);
    let src = source(move || {
        Ok(ScanGateContext::new(
            snap(),
            limits_of(&g),
            Box::new(Flat),
            mode(),
            BTreeMap::from([(1, d("101"))]),
            Some(exposure()),
        ))
    });
    let gc = gate_cfg(TEXT);
    let gate = Gate::new(&gc, &Store, &At);
    let cx = AssembleCx {
        cfg: &sc,
        gate: &gate,
        source: &src,
        now: now(),
        families_total: out.families_total,
    };
    let result = assemble(&ranked, vec![], &cx);
    assert_eq!(result.ideas().len(), 1, "{:?}", result.blocked());
    let rec = &result.ideas()[0];
    // Levels are the signal's, already on the 0.05 tick; the gate saw this idea's own evidence.
    assert_eq!(rec.entry_zone(), (d("100"), d("102")));
    assert_eq!(rec.stop_loss(), d("94"));
    assert_eq!(rec.targets(), &[d("110")]);
    assert_eq!(rec.gate().hard_failures, Vec::<RuleId>::new());
    assert_eq!(rec.gate().overridable_failures, Vec::<RuleId>::new());
    assert!(rec.gate().passed);
    assert_eq!(rec.generated_at(), rec.gate().evaluated_at);

    let lines = crate::why::render(rec);
    let text = lines.join("\n");
    // Sizing: within the 1% risk budget of Rs 1,00,000 (risk/share 8 at the worst fill, lot 1).
    let qty = rec.size().qty;
    assert!(qty > 0 && Decimal::from(qty) * d("8") <= d("1000"), "{qty}");
    assert_eq!(rec.size().lots, qty);
    assert!(text.contains(&format!("Quantity {qty}")), "{text}");
    assert!(text.contains("Target Rs 110."), "{text}");
    assert!(text.contains("The risk-rule checks passed"), "{text}");
    assert!(
        text.contains("Educational tool, not investment advice"),
        "{text}"
    );
}
