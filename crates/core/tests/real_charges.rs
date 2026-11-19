//! The shipped `config/charges.toml` (Zerodha pages, owner-pasted 2026-11-20) through the typed loader.
//! Expected values are the owner's tables in docs/owner-input/F2-charges.md. The whole
//! (exchange x segment x side) grid is asserted, not a sample.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::str::FromStr;

use bazaar_core::config::{
    Brokerage, ChargeExchange as Ex, ChargeSegment as Seg, ChargeSide as Side, ChargesConfig,
    TurnoverBasis, load,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn cfg() -> ChargesConfig {
    load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config"
    )))
    .unwrap()
}

fn on() -> NaiveDate {
    d(2026, 11, 20)
}

/// Per-segment owner values: brokerage, STT (percent), whether STT is charged on both sides,
/// (NSE, BSE) transaction charge percent, buy-side stamp percent, turnover basis.
struct Expect {
    seg: Seg,
    brokerage: Brokerage,
    stt: &'static str,
    stt_both_sides: bool,
    txn_nse: &'static str,
    txn_bse: &'static str,
    stamp_buy: &'static str,
    basis: TurnoverBasis,
}

fn grid() -> Vec<Expect> {
    let flat_or_pct = || Brokerage::FlatOrPctLower {
        flat: dec("20"),
        pct: dec("0.03"),
    };
    vec![
        Expect {
            seg: Seg::EquityDelivery,
            brokerage: Brokerage::Zero {},
            stt: "0.1",
            stt_both_sides: true,
            txn_nse: "0.00307",
            txn_bse: "0.00375",
            stamp_buy: "0.015",
            basis: TurnoverBasis::Notional,
        },
        Expect {
            seg: Seg::EquityIntraday,
            brokerage: flat_or_pct(),
            stt: "0.025",
            stt_both_sides: false,
            txn_nse: "0.00307",
            txn_bse: "0.00375",
            stamp_buy: "0.003",
            basis: TurnoverBasis::Notional,
        },
        Expect {
            seg: Seg::EquityFutures,
            brokerage: flat_or_pct(),
            stt: "0.05",
            stt_both_sides: false,
            txn_nse: "0.00183",
            txn_bse: "0",
            stamp_buy: "0.002",
            basis: TurnoverBasis::Notional,
        },
        Expect {
            seg: Seg::EquityOptions,
            brokerage: Brokerage::Flat { flat: dec("20") },
            stt: "0.15",
            stt_both_sides: false,
            txn_nse: "0.03553",
            txn_bse: "0.0325",
            stamp_buy: "0.003",
            basis: TurnoverBasis::Premium,
        },
    ]
}

#[test]
fn shipped_charges_are_real_dated_and_sourced() {
    let c = cfg();
    assert!(!c.placeholder);
    assert_eq!(c.verified_on, Some(on()));
    assert_eq!(c.meta.as_ref().unwrap().broker, "zerodha");
    assert_eq!(c.gst.as_ref().unwrap().rate_pct, dec("18"));
    assert_eq!(c.option_exercise.as_ref().unwrap().stt_pct, dec("0.15"));
    let dp = c.dp.as_ref().unwrap();
    assert_eq!(dp.per_scrip_total, dec("15.34")); // 3.5 CDSL + 9.5 Zerodha + 2.34 GST
    assert!(dp.includes_gst);
    assert_eq!(dp.verified_on, on());
    assert!(dp.source.contains("zerodha.com/charges"));
    assert_eq!(c.rate_set.len(), 16);
    for r in &c.rate_set {
        assert_eq!(r.verified_on, on());
        assert_eq!(r.effective_from, on());
        assert!(r.source.contains("zerodha.com/charges"));
    }
}

#[test]
fn full_grid_matches_the_owner_tables() {
    let c = cfg();
    for e in grid() {
        for (ex, txn) in [(Ex::Nse, e.txn_nse), (Ex::Bse, e.txn_bse)] {
            for side in [Side::Buy, Side::Sell] {
                let r = c.rate_set_for(ex, e.seg, side, on()).unwrap();
                let tag = format!("{ex:?} {:?} {side:?}", e.seg);
                assert_eq!(r.brokerage, e.brokerage, "{tag}");
                let stt = if side == Side::Sell || e.stt_both_sides {
                    e.stt
                } else {
                    "0"
                };
                assert_eq!(r.stt_ctt_pct, dec(stt), "{tag} stt");
                assert_eq!(r.exchange_txn_pct, dec(txn), "{tag} txn");
                let stamp = if side == Side::Buy { e.stamp_buy } else { "0" };
                assert_eq!(r.stamp_pct, dec(stamp), "{tag} stamp");
                assert_eq!(r.sebi_per_crore, dec("10"), "{tag} sebi");
                // IPFT: NSE Rs 0.01 per crore = 0.0000001 percent; no BSE rate was listed.
                let ipft = if ex == Ex::Nse { "0.0000001" } else { "0" };
                assert_eq!(r.ipft_pct, dec(ipft), "{tag} ipft");
                assert_eq!(r.turnover_basis, e.basis, "{tag} basis");
            }
        }
    }
}

#[test]
fn delivery_is_shipped_and_nothing_applies_before_the_effective_date() {
    let c = cfg();
    assert!(
        c.rate_set_for(Ex::Nse, Seg::EquityDelivery, Side::Sell, on())
            .is_ok()
    );
    for e in grid() {
        assert!(
            c.rate_set_for(Ex::Nse, e.seg, Side::Buy, d(2026, 11, 19))
                .is_err()
        );
    }
}
