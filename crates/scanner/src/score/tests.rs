use std::sync::Mutex;

use bazaar_core::config::{self, Event, EventKind, StrategiesConfig};
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_regime::{Fact, RegimeState};
use bazaar_strategies::{ExitPlan, Signal};
use chrono::TimeZone;
use chrono_tz::Asia::Kolkata;

use super::*;

fn cfg() -> ScannerConfig {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let c: StrategiesConfig = config::load(&dir).unwrap();
    c.scanner.unwrap()
}

fn at() -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2025, 11, 14, 15, 30, 0).unwrap()
}

fn date() -> NaiveDate {
    at().date_naive()
}

fn fact() -> Fact {
    Fact::new("fit", "x", "y")
}

fn hit(id: &'static str, family: &str, penalised: bool) -> Hit {
    let d = Decimal::from;
    Hit {
        family: family.into(),
        signal: Signal {
            strategy_id: id,
            instrument_token: 1,
            strategy_version: 1,
            params_hash: "h".into(),
            signal_bar_time: at(),
            entry_low: d(100),
            entry_high: d(102),
            stop: d(94),
            targets: vec![d(110)],
            risk_per_share: d(8),
            reference_rr: d(1),
            exit_plan: ExitPlan {
                time_stop_bars: 10,
                rule_text: "t".into(),
            },
            facts: vec![],
            valid_until: at(),
        },
        fit: if penalised {
            Fit::Penalised(fact())
        } else {
            Fit::Allowed(fact())
        },
        penalty_weight: 0.5,
        merged_ids: vec![],
    }
}

/// Median traded value in multiples of the floor (Rs 10 crore).
fn cand(sym: &str, token: u64, hits: Vec<Hit>, floor_multiple: i64) -> Candidate {
    let r = RegimeState::unknown(at(), fact());
    Candidate {
        instrument: Instrument {
            instrument_token: token,
            exchange_token: token,
            tradingsymbol: sym.into(),
            name: sym.into(),
            exchange: Exchange::parse("NSE"),
            instrument_type: InstrumentType::parse("EQ"),
            segment: "NSE".into(),
            expiry: None,
            strike: None,
            tick_size: Decimal::new(5, 2),
            lot_size: 1,
        },
        hits,
        market_regime: r.clone(),
        stock_regime: r,
        median_traded_value: Decimal::from(100_000_000_i64 * floor_multiple),
    }
}

fn ev(trades: u32, pf: f64) -> Evidence {
    Evidence {
        trades,
        win_rate: 0.5,
        profit_factor: pf,
        avg_r: 0.3,
        max_drawdown_pct: 10.0,
        period: "p".into(),
        costs_applied: true,
        out_of_sample: true,
    }
}

#[derive(Default)]
struct FakeEvidence {
    /// (symbol-token, evidence) per instrument; strategy-wide stats.
    own: Vec<(u64, Evidence)>,
    wide: Option<Evidence>,
    asked: Mutex<Vec<DateTime<Tz>>>,
}

impl EvidenceSource for FakeEvidence {
    fn for_instrument(&self, _: &str, token: u64, as_of: DateTime<Tz>) -> Option<Evidence> {
        self.asked.lock().unwrap().push(as_of);
        self.own
            .iter()
            .find(|(t, _)| *t == token)
            .map(|(_, e)| e.clone())
    }
    fn for_strategy(&self, _: &str, as_of: DateTime<Tz>) -> Option<Evidence> {
        self.asked.lock().unwrap().push(as_of);
        self.wide.clone()
    }
}

#[derive(Default)]
struct FakeEvents {
    rows: Vec<Event>,
    empty: bool,
}

impl EventCalendar for FakeEvents {
    fn upcoming(&self, sym: &str, from: NaiveDate, days: u32) -> Vec<Event> {
        self.rows
            .iter()
            .filter(|e| e.symbol.is_empty() || e.symbol == sym)
            .filter(|e| e.date >= from && (e.date - from).num_days() <= i64::from(days))
            .cloned()
            .collect()
    }
    fn is_empty(&self) -> bool {
        self.empty
    }
}

fn run(cands: Vec<Candidate>, evidence: &FakeEvidence, events: &FakeEvents) -> Vec<Scored> {
    let c = cfg();
    let cx = ScoreContext {
        cfg: &c,
        evidence,
        events,
        decision_date: date(),
        as_of: at(),
        families_total: 3,
    };
    score_and_rank(cands, &cx)
}

fn names(v: &[Scored]) -> Vec<&str> {
    v.iter()
        .map(|s| s.candidate.instrument.tradingsymbol.as_str())
        .collect()
}

#[test]
fn liquidity_is_zero_at_floor_half_at_root_ten_one_at_ten_times() {
    let c = cfg();
    let l = |m: i64| liquidity_component(Decimal::from(100_000_000_i64) * Decimal::from(m), &c);
    assert!(l(1).abs() < 1e-12);
    assert!((l(10) - 1.0).abs() < 1e-12);
    assert!(
        (l(100) - 1.0).abs() < 1e-12,
        "clamped above the full-score multiple"
    );
    // ln(3.1623)/ln(10) = 0.5
    let half = liquidity_component(Decimal::new(316_227_766, 0), &c);
    assert!((half - 0.5).abs() < 1e-6, "got {half}");
    assert!(liquidity_component(Decimal::from(5_000_000_i64), &c).abs() < 1e-12);
}

#[test]
fn evidence_shrinks_toward_neutral_prior_when_no_strategy_wide_stats() {
    let c = cfg(); // k = 30, min trades 30
    // n = 30: weight 0.5; pf 2.0 shrunk to 1.5; quality 0.5.
    let q = evidence_component(Some(&ev(30, 2.0)), None, &c);
    assert!((q - 0.5).abs() < 1e-12, "got {q}");
    // n = 90: weight 0.75; pf 2.0 -> 1.75; quality 0.75.
    let q = evidence_component(Some(&ev(90, 2.0)), None, &c);
    assert!((q - 0.75).abs() < 1e-12, "got {q}");
}

#[test]
fn evidence_shrinks_toward_strategy_wide_only_with_enough_trades() {
    let c = cfg();
    let thin_wide = ev(10, 2.0);
    let q = evidence_component(Some(&ev(30, 1.0)), Some(&thin_wide), &c);
    assert!(
        q.abs() < 1e-12,
        "thin wide stats must not lift the prior, got {q}"
    );
    let wide = ev(200, 2.0);
    let q = evidence_component(Some(&ev(30, 1.0)), Some(&wide), &c);
    assert!((q - 0.5).abs() < 1e-12, "got {q}"); // 0.5 x 1.0 + 0.5 x 2.0 = 1.5
}

#[test]
fn no_evidence_is_the_floor_and_low_evidence_and_capped() {
    let r = run(
        vec![cand("AAA", 1, vec![hit("a", "breakout", false)], 10)],
        &FakeEvidence::default(),
        &FakeEvents::default(),
    );
    let s = &r[0];
    assert!(s.components.evidence.abs() < 1e-12);
    assert!(s.low_evidence);
    assert!(s.evidence.is_none());
    assert!(s.score <= cfg().low_evidence_cap);
    assert!(s.warnings.contains(&Warning::LowEvidence));
}

#[test]
fn in_sample_or_gross_evidence_counts_as_none() {
    let c = cfg();
    let mut e = ev(100, 3.0);
    e.out_of_sample = false;
    assert!(evidence_component(Some(&e), None, &c).abs() < 1e-12);
    assert!(!proven(Some(&e), &c));
    let mut g = ev(100, 3.0);
    g.costs_applied = false;
    assert!(!proven(Some(&g), &c));
    assert!(proven(Some(&ev(30, 1.2)), &c));
    assert!(!proven(Some(&ev(29, 1.2)), &c));
}

#[test]
fn low_evidence_name_never_outranks_a_provisional_one() {
    // LOW: three families agree, huge liquidity, no evidence. PROV: one family, thin liquidity,
    // 30 trades at a modest profit factor.
    let low = cand(
        "AAA",
        1,
        vec![
            hit("a", "breakout", false),
            hit("b", "trend", false),
            hit("c", "reversion", false),
        ],
        100,
    );
    let prov = cand("ZZZ", 2, vec![hit("d", "trend", false)], 1);
    let evidence = FakeEvidence {
        own: vec![(2, ev(100, 2.0))],
        ..FakeEvidence::default()
    };
    let r = run(vec![low, prov], &evidence, &FakeEvents::default());
    let low_row = r
        .iter()
        .find(|s| s.candidate.instrument.instrument_token == 1)
        .unwrap();
    assert!(
        low_row.raw_score > low_row.score,
        "the cap must bite in this fixture"
    );
    assert_eq!(names(&r), vec!["ZZZ", "AAA"]);
}

#[test]
fn up_high_ranks_below_up_normal_when_otherwise_equal() {
    // "AAA" sorts first alphabetically, so only the penalty can put it second.
    let weak = cand("AAA", 1, vec![hit("a", "breakout", true)], 10);
    let strong = cand("BBB", 2, vec![hit("a", "breakout", false)], 10);
    let r = run(
        vec![weak, strong],
        &FakeEvidence::default(),
        &FakeEvents::default(),
    );
    assert_eq!(names(&r), vec!["BBB", "AAA"]);
    assert!(r[0].raw_score > r[1].raw_score);
    assert!((r[1].components.regime_fit - 0.5).abs() < 1e-12);
}

#[test]
fn ties_break_by_symbol_then_token_whatever_the_input_order() {
    let mk = |s: &str, t: u64| cand(s, t, vec![hit("a", "trend", false)], 10);
    let base = vec![mk("CCC", 3), mk("AAA", 1), mk("BBB", 2), mk("AAA", 0)];
    let ev = FakeEvidence::default();
    let events = FakeEvents::default();
    let first = run(base.clone(), &ev, &events);
    let order: Vec<(String, u64)> = first
        .iter()
        .map(|s| {
            (
                s.candidate.instrument.tradingsymbol.clone(),
                s.candidate.instrument.instrument_token,
            )
        })
        .collect();
    assert_eq!(
        order,
        vec![
            ("AAA".into(), 0),
            ("AAA".into(), 1),
            ("BBB".into(), 2),
            ("CCC".into(), 3)
        ]
    );
    let mut rev = base;
    rev.reverse();
    assert_eq!(
        run(rev, &ev, &events),
        first,
        "identical output for any input order"
    );
}

#[test]
fn golden_ranks_on_the_fixture() {
    // Pinned scores with the proposal weights (0.30/0.25/0.15/0.30): confluence 1/3 per family,
    // regime 1 or 0.5, liquidity log scale, evidence capped at 55 when unproven.
    let evidence = FakeEvidence {
        own: vec![(1, ev(60, 2.0)), (2, ev(30, 1.6))],
        ..FakeEvidence::default()
    };
    let cands = vec![
        cand(
            "ALPHA",
            1,
            vec![hit("a", "breakout", false), hit("b", "trend", false)],
            10,
        ),
        cand("BETA", 2, vec![hit("b", "trend", false)], 10),
        cand("GAMMA", 3, vec![hit("c", "reversion", true)], 10),
    ];
    let r = run(
        cands,
        &evidence,
        &FakeEvents {
            empty: false,
            ..FakeEvents::default()
        },
    );
    // ALPHA: conf 2/3, regime 1, liq 1, evid q(0.667*2+0.333*1=1.667)=0.667 -> 100*(0.2+0.25+0.15+0.2)=80.
    // BETA:  conf 1/3, regime 1, liq 1, evid q(0.5*1.6+0.5=1.3)=0.3 -> 100*(0.1+0.25+0.15+0.09)=59 (proven).
    // GAMMA: conf 1/3, regime .5, liq 1, no evid -> 100*(0.1+0.125+0.15)=37.5 -> 38.
    assert_eq!(names(&r), vec!["ALPHA", "BETA", "GAMMA"]);
    assert_eq!(
        r.iter().map(|s| s.score).collect::<Vec<_>>(),
        vec![80, 59, 38]
    );
    assert!(!r[0].low_evidence && !r[1].low_evidence && r[2].low_evidence);
}

#[test]
fn empty_calendar_warns_and_gives_no_bonus_or_penalty() {
    let events = FakeEvents {
        empty: true,
        ..FakeEvents::default()
    };
    let r = run(
        vec![cand("AAA", 1, vec![hit("a", "trend", false)], 10)],
        &FakeEvidence::default(),
        &events,
    );
    assert!(r[0].warnings.contains(&Warning::EventsNotChecked));
    assert!(r[0].components.event_penalty.abs() < 1e-12);
}

fn event(days: i64, symbol: &str) -> Event {
    Event {
        date: date() + chrono::Duration::days(days),
        kind: EventKind::Results,
        symbol: symbol.into(),
        note: "q".into(),
        source: "s".into(),
    }
}

#[test]
fn nearby_event_subtracts_points_scaled_by_proximity() {
    let max = cfg().event_penalty_max_points;
    let run_with = |rows: Vec<Event>| {
        run(
            vec![cand("AAA", 1, vec![hit("a", "trend", false)], 10)],
            &FakeEvidence::default(),
            &FakeEvents { rows, empty: false },
        )
    };
    let none = run_with(vec![]);
    let today = run_with(vec![event(0, "AAA")]);
    let later = run_with(vec![event(3, "AAA")]);
    let far = run_with(vec![event(30, "AAA")]);
    let other = run_with(vec![event(0, "ZZZ")]);
    assert!((today[0].components.event_penalty - max).abs() < 1e-9);
    assert!((later[0].components.event_penalty - max * (1.0 - 3.0 / 6.0)).abs() < 1e-9);
    assert!(far[0].components.event_penalty.abs() < 1e-12);
    assert!(other[0].components.event_penalty.abs() < 1e-12);
    assert!(today[0].raw_score < none[0].raw_score);
    assert!(
        today[0]
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::EventNearby(_)))
    );
    assert!(!none[0].warnings.contains(&Warning::EventsNotChecked));
}

#[test]
fn score_stays_within_zero_and_hundred_and_never_nan() {
    assert_eq!(to_score(f64::NAN), 0);
    assert_eq!(to_score(-5.0), 0);
    assert_eq!(to_score(250.0), 100);
    assert_eq!(to_score(57.5), 58);
}

#[test]
fn lookahead_sources_are_asked_only_as_of_the_decision_bar() {
    let evidence = FakeEvidence {
        own: vec![(1, ev(60, 2.0))],
        ..FakeEvidence::default()
    };
    let _ = run(
        vec![cand("AAA", 1, vec![hit("a", "trend", false)], 10)],
        &evidence,
        &FakeEvents::default(),
    );
    let asked = evidence.asked.lock().unwrap();
    assert!(!asked.is_empty());
    assert!(asked.iter().all(|t| *t == at()));
}

#[test]
fn proven_evidence_is_not_capped() {
    let evidence = FakeEvidence {
        own: vec![(1, ev(200, 3.0))],
        ..FakeEvidence::default()
    };
    let r = run(
        vec![cand(
            "AAA",
            1,
            vec![
                hit("a", "breakout", false),
                hit("b", "trend", false),
                hit("c", "reversion", false),
            ],
            100,
        )],
        &evidence,
        &FakeEvents::default(),
    );
    assert!(!r[0].low_evidence);
    assert_eq!(r[0].score, r[0].raw_score);
    assert!(r[0].score > cfg().low_evidence_cap);
}
