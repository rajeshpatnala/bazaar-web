//! The real `bazaar-server openapi` command: exits 0, prints JSON, and matches both the
//! in-process document and the committed generated client schema.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

fn run() -> serde_json::Value {
    let o = Command::new(env!("CARGO_BIN_EXE_bazaar-server"))
        .arg("openapi")
        .output()
        .unwrap();
    assert!(o.status.success(), "exit {:?}", o.status);
    serde_json::from_slice(&o.stdout).expect("stdout is JSON")
}

#[test]
fn openapi_command_prints_the_document() {
    let printed = run();
    let expected: serde_json::Value =
        serde_json::from_str(&bazaar_server::openapi::document().to_pretty_json().unwrap())
            .unwrap();
    assert_eq!(printed, expected);
}

#[test]
fn every_printed_path_is_in_the_committed_web_schema() {
    let printed = run();
    let paths = printed["paths"].as_object().expect("paths object");
    assert!(!paths.is_empty());
    let schema = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/api/schema.d.ts"),
    )
    .unwrap();
    for p in paths.keys() {
        assert!(
            schema.contains(&format!("\"{p}\"")),
            "{p} missing from schema.d.ts"
        );
    }
}
