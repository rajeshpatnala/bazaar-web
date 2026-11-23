#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use bazaar_core::testutil::{FakeClock, fixture_path};
use bazaar_data::instruments::ParseLimits;
use bazaar_store::SqliteStore;
use chrono::TimeZone;
use chrono_tz::Asia::Kolkata;

use super::*;
use crate::commands::{DB_FILE, import_instruments};
use crate::compose::open_store;

fn clock(d: u32) -> FakeClock {
    FakeClock::at(Kolkata.with_ymd_and_hms(2026, 1, d, 10, 0, 0).unwrap())
}

/// Temp data dir with the fixture master imported on 2026-01-06, and a temp config dir with the
/// fixture calendar and a watchlist of `symbols`.
async fn setup(symbols: &[&str]) -> (tempfile::TempDir, tempfile::TempDir, SqliteStore) {
    let data = tempfile::tempdir().unwrap();
    import_instruments(
        data.path(),
        &fixture_path("data/instruments_good.csv"),
        &clock(6),
        &ParseLimits::default(),
    )
    .await
    .unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let cal: PathBuf = fixture_path("config/calendar.toml");
    std::fs::copy(cal, cfg.path().join("calendar.toml")).unwrap();
    write_watchlist(&cfg, symbols);
    let store = open_store(&data.path().join(DB_FILE)).await.unwrap();
    (data, cfg, store)
}

fn write_watchlist(cfg: &tempfile::TempDir, symbols: &[&str]) {
    let list = symbols
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        cfg.path().join("watchlists.toml"),
        format!(
            "schema_version = 1\nverified_on = \"2026-01-05\"\nsource = \"test\"\n[[watchlist]]\nname = \"w\"\nsymbols = [{list}]\n"
        ),
    )
    .unwrap();
}

#[tokio::test]
async fn nse_equity_maps_from_the_master_with_no_freeze() {
    let (_d, cfg, store) = setup(&["TESTEQ"]).await;
    let (snap, warnings) = InstrumentSnapshot::load(&store, cfg.path(), &clock(6)).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let s = snap.spec("TESTEQ").unwrap();
    assert_eq!(s.segment, Segment::Equity);
    assert_eq!(s.kind, InstrumentKind::CashNonFno);
    assert_eq!(s.lot_size, 1);
    assert_eq!(s.tick_size, "0.05".parse::<Decimal>().unwrap());
    assert_eq!(s.freeze_qty, None);
    assert_eq!(snap.len(), 1);
}

#[tokio::test]
async fn unknown_derivative_and_non_equity_symbols_have_no_spec() {
    let (_d, cfg, store) = setup(&["TESTEQ", "NOPE", "TESTU25JANFUT", "TESTGB"]).await;
    let (snap, warnings) = InstrumentSnapshot::load(&store, cfg.path(), &clock(6)).await;
    assert_eq!(snap.len(), 1);
    for s in ["NOPE", "TESTU25JANFUT", "TESTGB", "testeq", ""] {
        assert_eq!(snap.spec(s), None, "{s}");
    }
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].starts_with("3 watchlist symbol"),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn stale_master_gives_an_empty_snapshot_with_the_check_connections_category() {
    let (_d, cfg, store) = setup(&["TESTEQ"]).await;
    // The same rule as `check-connections`: a week later the master is stale.
    let (snap, warnings) = InstrumentSnapshot::load(&store, cfg.path(), &clock(20)).await;
    assert!(snap.is_empty());
    assert_eq!(warnings, ["instrument specs unavailable: master stale"]);
}

#[tokio::test]
async fn never_fetched_master_and_missing_calendar_are_empty() {
    let data = tempfile::tempdir().unwrap();
    let store = open_store(&data.path().join(DB_FILE)).await.unwrap();
    let cfg = tempfile::tempdir().unwrap();
    write_watchlist(&cfg, &["TESTEQ"]);
    let (snap, w) = InstrumentSnapshot::load(&store, cfg.path(), &clock(6)).await;
    assert!(snap.is_empty());
    assert_eq!(
        w,
        ["instrument specs unavailable: master calendar_unreadable"]
    );
    std::fs::copy(
        fixture_path("config/calendar.toml"),
        cfg.path().join("calendar.toml"),
    )
    .unwrap();
    let (snap, w) = InstrumentSnapshot::load(&store, cfg.path(), &clock(6)).await;
    assert!(snap.is_empty());
    assert_eq!(w, ["instrument specs unavailable: master empty"]);
}

#[tokio::test]
async fn unreadable_watchlists_give_an_empty_snapshot() {
    let (_d, cfg, store) = setup(&["TESTEQ"]).await;
    std::fs::remove_file(cfg.path().join("watchlists.toml")).unwrap();
    let (snap, w) = InstrumentSnapshot::load(&store, cfg.path(), &clock(6)).await;
    assert!(snap.is_empty());
    assert_eq!(w, ["instrument specs unavailable: watchlists unreadable"]);
}

#[tokio::test]
async fn over_the_bound_loads_nothing_never_a_partial_set() {
    let (_d, cfg, store) = setup(&["TESTEQ"]).await;
    let many: Vec<String> = (0..=MAX_SPECS).map(|i| format!("S{i}")).collect();
    // Two lists of distinct symbols: each list is within its own cap, the union is over.
    let half = many.len() / 2;
    let list = |s: &[String]| {
        s.iter()
            .map(|x| format!("\"{x}\""))
            .collect::<Vec<_>>()
            .join(",")
    };
    std::fs::write(
        cfg.path().join("watchlists.toml"),
        format!(
            "schema_version = 1\nverified_on = \"2026-01-05\"\nsource = \"t\"\n[[watchlist]]\nname = \"a\"\nsymbols = [{}]\n[[watchlist]]\nname = \"b\"\nsymbols = [{}]\n",
            list(&many[..half]),
            list(&many[half..])
        ),
    )
    .unwrap();
    let (snap, w) = InstrumentSnapshot::load(&store, cfg.path(), &clock(6)).await;
    assert!(snap.is_empty());
    assert_eq!(w.len(), 1);
    assert!(w[0].contains("more than"), "{w:?}");
}

fn row() -> Instrument {
    Instrument {
        instrument_token: 1,
        exchange_token: 1,
        tradingsymbol: "X".into(),
        name: "X".into(),
        exchange: Exchange::Nse,
        instrument_type: InstrumentType::Equity,
        segment: "NSE".into(),
        expiry: None,
        strike: None,
        tick_size: "0.05".parse().unwrap(),
        lot_size: 1,
    }
}

#[test]
fn map_row_refuses_zero_tick_zero_lot_and_non_nse_or_derivative_rows() {
    let k = InstrumentKind::CashNonFno;
    assert!(map_row(&row(), "X", k).is_some());
    let mut r = row();
    r.tick_size = Decimal::ZERO;
    assert_eq!(map_row(&r, "X", k), None);
    let mut r = row();
    r.lot_size = 0;
    assert_eq!(map_row(&r, "X", k), None);
    let mut r = row();
    r.exchange = Exchange::Bse;
    assert_eq!(map_row(&r, "X", k), None);
    let mut r = row();
    r.instrument_type = InstrumentType::Future;
    assert_eq!(map_row(&r, "X", k), None);
    let mut r = row();
    r.expiry = chrono::NaiveDate::from_ymd_opt(2026, 1, 29);
    assert_eq!(map_row(&r, "X", k), None);
    assert_eq!(map_row(&row(), "Y", k), None);
}

#[test]
fn cash_fno_kind_is_carried_from_the_master_not_defaulted() {
    let s = map_row(&row(), "X", InstrumentKind::CashFno).unwrap();
    assert_eq!(s.kind, InstrumentKind::CashFno);
}
