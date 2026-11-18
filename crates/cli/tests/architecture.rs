//! Architecture checks from `cargo metadata`: crate edges against the `bz-architecture` table,
//! no domain crate depends on `bazaar-server`, `live-trading` is in no default feature.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeMap, process::Command};

use serde_json::Value;

fn metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo metadata");
    assert!(out.status.success(), "cargo metadata failed");
    serde_json::from_slice(&out.stdout).expect("metadata json")
}

/// Workspace (`bazaar-*`) edges per crate, dev-dependencies included.
fn edges(meta: &Value) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    for pkg in meta["packages"].as_array().unwrap() {
        let name = pkg["name"].as_str().unwrap().to_string();
        let deps = pkg["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["name"].as_str())
            .filter(|n| n.starts_with("bazaar-"))
            .map(str::to_string)
            .collect();
        map.insert(name, deps);
    }
    map
}

/// Allowed edges per domain crate; `None` for the composition roots (`cli`, `server`).
fn allowed(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "bazaar-core" => &[],
        "bazaar-indicators" | "bazaar-options" | "bazaar-data" | "bazaar-store" | "bazaar-risk" => {
            &["bazaar-core"]
        }
        "bazaar-regime" => &["bazaar-core", "bazaar-indicators"],
        "bazaar-strategies" => &[
            "bazaar-core",
            "bazaar-indicators",
            "bazaar-options",
            "bazaar-regime",
        ],
        "bazaar-backtest" => &[
            "bazaar-core",
            "bazaar-strategies",
            "bazaar-risk",
            "bazaar-indicators",
        ],
        "bazaar-scanner" => &[
            "bazaar-core",
            "bazaar-strategies",
            "bazaar-regime",
            "bazaar-risk",
            "bazaar-backtest",
        ],
        "bazaar-execution" => &["bazaar-core", "bazaar-risk"],
        "bazaar-paper" => &["bazaar-core", "bazaar-execution"],
        "bazaar-broker" => &["bazaar-core", "bazaar-execution", "bazaar-data"],
        _ => return None,
    })
}

#[test]
fn fifteen_workspace_crates() {
    assert_eq!(edges(&metadata()).len(), 15);
}

#[test]
fn every_edge_is_in_the_allowed_table() {
    for (name, deps) in edges(&metadata()) {
        let Some(ok) = allowed(&name) else { continue };
        for dep in deps {
            assert!(
                ok.contains(&dep.as_str()),
                "{name} -> {dep} is not an allowed edge"
            );
        }
    }
}

#[test]
fn no_domain_crate_depends_on_a_composition_root() {
    for (name, deps) in edges(&metadata()) {
        if allowed(&name).is_none() {
            continue;
        }
        assert!(
            !deps
                .iter()
                .any(|d| d == "bazaar-server" || d == "bazaar-cli"),
            "{name} depends on a composition root"
        );
    }
}

#[test]
fn live_trading_is_in_no_default_feature() {
    let meta = metadata();
    for pkg in meta["packages"].as_array().unwrap() {
        let name = pkg["name"].as_str().unwrap();
        let default = pkg["features"]
            .get("default")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            !default
                .iter()
                .any(|f| f.as_str().is_some_and(|f| f.contains("live-trading"))),
            "{name}: live-trading is a default feature"
        );
    }
}

#[test]
fn broker_is_only_an_optional_dependency() {
    let meta = metadata();
    for pkg in meta["packages"].as_array().unwrap() {
        let name = pkg["name"].as_str().unwrap();
        if allowed(name).is_some() {
            continue;
        }
        for d in pkg["dependencies"].as_array().unwrap() {
            if d["name"] == "bazaar-broker" {
                assert!(
                    d["optional"] == true,
                    "{name}: bazaar-broker must be optional"
                );
            }
        }
    }
}

#[test]
fn no_recipe_uses_all_features() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../justfile");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        !text.contains("--all-features"),
        "justfile uses --all-features"
    );
}
