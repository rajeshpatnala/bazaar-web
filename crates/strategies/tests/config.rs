//! AC 5 and 6 for the registry: the shipped `config/strategies.toml` ships every strategy disabled,
//! and unknown names, unknown param keys and out-of-range values are load errors.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use bazaar_strategies::ParamError;
use bazaar_strategies::registry::{build_all, factories};
use common::{CASES, enabled, shipped};

#[test]
fn every_strategy_ships_disabled_and_nothing_is_built() {
    let cfg = shipped();
    assert_eq!(cfg.strategy.len(), factories().len());
    for f in factories() {
        let table = cfg.strategy.get(f.id()).unwrap();
        assert_eq!(
            table.get("enabled").and_then(toml::Value::as_bool),
            Some(false),
            "{}",
            f.id()
        );
    }
    assert!(build_all(&cfg).unwrap().is_empty());
}

#[test]
fn enabled_strategies_build_with_the_declared_regime_cells() {
    let built = build_all(&enabled()).unwrap();
    let ids: Vec<_> = built.iter().map(|s| s.meta().id).collect();
    assert_eq!(ids, CASES.map(|c| c.id));
    assert!(built.iter().all(|s| s.meta().min_history_bars >= 2));
}

#[test]
fn an_unknown_strategy_name_is_a_load_error() {
    let mut cfg = shipped();
    let t = cfg.strategy.values().next().unwrap().clone();
    cfg.strategy.insert("momentum_magic".into(), t);
    assert_eq!(
        build_all(&cfg).map(|v| v.len()).unwrap_err(),
        ParamError::UnknownStrategy("momentum_magic".into())
    );
}

fn with_param(id: &str, key: &str, value: toml::Value) -> Result<usize, ParamError> {
    let mut cfg = enabled();
    let params = cfg
        .strategy
        .get_mut(id)
        .unwrap()
        .get_mut("params")
        .and_then(toml::Value::as_table_mut)
        .unwrap();
    // A range test is only real if the shipped table already has the key.
    assert_eq!(params.contains_key(key), key != "not_a_param", "{id} {key}");
    params.insert(key.into(), value);
    build_all(&cfg).map(|v| v.len())
}

#[test]
fn an_unknown_param_key_is_a_load_error_for_every_strategy() {
    for c in &CASES {
        let err = with_param(c.id, "not_a_param", 1.into()).unwrap_err();
        assert!(matches!(err, ParamError::Parse { .. }), "{}: {err}", c.id);
    }
}

#[test]
fn an_out_of_range_param_is_a_load_error_for_every_strategy() {
    for c in &CASES {
        for v in [1, 10_000] {
            let err = with_param(c.id, "atr_period", v.into()).unwrap_err();
            assert!(
                matches!(
                    err,
                    ParamError::Invalid {
                        key: "atr_period",
                        ..
                    }
                ),
                "{} {v}: {err}",
                c.id
            );
        }
    }
    // `max_stop_atr` exists in the trend_pullback and bollinger tables only.
    for id in ["trend_pullback", "bollinger_reversion"] {
        let err = with_param(id, "max_stop_atr", 0.0.into()).unwrap_err();
        assert!(
            matches!(
                err,
                ParamError::Invalid {
                    key: "max_stop_atr",
                    ..
                }
            ),
            "{id}: {err}"
        );
    }
}
