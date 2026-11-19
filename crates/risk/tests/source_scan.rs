//! Source scan over `crates/risk` (TS-45-1 and the never-break list): fail-closed shapes cannot be
//! reintroduced by accident. Comments are dropped before matching, so prose may mention a name.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every `.rs` file under `src` and `tests`, except this scanner (it names the banned words).
fn sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    for d in ["src", "tests"] {
        rs_files(&root().join(d), &mut files);
    }
    files
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n != "source_scan.rs"))
        .map(|p| {
            let name = p.strip_prefix(root()).unwrap().display().to_string();
            (name, fs::read_to_string(&p).unwrap())
        })
        .collect()
}

/// Code lines with whole-line comments removed (doc comments included), numbered from 1.
fn code_lines(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .map(|(i, l)| (i + 1, l.to_string()))
        .collect()
}

fn idents(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

#[test]
fn sources_are_found() {
    let s = sources();
    assert!(s.iter().any(|(n, _)| n == "src/gate.rs"));
    assert!(s.len() >= 10, "only {} files scanned", s.len());
}

#[test]
fn no_derive_default_on_snapshot_intent_or_ctx() {
    const GUARDED: [&str; 7] = [
        "PortfolioSnapshot",
        "OrderIntent",
        "RuleCtx",
        "Position",
        "GatedIntent",
        "DataInputs",
        "ModeEvidence",
    ];
    let mut hits = Vec::new();
    for (name, text) in sources() {
        let mut attrs = String::new();
        for (n, line) in code_lines(&text) {
            let t = line.trim();
            if t.starts_with("#[") {
                attrs.push_str(t);
                continue;
            }
            let w: Vec<&str> = idents(t).collect();
            let declared = w
                .windows(2)
                .find(|p| matches!(p[0], "struct" | "enum"))
                .map(|p| p[1]);
            if let Some(ty) = declared
                && GUARDED.contains(&ty)
                && idents(&attrs).any(|a| a == "Default")
            {
                hits.push(format!("{name}:{n} derive(Default) on {ty}"));
            }
            if w.starts_with(&["impl", "Default", "for"])
                && w.get(3).is_some_and(|ty| GUARDED.contains(ty))
            {
                hits.push(format!("{name}:{n} impl Default for {}", w[3]));
            }
            attrs.clear();
        }
    }
    assert!(hits.is_empty(), "{hits:#?}");
}

#[test]
fn no_bypass_skip_gate_or_force_identifier() {
    let mut hits = Vec::new();
    for (name, text) in sources() {
        for (n, line) in code_lines(&text) {
            for w in idents(&line) {
                let l = w.to_ascii_lowercase();
                if l.contains("bypass") || l.contains("skip_gate") || l.contains("skipgate") {
                    hits.push(format!("{name}:{n} {w}"));
                }
                if l == "force" || l.starts_with("force_") || l.ends_with("_force") {
                    hits.push(format!("{name}:{n} {w}"));
                }
            }
        }
    }
    // Names such as `..._forced_true` describe a rejected input and are not an API; only
    // exact `force` words are banned above.
    assert!(hits.is_empty(), "{hits:#?}");
}

#[test]
fn no_f64_or_f32_in_the_risk_crate() {
    let mut hits = Vec::new();
    for (name, text) in sources() {
        for (n, line) in code_lines(&text) {
            // Substring match also catches literal suffixes such as `0.5_f64` or `1f32`.
            if line.contains("f64") || line.contains("f32") {
                hits.push(format!("{name}:{n}"));
            }
        }
    }
    assert!(hits.is_empty(), "{hits:#?}");
}

#[test]
fn live_trading_is_not_in_default_features() {
    let manifest = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let features = manifest.split("[features]").nth(1).expect("[features]");
    let table = features.split("\n[").next().unwrap_or(features);
    let default = table
        .lines()
        .find(|l| l.trim_start().starts_with("default"))
        .expect("default line");
    assert!(!default.contains("live-trading"), "{default}");
    assert!(default.contains("[]"), "default must stay empty: {default}");
}

#[test]
fn gated_intent_is_built_only_in_gate_rs() {
    let mut hits = Vec::new();
    let mut in_gate = 0;
    // Library code only: the fields are private, so tests cannot construct it (compile error);
    // `-> GatedIntent {` in a test helper is a return type, not a construction.
    for (name, text) in sources().into_iter().filter(|(n, _)| n.starts_with("src/")) {
        for (n, line) in code_lines(&text) {
            // Whitespace removed so `GatedIntent  {` cannot slip past.
            let squeezed: String = line.split_whitespace().collect();
            let declaration =
                squeezed.contains("structGatedIntent{") || squeezed.contains("implGatedIntent{");
            if squeezed.contains("GatedIntent{") && !declaration {
                if name == "src/gate.rs" {
                    in_gate += 1;
                } else {
                    hits.push(format!("{name}:{n}"));
                }
            }
        }
    }
    assert!(hits.is_empty(), "{hits:#?}");
    assert_eq!(in_gate, 1, "exactly one construction site in gate.rs");
}

#[test]
fn the_scanner_helpers_behave() {
    let lines = code_lines("// force\nlet skip_gate = 1;\n");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0, 2, "comment lines are dropped, numbers kept");
    assert!(idents("let x: f64 = 1.0;").any(|w| w == "f64"));
}
