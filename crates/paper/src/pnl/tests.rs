use super::*;
use crate::cost_pool::replay::MAX_LINE_AMOUNT;

// 2026-11-20 is a Friday, 2026-11-23 a Monday.
const LAST_FRI: &str = "2026-11-20T10:00:00+05:30";
const MON: &str = "2026-11-23T10:00:00+05:30";
const TUE: &str = "2026-11-24T10:00:00+05:30";

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal literal")
}

fn day(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, n).expect("date")
}

fn id(n: u32) -> PaperSessionId {
    PaperSessionId::new(n).expect("session")
}

fn line(session: u32, ts: &'static str, kind: &'static str, amount: &str) -> ReplayLine<'static> {
    ReplayLine {
        session,
        kind,
        amount: d(amount),
        ref_id: Some(1),
        ts_ist_rfc3339: ts,
        symbol: None,
        qty: None,
        product: None,
    }
}

fn opening(session: u32, ts: &'static str, amount: &str) -> Vec<ReplayLine<'static>> {
    vec![line(session, ts, "opening", amount)]
}

/// A charge and its CNC trade, as the account writes them.
fn pair(
    session: u32,
    ts: &'static str,
    kind: &'static str,
    symbol: &'static str,
    qty: u32,
    amount: &str,
    charge: &str,
) -> Vec<ReplayLine<'static>> {
    let trade = ReplayLine {
        symbol: Some(symbol),
        qty: Some(qty),
        product: Some("cnc"),
        ..line(session, ts, kind, amount)
    };
    vec![line(session, ts, "charge", charge), trade]
}

/// Marks: INFY at `infy`, TCS far above any cost here, nothing else.
fn marks(infy: &'static str) -> impl Fn(&str) -> Option<Decimal> {
    move |symbol| match symbol {
        "INFY" => Some(d(infy)),
        "TCS" => Some(d("1000")),
        _ => None,
    }
}

fn figures(
    lines: &[ReplayLine<'_>],
    today: u32,
    active: u32,
    infy: &'static str,
) -> Result<SharedFigures, PnlError> {
    let sessions = session_figures(lines, day(today), id(active), marks(infy))?;
    shared_figures(&sessions, d("100000"))
}

fn window(pnl: &str, base: &str) -> WindowPnl {
    WindowPnl {
        pnl: d(pnl),
        equity_base: d(base),
    }
}

fn shared(day: WindowPnl, week: WindowPnl) -> SharedFigures {
    SharedFigures { day, week }
}

/// A window with lines in it.
fn traded(realised: &str, open_loss: &str, equity: &str) -> SessionWindow {
    SessionWindow {
        realised: d(realised),
        open_loss: d(open_loss),
        equity: d(equity),
        has_lines: true,
    }
}

fn both_windows(w: SessionWindow) -> SessionFigures {
    SessionFigures { day: w, week: w }
}

#[test]
fn a_day_two_lower_sale_is_a_loss_not_its_proceeds() {
    let lines = [
        opening(1, MON, "100000"),
        pair(1, MON, "buy", "INFY", 10, "-1000", "-10"),
        pair(1, TUE, "sell", "INFY", 10, "900", "-6"),
    ]
    .concat();
    let f = figures(&lines, 24, 1, "1").expect("figures");
    // Day base: cash 98990 plus share cost 1000, without the 10 of unsold charges.
    assert_eq!(f.day, window("-116", "99990"));
    // The session began inside the week: its opening alone, and the opening is no P&L.
    assert_eq!(f.week, window("-116", "100000"));
}

#[test]
fn unsold_buy_charges_count_at_an_unchanged_mark() {
    let lines = [
        opening(1, TUE, "100000"),
        pair(1, TUE, "buy", "INFY", 10, "-1000", "-10"),
    ]
    .concat();
    let f = figures(&lines, 24, 1, "100").expect("figures");
    assert_eq!(f.day, window("-10", "100000"));
    assert_eq!(f.week, window("-10", "100000"));
}

#[test]
fn owner_worked_case_and_a_gain_offsets_no_sale_or_other_pool() {
    let lines = [
        opening(1, TUE, "100000"),
        pair(1, TUE, "buy", "INFY", 10, "-1000", "-10"),
        pair(1, TUE, "buy", "INFY", 10, "-800", "-10"),
        pair(1, TUE, "sell", "INFY", 10, "900", "-6"),
    ]
    .concat();
    let both = |pnl| Ok(shared(window(pnl, "100000"), window(pnl, "100000")));
    assert_eq!(figures(&lines, 24, 1, "90"), both("-26"));
    assert_eq!(figures(&lines, 24, 1, "100"), both("-16"));
    // A winning TCS pool changes neither the realised -16 nor INFY's open -10.
    let with_winner = [lines, pair(1, TUE, "buy", "TCS", 1, "-100", "-1")].concat();
    assert_eq!(figures(&with_winner, 24, 1, "90"), both("-26"));
}

#[test]
fn the_open_losses_of_all_held_pools_add_up() {
    let mut mis = pair(1, TUE, "buy", "INFY", 5, "-500", "-5");
    mis[1].product = Some("mis");
    let lines = [
        opening(1, TUE, "100000"),
        pair(1, TUE, "buy", "INFY", 10, "-1000", "-10"),
        mis,
        pair(1, TUE, "buy", "TCS", 1, "-1500", "-1"),
    ]
    .concat();
    // -110 (INFY CNC), -55 (INFY MIS), -501 (TCS at 1000).
    let s = session_figures(&lines, day(24), id(1), marks("90")).expect("figures");
    let held = traded("0", "-666", "100000");
    assert_eq!(s.get(id(1)), Some(&both_windows(held)));
    assert_eq!(
        shared_figures(&s, d("100000")),
        Ok(shared(window("-666", "100000"), window("-666", "100000")))
    );
}

/// SEC-202-3: with two bad pools, the first in (session, symbol, product) order is reported.
#[test]
fn several_bad_pools_always_give_the_same_error() {
    let lines = [
        opening(1, TUE, "100000"),
        pair(1, TUE, "buy", "INFY", 1, "-100", "-1"),
        pair(1, TUE, "buy", "ACC", 1, "-100", "-1"),
    ]
    .concat();
    // ACC has no mark, INFY a bad one. Repeated because a map's order differs per replay.
    for _ in 0..64 {
        assert_eq!(figures(&lines, 24, 1, "0"), Err(PnlError::MissingMark));
    }
}

#[test]
fn a_carried_open_loss_counts_every_day_and_old_sales_do_not() {
    let lines = [
        opening(1, LAST_FRI, "100000"),
        pair(1, LAST_FRI, "buy", "INFY", 10, "-1000", "-10"),
        pair(1, LAST_FRI, "sell", "INFY", 5, "400", "-3"),
    ]
    .concat();
    let f = figures(&lines, 24, 1, "90").expect("figures");
    // Rest: 450 against 500 + 5. Base: cash 99387 + share cost 500. No line this week.
    assert_eq!(f.day, window("-55", "99887"));
    assert_eq!(f.week, window("-55", "99887"));
}

#[test]
fn the_week_starts_on_the_ist_monday() {
    let lines = [
        opening(1, LAST_FRI, "100000"),
        pair(1, LAST_FRI, "buy", "INFY", 10, "-1000", "-10"),
        pair(1, MON, "sell", "INFY", 5, "400", "-3"),
        pair(1, TUE, "sell", "INFY", 5, "450", "-2"),
    ]
    .concat();
    let f = figures(&lines, 24, 1, "1").expect("figures");
    // Monday: 397 - 505 = -108. Tuesday: 448 - 505 = -57. Week base: cash 98990 + cost 1000.
    assert_eq!(f.week, window("-165", "99990"));
    // Day base: cash 99387 + the 500 of cost still held on Monday night.
    assert_eq!(f.day, window("-57", "99887"));
}

#[test]
fn the_week_boundary_is_ist_midnight_into_monday() {
    let lines = [
        opening(1, LAST_FRI, "100000"),
        pair(1, LAST_FRI, "buy", "INFY", 10, "-1000", "-10"),
        // Sunday's last second and Monday's first.
        pair(
            1,
            "2026-11-22T23:59:59+05:30",
            "sell",
            "INFY",
            5,
            "400",
            "-3",
        ),
        pair(
            1,
            "2026-11-23T00:00:00+05:30",
            "sell",
            "INFY",
            5,
            "450",
            "-2",
        ),
    ]
    .concat();
    let on = |today| figures(&lines, today, 1, "1").expect("figures");
    // Today is Monday: the week is the day, and Sunday's sale is outside both.
    assert_eq!(
        on(23),
        shared(window("-57", "99887"), window("-57", "99887"))
    );
    // Tuesday, and the following Sunday: the week still starts on Monday the 23rd.
    for today in [24, 29] {
        assert_eq!(
            on(today),
            shared(window("0", "99835"), window("-57", "99887"))
        );
    }
}

/// P#1 loses 5016 on `ts`; P#2 is then opened on Tuesday with the full starting cash.
fn loss_then_new_session(ts: &'static str) -> Vec<ReplayLine<'static>> {
    [
        opening(1, LAST_FRI, "100000"),
        pair(1, ts, "buy", "INFY", 100, "-10000", "-10"),
        pair(1, ts, "sell", "INFY", 100, "5000", "-6"),
        opening(2, TUE, "100000"),
    ]
    .concat()
}

#[test]
fn a_loss_today_follows_into_a_new_session() {
    assert_eq!(
        figures(&loss_then_new_session(TUE), 24, 2, "1"),
        Ok(shared(window("-5016", "100000"), window("-5016", "100000")))
    );
}

/// NOT a safety property: this pins P1 as written, under which the day base RISES from 94,984
/// to 100,000 on switching. Hard condition on slice 4 (RISK-202-1, owner decision; see the F10
/// record, Deferred). If the formula changes, the first assertion changes with it.
#[test]
fn p1_literal_base_drops_an_idle_session_after_an_earlier_day_loss() {
    let lines = loss_then_new_session(MON);
    let week = window("-5016", "100000");
    assert_eq!(
        figures(&lines, 24, 2, "1"),
        Ok(shared(window("0", "100000"), week))
    );
    // While P#1 is active its own equity is the day base.
    assert_eq!(
        figures(&lines, 24, 1, "1"),
        Ok(shared(window("0", "94984"), week))
    );
}

#[test]
fn the_base_is_the_smallest_equity_of_active_and_traded_sessions() {
    let lines = [
        opening(1, LAST_FRI, "100000"),
        pair(1, LAST_FRI, "buy", "INFY", 100, "-10000", "-10"),
        pair(1, LAST_FRI, "sell", "INFY", 100, "5000", "-6"),
        opening(2, LAST_FRI, "100000"),
        pair(1, TUE, "buy", "INFY", 1, "-100", "-1"),
        pair(1, TUE, "sell", "INFY", 1, "100", "-1"),
    ]
    .concat();
    let f = figures(&lines, 24, 2, "1").expect("figures");
    assert_eq!(f.day, window("-2", "94984"));
    assert_eq!(f.week, window("-2", "94984"));
    // The starting cash caps a session that grew past it.
    let rich = opening(1, LAST_FRI, "250000");
    let f = figures(&rich, 24, 1, "1").expect("figures");
    assert_eq!(f, shared(window("0", "100000"), window("0", "100000")));
}

#[test]
fn session_figures_are_kept_apart() {
    // P#1 lost on Monday; P#2 sells half of today's buy and holds the rest.
    let lines = [
        loss_then_new_session(MON),
        pair(2, TUE, "buy", "INFY", 10, "-1000", "-10"),
        pair(2, TUE, "sell", "INFY", 5, "400", "-3"),
    ]
    .concat();
    let s = session_figures(&lines, day(24), id(2), marks("90")).expect("figures");
    let one = SessionFigures {
        day: SessionWindow {
            equity: d("94984"),
            ..SessionWindow::default()
        },
        week: traded("-5016", "0", "100000"),
    };
    assert_eq!(s.get(id(1)), Some(&one));
    let two = traded("-108", "-55", "100000");
    assert_eq!(s.get(id(2)), Some(&both_windows(two)));
    assert_eq!(s.get(id(3)), None);
}

#[test]
fn a_replay_failure_is_passed_on_never_zero() {
    let mut lines = [
        opening(1, TUE, "100000"),
        pair(1, TUE, "sell", "INFY", 1, "100", "-1"),
    ]
    .concat();
    let refused = |l: &[ReplayLine<'_>]| figures(l, 24, 1, "100");
    let at = |fault| Err(PnlError::Replay(ReplayError { line: 2, fault }));
    assert_eq!(
        refused(&lines),
        at(LineFault::Pool(PoolError::SellAboveHeld))
    );
    lines[2].symbol = None;
    assert_eq!(refused(&lines), at(LineFault::MissingSymbol));
    lines[2].ts_ist_rfc3339 = "2026-11-25T10:00:00+05:30";
    assert_eq!(refused(&lines), at(LineFault::FutureDate));
}

#[test]
fn a_session_must_start_with_its_one_opening_line() {
    let no_opening = pair(1, TUE, "buy", "INFY", 1, "-100", "-1");
    assert_eq!(
        figures(&no_opening, 24, 1, "100"),
        Err(PnlError::NoOpening { line: 0 })
    );
    // The opening is there, but not first.
    let late = [no_opening, opening(1, TUE, "100000")].concat();
    assert_eq!(
        figures(&late, 24, 1, "100"),
        Err(PnlError::NoOpening { line: 0 })
    );
    let twice = [opening(1, MON, "100000"), opening(1, TUE, "100000")].concat();
    assert_eq!(
        figures(&twice, 24, 1, "100"),
        Err(PnlError::SecondOpening { line: 1 })
    );
}

#[test]
fn a_line_backdated_within_its_session_is_refused() {
    let lines = [
        opening(1, TUE, "100000"),
        pair(1, MON, "buy", "INFY", 1, "-100", "-1"),
    ]
    .concat();
    assert_eq!(
        figures(&lines, 24, 1, "100"),
        Err(PnlError::DateOrder { line: 1 })
    );
    // Backdated against the latest line, not only against the opening.
    let later = [
        opening(1, LAST_FRI, "100000"),
        pair(1, TUE, "buy", "INFY", 1, "-100", "-1"),
        pair(1, MON, "buy", "INFY", 1, "-100", "-1"),
    ]
    .concat();
    assert_eq!(
        figures(&later, 24, 1, "100"),
        Err(PnlError::DateOrder { line: 3 })
    );
}

#[test]
fn the_active_session_must_have_lines() {
    for lines in [Vec::new(), opening(1, TUE, "100000")] {
        assert_eq!(
            figures(&lines, 24, 2, "100"),
            Err(PnlError::NoActiveSession)
        );
    }
}

#[test]
fn shares_held_outside_the_active_session_are_refused() {
    let lines = [
        opening(1, TUE, "100000"),
        pair(1, TUE, "buy", "INFY", 1, "-100", "-1"),
        opening(2, TUE, "100000"),
    ]
    .concat();
    assert_eq!(
        figures(&lines, 24, 2, "100"),
        Err(PnlError::OpenOutsideActive(id(1)))
    );
}

#[test]
fn an_open_position_without_a_usable_mark_has_no_figures() {
    let held = |symbol| {
        [
            opening(1, TUE, "100000"),
            pair(1, TUE, "buy", symbol, 1, "-100", "-1"),
        ]
        .concat()
    };
    assert_eq!(
        figures(&held("SBIN"), 24, 1, "100"),
        Err(PnlError::MissingMark)
    );
    for (mark, fault) in [
        ("0", PoolError::BadMark),
        ("0.00001", PoolError::MarkOutOfBounds),
    ] {
        assert_eq!(
            figures(&held("INFY"), 24, 1, mark),
            Err(PnlError::Pool(fault))
        );
    }
}

#[test]
fn a_base_that_is_not_positive_is_refused() {
    let sessions =
        session_figures(&opening(1, TUE, "100000"), day(24), id(1), marks("1")).expect("figures");
    for cash in ["0", "-1"] {
        assert_eq!(
            shared_figures(&sessions, d(cash)),
            Err(PnlError::BaseNotPositive)
        );
    }
    // All cash gone before today.
    let spent = [
        opening(1, MON, "10"),
        pair(1, MON, "buy", "INFY", 1, "-100", "-1"),
        pair(1, MON, "sell", "INFY", 1, "1", "-1"),
    ]
    .concat();
    assert_eq!(figures(&spent, 24, 1, "1"), Err(PnlError::BaseNotPositive));
}

#[test]
fn sums_are_bounded_at_1e15() {
    assert_eq!(MAX_SUM, d("1000000000000000"));
    assert_eq!(sum(MAX_SUM - Decimal::ONE, Decimal::ONE), Ok(MAX_SUM));
    for (a, b) in [
        (MAX_SUM, d("0.0001")),
        (-MAX_SUM, d("-0.0001")),
        (Decimal::MAX, Decimal::MAX),
    ] {
        assert_eq!(sum(a, b), Err(PnlError::SumTooLarge));
    }
}

/// `pairs` round trips in one session, each buying for `cost` and selling for `proceeds`.
fn round_trips(
    ts: &'static str,
    session: u32,
    pairs: u32,
    cost: Decimal,
    proceeds: Decimal,
) -> Vec<ReplayLine<'static>> {
    let mut lines = opening(session, ts, "100000");
    for _ in 0..pairs {
        let mut buy = pair(session, ts, "buy", "INFY", 1, "-1", "-1");
        buy[1].amount = -cost;
        let mut sell = pair(session, ts, "sell", "INFY", 1, "1", "-1");
        sell[1].amount = proceeds;
        lines.extend(buy);
        lines.extend(sell);
    }
    lines
}

#[test]
fn every_place_that_adds_up_enforces_the_bound() {
    let big = |session, n| round_trips(TUE, session, n, MAX_LINE_AMOUNT, Decimal::ONE);
    // Realised: 10,001 losses of about 1e11 in one session pass 1e15.
    assert_eq!(
        session_figures(&big(1, 10_001), day(24), id(1), marks("1")),
        Err(PnlError::SumTooLarge)
    );
    // Two sessions, each inside the bound, pass it only when added together.
    let both = [big(1, 6_000), big(2, 6_000)].concat();
    assert!(session_figures(&both, day(24), id(1), marks("1")).is_ok());
    assert_eq!(figures(&both, 24, 1, "1"), Err(PnlError::SumTooLarge));
    // Cash at the window start: 10,001 gains of about 1e11 from before this week.
    let gains = round_trips(LAST_FRI, 1, 10_001, Decimal::ONE, MAX_LINE_AMOUNT);
    assert_eq!(figures(&gains, 24, 1, "1"), Err(PnlError::SumTooLarge));
}

/// One assertion per addition, so no later check can answer for an earlier one.
#[test]
fn each_addition_checks_the_bound_itself() {
    let too_large = Err(PnlError::SumTooLarge);
    let (one, today) = (Decimal::ONE, day(24));
    let full = SessionWindow {
        realised: MAX_SUM,
        open_loss: -MAX_SUM,
        equity: MAX_SUM,
        has_lines: false,
    };
    // Window cash.
    let mut w = full;
    assert_eq!(w.line(LedgerKind::Opening, one, today, today), too_large);
    // A buy's cost added back. Cash alone lands on the bound, so only the add-back can fail
    // (from a start already past it).
    let mut w = SessionWindow {
        equity: MAX_SUM + one,
        ..full
    };
    assert_eq!(w.line(LedgerKind::Buy, -one, day(23), today), too_large);
    // Cost leaving a pool before the window.
    let mut w = SessionWindow {
        equity: -MAX_SUM,
        ..full
    };
    assert_eq!(w.sale(one, one, day(23), today), too_large);
    // Realised.
    let mut w = full;
    assert_eq!(w.sale(one, one, today, today), too_large);
    // Open loss.
    let mut w = full;
    assert_eq!(w.holding(-one), too_large);
    // Across sessions: realised, then open loss. In the first case the second session's open
    // loss brings the total back inside the bound, so only the realised addition can fail.
    for (realised, open_loss, then) in [
        (MAX_SUM, Decimal::ZERO, (one, -one)),
        (Decimal::ZERO, -MAX_SUM, (Decimal::ZERO, -one)),
    ] {
        let first = SessionWindow {
            realised,
            open_loss,
            equity: one,
            has_lines: true,
        };
        let second = SessionWindow {
            realised: then.0,
            open_loss: then.1,
            ..first
        };
        let sessions = Sessions {
            active: id(1),
            by_session: [(id(1), both_windows(first)), (id(2), both_windows(second))].into(),
        };
        assert_eq!(shared_figures(&sessions, one), Err(PnlError::SumTooLarge));
    }
}
