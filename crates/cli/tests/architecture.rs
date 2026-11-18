//! Architecture checks from `cargo metadata`: crate edges against the `bz-architecture` table,
//! no domain crate depends on `bazaar-server`, `live-trading` is in no default feature.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{collections::BTreeMap, process::Command};

use serde_json::Value;

fn metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
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
        // Composition roots may depend on every crate (`bz-architecture` s2), so no edge list.
        "bazaar-cli" | "bazaar-server" => return None,
        other => panic!("unknown crate {other}: add it to allowed() (new crate = ADR)"),
    })
}

#[test]
fn fifteen_workspace_crates() {
    assert_eq!(
        edges(&metadata()).len(),
        15,
        "update allowed() when adding a crate"
    );
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

/// Every entry reachable from the `default` feature of a package (features, `dep:x`, `x/feat`).
fn default_closure(pkg: &Value) -> Vec<String> {
    let features = pkg["features"].as_object().unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut todo = vec!["default".to_string()];
    while let Some(f) = todo.pop() {
        if seen.contains(&f) {
            continue;
        }
        if let Some(list) = features.get(&f).and_then(Value::as_array) {
            todo.extend(list.iter().filter_map(Value::as_str).map(str::to_string));
        }
        seen.push(f);
    }
    seen
}

#[test]
fn live_trading_is_in_no_default_feature() {
    let meta = metadata();
    for pkg in meta["packages"].as_array().unwrap() {
        let name = pkg["name"].as_str().unwrap();
        for entry in default_closure(pkg) {
            assert!(
                !entry.contains("live-trading") && !entry.contains("bazaar-broker"),
                "{name}: default features reach `{entry}`"
            );
        }
    }
}

#[test]
fn default_closure_follows_indirect_features() {
    let pkg = serde_json::json!({"features": {
        "default": ["extras"],
        "extras": ["bazaar-broker/live-trading"]
    }});
    assert!(
        default_closure(&pkg)
            .iter()
            .any(|e| e.contains("live-trading"))
    );
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

fn scan_for_all_features(path: &std::path::Path, hits: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan_for_all_features(&p, hits);
        } else if std::fs::read_to_string(&p).is_ok_and(|t| t.contains("--all-features")) {
            hits.push(p.display().to_string());
        }
    }
}

#[test]
fn no_recipe_or_workflow_uses_all_features() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let justfile = std::fs::read_to_string(root.join("justfile")).unwrap();
    assert!(
        !justfile.contains("--all-features"),
        "justfile uses --all-features"
    );
    let mut hits = Vec::new();
    scan_for_all_features(&root.join(".github"), &mut hits);
    scan_for_all_features(&root.join("scripts"), &mut hits);
    assert!(hits.is_empty(), "--all-features found in {hits:?}");
}
