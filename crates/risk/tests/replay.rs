//! Replay of the scenarios from the discipline skill through the public gate API only, under the
//! shipped `config/risk.toml` (1% per trade of Rs 1,00,000 = Rs 1,000; 3% day; 6% week; two
//! losses pause entries; 60 minute re-entry lockout). Gate state lives in the snapshot, so a
//! "restart" is a new `Gate` over the same snapshot.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_core::clock::Clock;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::money::Qty;
use bazaar_risk::baseline::{BaselineError, LimitsBaseline};
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_risk::gate::{CostModel, Evaluation, Gate, GateInputs, ModeEvidence};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::{LosingExit, Mark, PortfolioSnapshot, Position};
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::RuleId;
use chrono::{DateTime, Duration, NaiveDate, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

const SHIPPED: &str = include_str!("../../../config/risk.toml");

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// Thursday 2026-11-19 11:00 IST; its Monday is 2026-11-16.
fn thursday() -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap()
}

struct At(DateTime<Tz>);
impl Clock for At {
    fn now_ist(&self) -> DateTime<Tz> {
        self.0
    }
}

struct Store;
impl LimitsBaseline for Store {
    fn read(&self) -> Result<Baseline, BaselineError> {
        Ok(Baseline::none())
    }
}

/// Fake F9 cost: a flat Rs 20 (below the shipped Rs 60 floor, so the floor applies).
#[derive(Debug)]
struct Flat;
impl CostModel for Flat {
    fn cost(&self, _s: Side, _q: u32, _p: Decimal) -> Tri<Decimal> {
        Tri::Known(d("20"))
    }
}

fn instrument(lot: u32) -> Instrument {
    Instrument {
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
        lot_size: lot,
    }
}

/// Entry at Rs 100 with a stop Rs 10 away on the protective side (90 for a buy, 110 for a sell).
fn intent(side: Side, qty: u32, lot: u32) -> OrderIntent {
    OrderIntent {
        symbol: "ABC".into(),
        side,
        qty: Qty(qty),
        price: d("100"),
        stop: Some(if side == Side::Buy { d("90") } else { d("110") }),
        new_stop: None,
        lot_size: lot,
        tick_size: d("0.05"),
    }
}

fn snap(at: DateTime<Tz>, monday: (i32, u32, u32)) -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.as_of = Tri::Known(at);
    s.positions = Tri::Known(vec![]);
    s.consecutive_losses = Tri::Known(0);
    s.day_pnl = Tri::Known(Decimal::ZERO);
    s.week_pnl = Tri::Known(Decimal::ZERO);
    s.week_start = Tri::Known(NaiveDate::from_ymd_opt(monday.0, monday.1, monday.2).unwrap());
    s.entries_today = Tri::Known(0);
    s.orders_last_minute = Tri::Known(0);
    s.orders_today = Tri::Known(0);
    s.overrides_today = Tri::Known(0);
    s.overrides_week = Tri::Known(0);
    s.kill_switch = Tri::Known(false);
    s.charges_buffer = Tri::Known(d("60"));
    s.starting_cash = Tri::Known(d("100000"));
    s.realised_pnl = Tri::Known(Decimal::ZERO);
    s.equity_mtm = Tri::Known(d("100000"));
    s.marks = Tri::Known(vec![]);
    s.last_losing_exits = Tri::Known(vec![]);
    s
}

fn held_long(qty: u32) -> Position {
    Position {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty,
        stop: Tri::Known(d("90")),
        risk_price: Tri::Known(d("100")),
        avg_cost: Tri::Known(d("100")),
    }
}

/// One fresh `Gate` per call, as after a process restart: nothing is carried but the snapshot.
fn eval(i: &OrderIntent, s: &PortfolioSnapshot, at: DateTime<Tz>) -> Evaluation {
    let cfg = GateConfig::from_toml_str(SHIPPED).unwrap();
    let inst = instrument(i.lot_size);
    let quote = at;
    // The margin need is worked out for the opening part of the order (X03 checks the quantity).
    let held: u32 = s
        .positions
        .known()
        .map(|v| v.iter().filter(|p| p.symbol == i.symbol).map(|p| p.qty).sum())
        .unwrap_or(0);
    let entry_qty = if i.side == Side::Sell {
        i.qty.0.saturating_sub(held)
    } else {
        i.qty.0
    };
    let data = DataInputs {
        inputs_as_of: Tri::Known(quote),
        evidence: EvidenceInputs {
            oos_trades: Tri::Known(50),
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
        quote_at: Tri::Known(quote),
        session_open: Tri::Known(true),
        margin: MarginInputs {
            qty: entry_qty,
            required: Tri::Known(d("5000")),
            available: Tri::Known(d("100000")),
        },
    };
    let mode = ModeEvidence {
        approval: None,
        compliance_fresh: Tri::Known(false),
        readiness_report: Tri::Known(false),
        owner_opt_in: Tri::Known(false),
        live: Tri::Known(false),
    };
    let series: Vec<(NaiveDate, Decimal)> = (0..100u32)
        .map(|n| {
            let day = at.date_naive() - Duration::days(i64::from(99 - n));
            (day, Decimal::new(i64::from(n % 7) - 3, 3))
        })
        .collect();
    let exposure = ExposureInputs {
        sectors: BTreeMap::from([("ABC".to_string(), "IT".to_string())]),
        returns: BTreeMap::from([("ABC".to_string(), series)]),
    };
    let inputs = GateInputs {
        cost: &Flat,
        data: &data,
        instrument: Some(&inst),
        defined_max_loss: None,
        mode: &mode,
        entry_zone: Tri::Known(EntryZone {
            low: d("99"),
            high: d("101"),
        }),
        live_price: Tri::Known(d("100")),
        exposure: Some(&exposure),
    };
    Gate::new(&cfg, &Store, &At(at))
        .evaluate(i, s, &inputs)
        .unwrap()
}

fn failed(e: &Evaluation) -> Vec<RuleId> {
    e.verdict()
        .failures()
        .map(bazaar_risk::verdict::RuleResult::rule)
        .collect()
}

#[test]
fn the_clean_baseline_passes_every_rule() {
    let e = eval(
        &intent(Side::Buy, 50, 1),
        &snap(thursday(), (2026, 11, 16)),
        thursday(),
    );
    assert_eq!(failed(&e), vec![], "{:?}", e.verdict().results());
}

#[test]
fn two_losses_pause_entries_until_the_session_ends_but_exits_pass() {
    let mut s = snap(thursday(), (2026, 11, 16));
    s.positions = Tri::Known(vec![held_long(10)]);
    for (losses, paused) in [(0, false), (1, false), (2, true), (3, true)] {
        s.consecutive_losses = Tri::Known(losses);
        let entry = failed(&eval(&intent(Side::Buy, 5, 1), &s, thursday()));
        assert_eq!(
            entry.contains(&RuleId::G08),
            paused,
            "{losses} losses: {entry:?}"
        );
        let exit = failed(&eval(&intent(Side::Sell, 10, 1), &s, thursday()));
        assert!(
            !exit.contains(&RuleId::G08),
            "an exit is never paused: {exit:?}"
        );
    }
    // To the session end: still paused later the same session, released next session once the
    // snapshot's loss counter is reset for the new day.
    s.consecutive_losses = Tri::Known(2);
    let later = thursday() + Duration::hours(4);
    let mut same = s.clone();
    same.as_of = Tri::Known(later);
    assert!(failed(&eval(&intent(Side::Buy, 5, 1), &same, later)).contains(&RuleId::G08));
    let next = Kolkata.with_ymd_and_hms(2026, 11, 20, 10, 0, 0).unwrap();
    let mut fresh = snap(next, (2026, 11, 16));
    fresh.positions = s.positions.clone();
    fresh.marks = s.marks.clone();
    assert!(!failed(&eval(&intent(Side::Buy, 5, 1), &fresh, next)).contains(&RuleId::G08));
    // Unknown run of losses fails closed.
    s.consecutive_losses = Tri::Unknown;
    assert!(failed(&eval(&intent(Side::Buy, 5, 1), &s, thursday())).contains(&RuleId::G08));
}

#[test]
fn a_gap_down_through_the_stop_stops_new_entries_but_never_the_exit() {
    // 100 shares, stop Rs 90 (planned loss Rs 1,000). The open gaps to Rs 50: Rs 5,000 lost, not
    // the Rs 1,000 the stop promised. The day figure is the real one; equity is the lower of start + realised and the mark-to-market
    // value, capped at starting cash (Rs 95,000 here), so the day cap is 3% of that = Rs 2,850.
    let mut s = snap(thursday(), (2026, 11, 16));
    s.positions = Tri::Known(vec![held_long(100)]);
    s.marks = Tri::Known(vec![Mark {
        symbol: "ABC".into(),
        price: d("50"),
    }]);
    s.day_pnl = Tri::Known(d("-5000"));
    s.week_pnl = Tri::Known(d("-5000"));
    s.equity_mtm = Tri::Known(d("95000"));
    // Note: buying more ABC at Rs 100 against a Rs 50 mark is also an averaging-down add, so G11
    // fires too; this test asserts only on G03 and G19.
    let entry = failed(&eval(&intent(Side::Buy, 10, 1), &s, thursday()));
    assert!(entry.contains(&RuleId::G03), "{entry:?}");
    assert!(
        !entry.contains(&RuleId::G19),
        "Rs 5,000 is under the 6% week cap: {entry:?}"
    );
    let exit = failed(&eval(&intent(Side::Sell, 100, 1), &s, thursday()));
    assert!(!exit.contains(&RuleId::G03), "{exit:?}");
    // A gap that stays inside the stop does not trip the day cap.
    s.day_pnl = Tri::Known(d("-1000"));
    s.equity_mtm = Tri::Known(d("99000"));
    assert!(!failed(&eval(&intent(Side::Buy, 10, 1), &s, thursday())).contains(&RuleId::G03));
}

#[test]
fn the_day_cap_boundary_is_replayed_against_the_shipped_three_percent() {
    // A journal-shaped snapshot: the loss is realised, so equity (the lower of start + realised
    // and mark-to-market, capped at starting cash) is Rs 1,00,000 less the loss. The cap is 3% of
    // that, so it bites at 103 x loss >= 3,00,000: Rs 2,912 passes, Rs 2,913 blocks.
    let mut s = snap(thursday(), (2026, 11, 16));
    for (loss, blocked) in [("2912", false), ("2913", true), ("3000", true)] {
        s.day_pnl = Tri::Known(-d(loss));
        s.realised_pnl = Tri::Known(-d(loss));
        s.equity_mtm = Tri::Known(d("100000") - d(loss));
        let f = failed(&eval(&intent(Side::Buy, 1, 1), &s, thursday()));
        assert_eq!(f.contains(&RuleId::G03), blocked, "{loss}: {f:?}");
    }
}

#[test]
fn a_restart_in_the_middle_of_a_lockout_keeps_the_lockout() {
    let exit_at = thursday() - Duration::minutes(30);
    let mut s = snap(thursday(), (2026, 11, 16));
    s.last_losing_exits = Tri::Known(vec![LosingExit {
        symbol: "ABC".into(),
        at: exit_at,
    }]);
    let i = intent(Side::Buy, 10, 1);
    // Determinism check: two fresh gates over the same snapshot agree. The lockout comes only from
    // the snapshot (`Store` returns an empty baseline), so nothing here proves absence of hidden state.
    let (a, b) = (eval(&i, &s, thursday()), eval(&i, &s, thursday()));
    assert_eq!(failed(&a), vec![RuleId::G17]);
    assert_eq!(a.verdict(), b.verdict());
    // 59 minutes in: still locked; at exactly 60 it opens.
    let at = |m: i64| exit_at + Duration::minutes(m);
    let moved = |m: i64| {
        let mut s2 = s.clone();
        s2.as_of = Tri::Known(at(m));
        s2
    };
    assert!(failed(&eval(&i, &moved(59), at(59))).contains(&RuleId::G17));
    assert!(!failed(&eval(&i, &moved(60), at(60))).contains(&RuleId::G17));
    // A fresh losing exit in another symbol does not lock ABC.
    let mut elsewhere = s.clone();
    elsewhere.last_losing_exits = Tri::Known(vec![LosingExit {
        symbol: "XYZ".into(),
        at: thursday() - Duration::minutes(5),
    }]);
    assert!(!failed(&eval(&i, &elsewhere, thursday())).contains(&RuleId::G17));
    // A journal that did not come back after the restart fails closed.
    s.last_losing_exits = Tri::Unknown;
    assert!(failed(&eval(&i, &s, thursday())).contains(&RuleId::G17));
}

#[test]
fn the_week_cap_holds_until_monday_and_a_stale_week_start_fails_closed() {
    let friday = Kolkata.with_ymd_and_hms(2026, 11, 20, 14, 0, 0).unwrap();
    let mut s = snap(friday, (2026, 11, 16));
    s.week_pnl = Tri::Known(d("-6000"));
    s.equity_mtm = Tri::Known(d("94000"));
    let i = intent(Side::Buy, 10, 1);
    assert!(failed(&eval(&i, &s, friday)).contains(&RuleId::G19));
    // Monday 2026-11-23: a fresh week starts from zero and the entry passes.
    let monday = Kolkata.with_ymd_and_hms(2026, 11, 23, 10, 0, 0).unwrap();
    let fresh = snap(monday, (2026, 11, 23));
    assert_eq!(failed(&eval(&i, &fresh, monday)), vec![]);
    // The old week's start (and its loss) carried into Monday is not trusted: G19 rejects.
    let mut stale = snap(monday, (2026, 11, 16));
    stale.week_pnl = Tri::Known(d("-6000"));
    assert!(failed(&eval(&i, &stale, monday)).contains(&RuleId::G19));
    stale.week_pnl = Tri::Known(Decimal::ZERO);
    assert!(failed(&eval(&i, &stale, monday)).contains(&RuleId::G19));
}

#[test]
fn a_one_lot_trade_above_the_rs_1000_budget_is_refused_not_rounded_up() {
    // Rs 10 a share plus the Rs 60 charges floor against a Rs 1,000 budget.
    let s = snap(thursday(), (2026, 11, 16));
    // 1 lot of 100 = Rs 1,000 + Rs 60: over the budget, so G02 fails and there is no smaller lot.
    let over = failed(&eval(&intent(Side::Buy, 100, 100), &s, thursday()));
    assert!(over.contains(&RuleId::G02), "{over:?}");
    // 1 lot of 50 = Rs 500 + Rs 60 fits.
    let fits = failed(&eval(&intent(Side::Buy, 50, 50), &s, thursday()));
    assert!(!fits.contains(&RuleId::G02), "{fits:?}");
    // The share boundary on lot size 1: 94 x 10 + 60 = 1,000 passes, 95 x 10 + 60 fails.
    assert!(!failed(&eval(&intent(Side::Buy, 94, 1), &s, thursday())).contains(&RuleId::G02));
    assert!(failed(&eval(&intent(Side::Buy, 95, 1), &s, thursday())).contains(&RuleId::G02));
}

#[test]
fn a_sell_that_closes_and_reverses_is_an_entry_and_hits_the_lockout() {
    // Long 10 ABC, a losing exit 10 minutes ago in the same name, and two losses today. A Sell of
    // 30 closes 10 and opens a 20 share short: the 20 is a new entry, so G17 and G08 apply.
    let mut s = snap(thursday(), (2026, 11, 16));
    s.positions = Tri::Known(vec![held_long(10)]);
    s.consecutive_losses = Tri::Known(2);
    s.marks = Tri::Known(vec![Mark {
        symbol: "ABC".into(),
        price: d("100"),
    }]);
    s.last_losing_exits = Tri::Known(vec![LosingExit {
        symbol: "ABC".into(),
        at: thursday() - Duration::minutes(10),
    }]);
    let disguised = failed(&eval(&intent(Side::Sell, 30, 1), &s, thursday()));
    assert!(disguised.contains(&RuleId::G17), "{disguised:?}");
    assert!(disguised.contains(&RuleId::G08), "{disguised:?}");
    // Exactly the lockout and the pause fail. (The fixture works the margin need out for the 20
    // share entry part, as X03 requires, and marks the held ABC, as G05 requires.)
    let mut got = disguised.clone();
    got.sort();
    let mut want = vec![RuleId::G08, RuleId::G17];
    want.sort();
    assert_eq!(got, want, "{disguised:?}");
    // The honest exit of exactly the 10 held is never blocked by either rule.
    let honest = failed(&eval(&intent(Side::Sell, 10, 1), &s, thursday()));
    assert!(
        !honest.contains(&RuleId::G17) && !honest.contains(&RuleId::G08),
        "{honest:?}"
    );
}
