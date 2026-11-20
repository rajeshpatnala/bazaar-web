//! Workspace-level checks: `.env.example`, versions, config files, disclaimer equality.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn env_example_has_empty_secrets_and_no_mode_variable() {
    let text = std::fs::read_to_string(root().join(".env.example")).unwrap();
    for line in text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && l.contains('='))
    {
        let (key, value) = line.split_once('=').unwrap();
        let key = key.trim();
        assert!(
            !key.to_ascii_lowercase().contains("mode"),
            "{key}: execution mode lives only in config/risk.toml"
        );
        if key != "BAZAAR_PORT" {
            assert!(
                value.trim().is_empty(),
                "{key} must be empty in .env.example"
            );
        }
    }
}

#[test]
fn server_and_web_versions_are_equal() {
    let server = std::fs::read_to_string(root().join("crates/server/Cargo.toml")).unwrap();
    let server: toml::Table = server.parse().unwrap();
    let server_v = server["package"]["version"].as_str().unwrap().to_string();
    let web = std::fs::read_to_string(root().join("web/package.json")).unwrap();
    let web: serde_json::Value = serde_json::from_str(&web).unwrap();
    assert_eq!(web["version"].as_str().unwrap(), server_v);
}

#[test]
fn config_holds_exactly_the_nine_files() {
    let mut names: Vec<String> = std::fs::read_dir(root().join("config"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "backtest.toml",
            "calendar.toml",
            "charges.toml",
            "compliance.toml",
            "data.toml",
            "events.toml",
            "risk.toml",
            "strategies.toml",
            "watchlists.toml"
        ]
    );
}

#[test]
fn rust_and_ts_disclaimers_are_byte_equal() {
    let ts = std::fs::read_to_string(root().join("web/src/shared/disclaimer.ts")).unwrap();
    let decl = ts
        .split("export const DISCLAIMER")
        .nth(1)
        .expect("DISCLAIMER declaration");
    let start = decl.find('"').unwrap() + 1;
    let end = start + decl[start..].find('"').unwrap();
    let text = &decl[start..end];
    assert!(!text.contains('\\'), "no escapes allowed in the constant");
    assert_eq!(text, bazaar_core::disclaimer::DISCLAIMER);
}
