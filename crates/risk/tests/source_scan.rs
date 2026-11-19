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

const GUARDED: [&str; 11] = [
    "PortfolioSnapshot",
    "OrderIntent",
    "RuleCtx",
    "Position",
    "GatedIntent",
    "DataInputs",
    "ModeEvidence",
    "EvidenceInputs",
    "EventFlags",
    "MarginInputs",
    "ExposureInputs",
];

/// Findings for one file: a guarded type that derives or implements `Default`. Works on the whole
/// comment-free text, so a rustfmt-wrapped multi-line derive and a same-line
/// `#[derive(Default)] struct X` are both seen.
fn default_findings(name: &str, text: &str) -> Vec<String> {
    let code: String = code_lines(text)
        .into_iter()
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n");
    let mut hits = Vec::new();
    let words: Vec<&str> = idents(&code).collect();
    for (i, w) in words.iter().enumerate() {
        if matches!(*w, "struct" | "enum") {
            let Some(ty) = words.get(i + 1).filter(|t| GUARDED.contains(t)) else {
                continue;
            };
            // Attributes sit between the previous item's end and this keyword: take the text
            // before the keyword back to the last `;`, `}` or `{`.
            let at = code.find(&format!("{w} {ty}")).unwrap_or(0);
            let head = &code[..at];
            let start = head.rfind([';', '}', '{']).map_or(0, |p| p + 1);
            let attrs = &head[start..];
            if idents(attrs).any(|a| a == "Default") {
                hits.push(format!("{name}: derive(Default) on {ty}"));
            }
        }
        if *w == "impl"
            && words.get(i + 1) == Some(&"Default")
            && words.get(i + 2) == Some(&"for")
            && words.get(i + 3).is_some_and(|t| GUARDED.contains(t))
        {
            hits.push(format!("{name}: impl Default for {}", words[i + 3]));
        }
    }
    hits
}

#[test]
fn no_derive_default_on_snapshot_intent_or_ctx() {
    let hits: Vec<String> = sources()
        .iter()
        .flat_map(|(name, text)| default_findings(name, text))
        .collect();
    assert!(hits.is_empty(), "{hits:#?}");
}

#[test]
fn the_default_scan_sees_wrapped_and_same_line_derives() {
    let wrapped = "#[derive(\n    Debug,\n    Default,\n)]\npub struct PortfolioSnapshot {}\n";
    assert_eq!(default_findings("t", wrapped).len(), 1);
    let same = "#[derive(Default)] pub struct EventFlags {}\n";
    assert_eq!(default_findings("t", same).len(), 1);
    let manual = "impl Default for OrderIntent {\n}\n";
    assert_eq!(default_findings("t", manual).len(), 1);
    let fine =
        "#[derive(Debug, Clone)]\npub struct Position {}\n#[derive(Default)]\nstruct Other {}\n";
    assert_eq!(default_findings("t", fine), Vec::<String>::new());
    // A Default derive on the previous item does not leak onto the next one.
    let prev = "#[derive(Default)]\nstruct Other {}\npub struct Position {}\n";
    assert_eq!(default_findings("t", prev), Vec::<String>::new());
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
    // Library code only: the fields are private (checked below, and by the `compile_fail`
    // doctests on `GatedIntent`), so other modules and crates cannot construct it.
    let mut hits = Vec::new();
    let mut in_gate = 0;
    for (name, text) in sources().into_iter().filter(|(n, _)| n.starts_with("src/")) {
        let mut in_impl = false;
        for (n, line) in code_lines(&text) {
            // Whitespace removed so `GatedIntent  {` cannot slip past.
            let sq: String = line.split_whitespace().collect();
            if sq.starts_with("implGatedIntent{") {
                in_impl = true;
                continue;
            }
            if in_impl && line.starts_with('}') {
                in_impl = false;
            }
            let declaration = sq.contains("structGatedIntent{");
            let return_type = sq.contains("->GatedIntent{")
                || sq.contains("->Option<GatedIntent>{")
                || sq.contains("->Result<GatedIntent");
            let built = (sq.contains("GatedIntent{") && !declaration && !return_type)
                || (in_impl && sq.contains("Self{") && !sq.contains("->Self{"));
            if built {
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
fn gated_intent_has_no_public_fields() {
    let gate = fs::read_to_string(root().join("src/gate.rs")).unwrap();
    let code: Vec<String> = code_lines(&gate).into_iter().map(|(_, l)| l).collect();
    let start = code
        .iter()
        .position(|l| l.contains("pub struct GatedIntent"))
        .expect("GatedIntent declared");
    let body: Vec<&String> = code[start + 1..]
        .iter()
        .take_while(|l| l.trim() != "}")
        .collect();
    assert_ne!(body.len(), 0);
    let public: Vec<&&String> = body
        .iter()
        .filter(|l| l.trim_start().starts_with("pub "))
        .collect();
    assert!(public.is_empty(), "{public:?}");
}

#[test]
fn the_scanner_helpers_behave() {
    let lines = code_lines("// force\nlet skip_gate = 1;\n");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0, 2, "comment lines are dropped, numbers kept");
    assert!(idents("let x: f64 = 1.0;").any(|w| w == "f64"));
}
