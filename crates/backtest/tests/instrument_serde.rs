//! `LegInstrument` and `Product::Carry` parse from config-style text (QUAN-83-2).

#![allow(clippy::unwrap_used)]

use bazaar_backtest::{LegInstrument, OptionRight, Product};
use serde::Deserialize;

#[derive(Deserialize)]
struct W {
    i: LegInstrument,
    p: Product,
}

#[test]
fn option_put_with_carry_parses() {
    let w: W = toml::from_str("p = \"carry\"\ni = { option = \"put\" }").unwrap();
    assert_eq!(w.i, LegInstrument::Option(OptionRight::Put));
    assert_eq!(w.p, Product::Carry);
}

#[test]
fn future_and_delivery_parse_and_call_differs_from_put() {
    let w: W = toml::from_str("p = \"delivery\"\ni = \"future\"").unwrap();
    assert_eq!(w.i, LegInstrument::Future);
    assert_ne!(
        LegInstrument::Option(OptionRight::Call),
        LegInstrument::Option(OptionRight::Put)
    );
}
