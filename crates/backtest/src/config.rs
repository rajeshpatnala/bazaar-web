//! `config/backtest.toml`: slippage, fill, walk-forward, evidence and trial policy.
//!
//! Every value is an owner-policy convention (UNVERIFIED), carried with a `source`. Unknown keys,
//! unknown slippage classes, TOML floats for decimals and out-of-range values fail to load with a
//! typed error. Tightening is always allowed; `mod floor` holds the policy value of every knob and a
//! value below it (looser) fails to load.

use std::path::Path;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::types::{HistoryPolicy, SlippageClass};

/// File name inside the config directory.
pub const FILE: &str = "backtest.toml";
/// The only `schema_version` understood.
pub const SCHEMA_VERSION: u32 = 1;

/// Why `backtest.toml` could not be loaded.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BacktestConfigError {
    #[error("{FILE} could not be read: {0}")]
    Io(String),
    #[error("{FILE} is not valid for its schema: {0}")]
    Parse(String),
    #[error("{FILE}: {key}: {reason}")]
    Invalid { key: String, reason: String },
}

fn invalid(key: &str, reason: &str) -> BacktestConfigError {
    BacktestConfigError::Invalid {
        key: key.to_owned(),
        reason: reason.to_owned(),
    }
}

mod dec {
    use std::str::FromStr;

    use rust_decimal::Decimal;
    use serde::Deserialize;
    use serde::de::{Deserializer, Error};

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Decimal, D::Error> {
        let s = String::deserialize(d)?;
        Decimal::from_str(&s).map_err(|_| D::Error::custom("expected a decimal string"))
    }
}

/// `[run]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub history_policy: HistoryPolicy,
}

/// One `[slippage.<class>]` table. Percentages are percent of price (0.05 = 0.05%).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slippage {
    pub source: String,
    pub unverified: bool,
    #[serde(with = "dec")]
    pub entry_pct: Decimal,
    pub entry_min_ticks: u32,
    #[serde(with = "dec")]
    pub stop_pct: Decimal,
    pub stop_min_ticks: u32,
}

/// All slippage classes. A class missing from the file or unknown in it is an error.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlippageTable {
    pub equity: Slippage,
    pub index_futures: Slippage,
    pub index_options: Slippage,
}

impl SlippageTable {
    /// The table of one class.
    #[must_use]
    pub const fn for_class(&self, c: SlippageClass) -> &Slippage {
        match c {
            SlippageClass::Equity => &self.equity,
            SlippageClass::IndexFutures => &self.index_futures,
            SlippageClass::IndexOptions => &self.index_options,
        }
    }
}

/// `[fills]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fills {
    pub source: String,
    pub unverified: bool,
    pub target_through_ticks: u32,
    #[serde(with = "dec")]
    pub stress_slippage_multiplier: Decimal,
}

/// `[walk_forward]`: rolling windows, never anchored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkForward {
    pub source: String,
    pub unverified: bool,
    pub in_sample_months: u32,
    pub out_of_sample_months: u32,
    pub step_months: u32,
    pub warmup_bars: u32,
    pub min_folds: u32,
    pub min_pooled_oos_trades: u32,
    pub thin_fold_trades: u32,
}

/// `[evidence]`: the bar, all after costs and slippage.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: String,
    pub unverified: bool,
    #[serde(with = "dec")]
    pub min_profit_factor: Decimal,
    #[serde(with = "dec")]
    pub min_expectancy_r: Decimal,
    #[serde(with = "dec")]
    pub min_oos_is_ratio: Decimal,
    #[serde(with = "dec")]
    pub min_profitable_fold_pct: Decimal,
    pub provisional_min_trades: u32,
    pub trusted_min_trades: u32,
    pub trusted_min_regimes: u32,
    #[serde(with = "dec")]
    pub max_drawdown_pct: Decimal,
}

/// `[trials]`: the multiple-testing penalty. It only raises the bar.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trials {
    pub source: String,
    pub unverified: bool,
    pub prior_trials: u32,
    pub tier1_max_trials: u32,
    #[serde(with = "dec")]
    pub tier1_pf_add: Decimal,
    pub tier2_max_trials: u32,
    #[serde(with = "dec")]
    pub tier2_pf_add: Decimal,
    #[serde(with = "dec")]
    pub tier3_pf_add: Decimal,
}

impl Trials {
    /// Trials counted against the bar: the owner-declared `prior_trials` plus this run's
    /// distinct parameter sets (saturating). Callers pass only the in-run count.
    #[must_use]
    pub const fn total_trials(&self, in_run: u32) -> u32 {
        self.prior_trials.saturating_add(in_run)
    }

    /// Extra profit factor the bar needs after `in_run` new parameter sets, on top of
    /// `prior_trials` (a total of 0 or 1 = none).
    #[must_use]
    pub fn profit_factor_penalty(&self, in_run: u32) -> Decimal {
        let trials = self.total_trials(in_run);
        if trials <= 1 {
            Decimal::ZERO
        } else if trials <= self.tier1_max_trials {
            self.tier1_pf_add
        } else if trials <= self.tier2_max_trials {
            self.tier2_pf_add
        } else {
            self.tier3_pf_add
        }
    }

    /// True when the label is capped at Provisional (total above `tier1_max_trials`).
    #[must_use]
    pub const fn caps_label(&self, in_run: u32) -> bool {
        self.total_trials(in_run) > self.tier1_max_trials
    }
}

/// The whole file. `policy_date` is when the owner policy was written, not a source check:
/// every value stays an UNVERIFIED convention.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestConfig {
    pub schema_version: u32,
    pub source: String,
    pub policy_date: NaiveDate,
    pub run: RunConfig,
    pub slippage: SlippageTable,
    pub fills: Fills,
    pub walk_forward: WalkForward,
    pub evidence: Evidence,
    pub trials: Trials,
}

/// Policy values (docs/strategy/F9-backtest-policy.md). A config value looser than its floor
/// fails to load; tightening is allowed. The first three equal `risk.toml [gate.evidence]`
/// (a test compares them); the rest are owner conventions, UNVERIFIED.
mod floor {
    use rust_decimal::Decimal;

    pub(super) const MIN_OOS_TRADES: u32 = 30;
    pub(super) const PROFIT_FACTOR: Decimal = Decimal::from_parts(13, 0, 0, false, 1);
    pub(super) const EXPECTANCY_R: Decimal = Decimal::from_parts(1, 0, 0, false, 1);
    pub(super) const OOS_IS_RATIO: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
    pub(super) const PROFITABLE_FOLD_PCT: Decimal = Decimal::from_parts(60, 0, 0, false, 0);
    pub(super) const MAX_DRAWDOWN_PCT: Decimal = Decimal::from_parts(10, 0, 0, false, 0);
    pub(super) const STRESS_MULTIPLIER: Decimal = Decimal::TWO;
    pub(super) const IN_SAMPLE_MONTHS: u32 = 24;
    pub(super) const OOS_MONTHS: u32 = 6;
    pub(super) const MIN_FOLDS: u32 = 4;
    pub(super) const WARMUP_BARS: u32 = 250;
    pub(super) const THIN_FOLD_TRADES: u32 = 5;
    pub(super) const TRUSTED_TRADES: u32 = 100;
    pub(super) const TRUSTED_REGIMES: u32 = 2;
    pub(super) const TIER1_MAX: u32 = 10;
    pub(super) const TIER2_MAX: u32 = 50;
    pub(super) const TIER1_ADD: Decimal = Decimal::from_parts(1, 0, 0, false, 1);
    pub(super) const TIER2_ADD: Decimal = Decimal::from_parts(2, 0, 0, false, 1);
    pub(super) const TIER3_ADD: Decimal = Decimal::from_parts(3, 0, 0, false, 1);

    /// `(entry_pct, entry_min_ticks, stop_pct, stop_min_ticks)` per class.
    pub(super) const EQUITY: (Decimal, u32, Decimal, u32) = (
        Decimal::from_parts(5, 0, 0, false, 2),
        1,
        Decimal::from_parts(10, 0, 0, false, 2),
        1,
    );
    pub(super) const INDEX_FUTURES: (Decimal, u32, Decimal, u32) =
        (Decimal::ZERO, 1, Decimal::ZERO, 2);
    pub(super) const INDEX_OPTIONS: (Decimal, u32, Decimal, u32) =
        (Decimal::from_parts(5, 0, 0, false, 1), 2, Decimal::ONE, 3);
}

impl BacktestConfig {
    /// Parse and validate from text (the pure half of [`Self::load`]).
    pub fn parse_str(text: &str) -> Result<Self, BacktestConfigError> {
        let cfg: Self =
            toml::from_str(text).map_err(|e| BacktestConfigError::Parse(e.message().to_owned()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Read, parse and validate `dir/backtest.toml`.
    pub fn load(dir: &Path) -> Result<Self, BacktestConfigError> {
        let text = std::fs::read_to_string(dir.join(FILE))
            .map_err(|e| BacktestConfigError::Io(e.kind().to_string()))?;
        Self::parse_str(&text)
    }

    /// The slippage multiplier of the 2x stress test.
    #[must_use]
    pub const fn stress_multiplier(&self) -> Decimal {
        self.fills.stress_slippage_multiplier
    }

    fn validate(&self) -> Result<(), BacktestConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid("schema_version", "unsupported version"));
        }
        require_source("source", &self.source)?;
        // `latest_for_all` is a per-run opt-in (it marks the run approximate); the shared file
        // always says `strict`.
        if self.run.history_policy != HistoryPolicy::Strict {
            return Err(invalid(
                "run.history_policy",
                "must be strict in the file; latest_for_all is a per-run opt-in",
            ));
        }
        for (name, s, f) in [
            ("slippage.equity", &self.slippage.equity, floor::EQUITY),
            (
                "slippage.index_futures",
                &self.slippage.index_futures,
                floor::INDEX_FUTURES,
            ),
            (
                "slippage.index_options",
                &self.slippage.index_options,
                floor::INDEX_OPTIONS,
            ),
        ] {
            require_source(&format!("{name}.source"), &s.source)?;
            if s.entry_pct > Decimal::ONE_HUNDRED || s.stop_pct > Decimal::ONE_HUNDRED {
                return Err(invalid(name, "pct must be at most 100"));
            }
            at_least(&format!("{name}.entry_pct"), s.entry_pct, f.0)?;
            at_least(&format!("{name}.entry_min_ticks"), s.entry_min_ticks, f.1)?;
            at_least(&format!("{name}.stop_pct"), s.stop_pct, f.2)?;
            at_least(&format!("{name}.stop_min_ticks"), s.stop_min_ticks, f.3)?;
            if s.stop_min_ticks < s.entry_min_ticks || s.stop_pct < s.entry_pct {
                return Err(invalid(
                    name,
                    "stop slippage must not be below entry slippage",
                ));
            }
        }
        self.validate_fills()?;
        self.validate_walk_forward()?;
        self.validate_evidence()?;
        self.validate_trials()
    }

    fn validate_fills(&self) -> Result<(), BacktestConfigError> {
        let f = &self.fills;
        require_source("fills.source", &f.source)?;
        at_least("fills.target_through_ticks", f.target_through_ticks, 1)?;
        at_least(
            "fills.stress_slippage_multiplier",
            f.stress_slippage_multiplier,
            floor::STRESS_MULTIPLIER,
        )
    }

    fn validate_walk_forward(&self) -> Result<(), BacktestConfigError> {
        let w = &self.walk_forward;
        require_source("walk_forward.source", &w.source)?;
        at_least(
            "walk_forward.in_sample_months",
            w.in_sample_months,
            floor::IN_SAMPLE_MONTHS,
        )?;
        at_least(
            "walk_forward.out_of_sample_months",
            w.out_of_sample_months,
            floor::OOS_MONTHS,
        )?;
        if w.step_months < w.out_of_sample_months {
            return Err(invalid(
                "walk_forward.step_months",
                "must be at least out_of_sample_months (OOS windows must not overlap)",
            ));
        }
        at_least(
            "walk_forward.warmup_bars",
            w.warmup_bars,
            floor::WARMUP_BARS,
        )?;
        at_least("walk_forward.min_folds", w.min_folds, floor::MIN_FOLDS)?;
        at_least(
            "walk_forward.min_pooled_oos_trades",
            w.min_pooled_oos_trades,
            floor::MIN_OOS_TRADES,
        )?;
        at_least(
            "walk_forward.thin_fold_trades",
            w.thin_fold_trades,
            floor::THIN_FOLD_TRADES,
        )?;
        if w.thin_fold_trades > w.min_pooled_oos_trades {
            return Err(invalid(
                "walk_forward.thin_fold_trades",
                "must not exceed min_pooled_oos_trades",
            ));
        }
        Ok(())
    }

    fn validate_evidence(&self) -> Result<(), BacktestConfigError> {
        let e = &self.evidence;
        require_source("evidence.source", &e.source)?;
        at_least(
            "evidence.min_profit_factor",
            e.min_profit_factor,
            floor::PROFIT_FACTOR,
        )?;
        at_least(
            "evidence.min_expectancy_r",
            e.min_expectancy_r,
            floor::EXPECTANCY_R,
        )?;
        at_least(
            "evidence.min_oos_is_ratio",
            e.min_oos_is_ratio,
            floor::OOS_IS_RATIO,
        )?;
        at_least(
            "evidence.min_profitable_fold_pct",
            e.min_profitable_fold_pct,
            floor::PROFITABLE_FOLD_PCT,
        )?;
        if e.min_profitable_fold_pct > Decimal::ONE_HUNDRED {
            return Err(invalid(
                "evidence.min_profitable_fold_pct",
                "must be at most 100",
            ));
        }
        // Drawdown is a ceiling: a lower number is tighter.
        if e.max_drawdown_pct <= Decimal::ZERO || e.max_drawdown_pct > floor::MAX_DRAWDOWN_PCT {
            return Err(invalid(
                "evidence.max_drawdown_pct",
                "must be in (0, 10]; a higher ceiling is looser than the policy",
            ));
        }
        at_least(
            "evidence.provisional_min_trades",
            e.provisional_min_trades,
            self.walk_forward.min_pooled_oos_trades,
        )?;
        at_least(
            "evidence.trusted_min_trades",
            e.trusted_min_trades,
            floor::TRUSTED_TRADES,
        )?;
        if e.trusted_min_trades <= e.provisional_min_trades {
            return Err(invalid(
                "evidence.trusted_min_trades",
                "must exceed provisional_min_trades",
            ));
        }
        at_least(
            "evidence.trusted_min_regimes",
            e.trusted_min_regimes,
            floor::TRUSTED_REGIMES,
        )
    }

    fn validate_trials(&self) -> Result<(), BacktestConfigError> {
        let t = &self.trials;
        require_source("trials.source", &t.source)?;
        // Lower tier boundaries are tighter (the penalty starts sooner).
        if t.tier1_max_trials < 2 || t.tier1_max_trials > floor::TIER1_MAX {
            return Err(invalid("trials.tier1_max_trials", "must be in 2..=10"));
        }
        if t.tier2_max_trials <= t.tier1_max_trials || t.tier2_max_trials > floor::TIER2_MAX {
            return Err(invalid(
                "trials.tier2_max_trials",
                "must exceed tier1_max_trials and be at most 50",
            ));
        }
        at_least("trials.tier1_pf_add", t.tier1_pf_add, floor::TIER1_ADD)?;
        at_least("trials.tier2_pf_add", t.tier2_pf_add, floor::TIER2_ADD)?;
        at_least("trials.tier3_pf_add", t.tier3_pf_add, floor::TIER3_ADD)?;
        if t.tier2_pf_add < t.tier1_pf_add || t.tier3_pf_add < t.tier2_pf_add {
            return Err(invalid(
                "trials",
                "the penalty must not fall as trials grow",
            ));
        }
        Ok(())
    }
}

fn require_source(key: &str, s: &str) -> Result<(), BacktestConfigError> {
    if s.trim().is_empty() {
        Err(invalid(key, "must not be empty"))
    } else {
        Ok(())
    }
}

/// `value >= floor`, else a typed "looser than the policy" error.
#[allow(clippy::needless_pass_by_value)] // generic over Copy numbers and Decimal
fn at_least<T: PartialOrd>(key: &str, value: T, floor: T) -> Result<(), BacktestConfigError> {
    if value < floor {
        Err(invalid(
            key,
            "below the policy value (looser); loosening needs the owner",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml::Value;

    fn shipped() -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/backtest.toml");
        std::fs::read_to_string(p).unwrap()
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn s(v: &str) -> Value {
        Value::String(v.to_owned())
    }

    fn i(v: i64) -> Value {
        Value::Integer(v)
    }

    /// Shipped file with `path` (dotted table path, "" for the top level) / `key` set to `val`
    /// (or removed for `None`), re-serialised so edits do not depend on column padding.
    fn edited(path: &str, key: &str, val: Option<Value>) -> String {
        let mut root: toml::Table = shipped().parse().unwrap();
        let mut t = &mut root;
        for part in path.split('.').filter(|p| !p.is_empty()) {
            t = t.get_mut(part).unwrap().as_table_mut().unwrap();
        }
        assert!(t.contains_key(key), "no such key {path}.{key}");
        match val {
            Some(v) => t.insert(key.to_owned(), v),
            None => t.remove(key),
        };
        toml::to_string(&root).unwrap()
    }

    fn with(path: &str, key: &str, val: Value) -> Result<BacktestConfig, BacktestConfigError> {
        BacktestConfig::parse_str(&edited(path, key, Some(val)))
    }

    fn key_of(r: Result<BacktestConfig, BacktestConfigError>) -> String {
        match r {
            Err(BacktestConfigError::Invalid { key, .. }) => key,
            #[allow(clippy::panic)]
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn shipped_file_loads_with_policy_numbers() {
        let c = BacktestConfig::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config"))
            .unwrap();
        assert_eq!(c.run.history_policy, HistoryPolicy::Strict);
        let eq = c.slippage.for_class(SlippageClass::Equity);
        assert_eq!((eq.entry_pct, eq.entry_min_ticks), (d("0.05"), 1));
        assert_eq!((eq.stop_pct, eq.stop_min_ticks), (d("0.10"), 1));
        let fut = c.slippage.for_class(SlippageClass::IndexFutures);
        assert_eq!((fut.entry_min_ticks, fut.stop_min_ticks), (1, 2));
        let opt = c.slippage.for_class(SlippageClass::IndexOptions);
        assert_eq!((opt.entry_pct, opt.entry_min_ticks), (d("0.5"), 2));
        assert_eq!((opt.stop_pct, opt.stop_min_ticks), (d("1"), 3));
        assert_eq!(c.stress_multiplier(), d("2"));
        let w = &c.walk_forward;
        assert_eq!(
            (w.in_sample_months, w.out_of_sample_months, w.step_months),
            (24, 6, 6)
        );
        assert_eq!(
            (w.min_folds, w.min_pooled_oos_trades, w.thin_fold_trades),
            (4, 30, 5)
        );
        assert_eq!(c.evidence.min_profit_factor, d("1.3"));
        assert_eq!(c.evidence.min_expectancy_r, d("0.1"));
        assert_eq!(c.evidence.min_profitable_fold_pct, d("60"));
        assert_eq!(c.trials.prior_trials, 0);
        assert!(c.slippage.equity.unverified && c.walk_forward.unverified);
    }

    #[test]
    fn shipped_values_equal_their_floors() {
        // The shipped file is exactly the policy: every floor constant matches it.
        let c = BacktestConfig::parse_str(&shipped()).unwrap();
        assert_eq!(c.evidence.min_oos_is_ratio, floor::OOS_IS_RATIO);
        assert_eq!(c.evidence.max_drawdown_pct, floor::MAX_DRAWDOWN_PCT);
        assert_eq!(c.walk_forward.warmup_bars, floor::WARMUP_BARS);
        assert_eq!(c.trials.tier1_pf_add, floor::TIER1_ADD);
        assert_eq!(c.trials.tier2_max_trials, floor::TIER2_MAX);
    }

    #[test]
    fn evidence_floors_match_risk_toml() {
        // Parsed, not substring-matched: the gate's [gate.evidence] values must equal the floors.
        let risk = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/risk.toml"),
        )
        .unwrap();
        let risk: toml::Table = risk.parse().unwrap();
        let ev = risk["gate"]["evidence"].as_table().unwrap();
        let dec = |k: &str| -> Decimal { ev[k].as_str().unwrap().parse().unwrap() };
        assert_eq!(
            ev["min_oos_trades"].as_integer().unwrap(),
            i64::from(floor::MIN_OOS_TRADES)
        );
        assert_eq!(dec("min_profit_factor"), floor::PROFIT_FACTOR);
        assert_eq!(dec("min_expectancy_r"), floor::EXPECTANCY_R);
    }

    #[test]
    fn unknown_slippage_class_is_a_parse_error() {
        let mut root: toml::Table = shipped().parse().unwrap();
        let eq = root["slippage"]["equity"].clone();
        root["slippage"]
            .as_table_mut()
            .unwrap()
            .insert("crypto".into(), eq);
        assert!(matches!(
            BacktestConfig::parse_str(&toml::to_string(&root).unwrap()),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn missing_slippage_class_is_a_parse_error() {
        let mut root: toml::Table = shipped().parse().unwrap();
        root["slippage"]
            .as_table_mut()
            .unwrap()
            .remove("index_options");
        assert!(matches!(
            BacktestConfig::parse_str(&toml::to_string(&root).unwrap()),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn unknown_key_and_toml_float_fail() {
        let mut root: toml::Table = shipped().parse().unwrap();
        root["fills"]
            .as_table_mut()
            .unwrap()
            .insert("extra".into(), i(1));
        assert!(matches!(
            BacktestConfig::parse_str(&toml::to_string(&root).unwrap()),
            Err(BacktestConfigError::Parse(_))
        ));
        assert!(matches!(
            with("slippage.equity", "entry_pct", Value::Float(0.05)),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn bad_header_is_rejected() {
        assert_eq!(key_of(with("", "schema_version", i(2))), "schema_version");
        assert_eq!(key_of(with("", "source", s(" "))), "source");
        assert!(matches!(
            BacktestConfig::parse_str(&edited("", "policy_date", None)),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn latest_for_all_is_refused_in_the_shared_file() {
        assert_eq!(
            key_of(with("run", "history_policy", s("latest_for_all"))),
            "run.history_policy"
        );
    }

    #[test]
    fn slippage_below_policy_is_refused_for_every_class() {
        let cases = [
            ("slippage.equity", "entry_pct", s("0.0001")),
            ("slippage.equity", "stop_pct", s("0.01")),
            ("slippage.equity", "entry_min_ticks", i(0)),
            ("slippage.equity", "stop_min_ticks", i(0)),
            ("slippage.index_futures", "entry_min_ticks", i(0)),
            ("slippage.index_futures", "stop_min_ticks", i(1)),
            ("slippage.index_options", "entry_pct", s("0.1")),
            ("slippage.index_options", "stop_pct", s("0.5")),
            ("slippage.index_options", "entry_min_ticks", i(1)),
            ("slippage.index_options", "stop_min_ticks", i(2)),
        ];
        for (path, key, v) in cases {
            assert_eq!(key_of(with(path, key, v)), format!("{path}.{key}"));
        }
        assert_eq!(
            key_of(with("slippage.equity", "entry_pct", s("-1"))),
            "slippage.equity.entry_pct"
        );
        // tightening is fine
        assert!(with("slippage.equity", "entry_pct", s("0.10")).is_ok());
        // stop below entry is refused
        assert_eq!(
            key_of(with("slippage.equity", "entry_pct", s("0.5"))),
            "slippage.equity"
        );
    }

    #[test]
    fn fills_must_keep_stress_and_through_ticks() {
        assert_eq!(
            key_of(with("fills", "target_through_ticks", i(0))),
            "fills.target_through_ticks"
        );
        assert_eq!(
            key_of(with("fills", "stress_slippage_multiplier", s("1"))),
            "fills.stress_slippage_multiplier"
        );
    }

    #[test]
    fn walk_forward_loosening_is_refused() {
        let cases = [
            ("step_months", i(3)),
            ("min_pooled_oos_trades", i(20)),
            ("in_sample_months", i(12)),
            ("out_of_sample_months", i(3)),
            ("min_folds", i(1)),
            ("warmup_bars", i(0)),
            ("thin_fold_trades", i(0)),
        ];
        for (key, v) in cases {
            assert_eq!(
                key_of(with("walk_forward", key, v)),
                format!("walk_forward.{key}")
            );
        }
        // thin above the pooled gate is incoherent
        assert_eq!(
            key_of(with("walk_forward", "thin_fold_trades", i(31))),
            "walk_forward.thin_fold_trades"
        );
        assert!(with("walk_forward", "min_folds", i(6)).is_ok());
    }

    #[test]
    fn evidence_may_tighten_but_not_loosen() {
        let cases = [
            ("min_profit_factor", s("1.2")),
            ("min_expectancy_r", s("0.05")),
            ("min_oos_is_ratio", s("0.01")),
            ("min_profitable_fold_pct", s("1")),
            ("trusted_min_trades", i(31)),
            ("trusted_min_regimes", i(1)),
            ("max_drawdown_pct", s("11")),
            ("provisional_min_trades", i(10)),
        ];
        for (key, v) in cases {
            assert_eq!(key_of(with("evidence", key, v)), format!("evidence.{key}"));
        }
        assert!(with("evidence", "min_profit_factor", s("1.5")).is_ok());
        assert!(with("evidence", "max_drawdown_pct", s("8")).is_ok());
        assert_eq!(
            key_of(with("evidence", "min_profitable_fold_pct", s("101"))),
            "evidence.min_profitable_fold_pct"
        );
    }

    #[test]
    fn trial_tiers_must_be_ordered_and_not_loosened() {
        assert_eq!(
            key_of(with("trials", "tier2_max_trials", i(10))),
            "trials.tier2_max_trials"
        );
        assert_eq!(
            key_of(with("trials", "tier2_max_trials", i(60))),
            "trials.tier2_max_trials"
        );
        assert_eq!(
            key_of(with("trials", "tier1_max_trials", i(20))),
            "trials.tier1_max_trials"
        );
        assert_eq!(
            key_of(with("trials", "tier3_pf_add", s("0.1"))),
            "trials.tier3_pf_add"
        );
        assert_eq!(
            key_of(with("trials", "tier1_pf_add", s("0.05"))),
            "trials.tier1_pf_add"
        );
        // a penalty that falls as trials grow
        assert_eq!(key_of(with("trials", "tier1_pf_add", s("0.25"))), "trials");
    }

    #[test]
    fn trial_penalty_matches_policy_tiers() {
        let c = BacktestConfig::parse_str(&shipped()).unwrap();
        let t = &c.trials;
        // in-run 0 with prior_trials = 0 stays valid, no penalty
        assert_eq!(t.profit_factor_penalty(0), Decimal::ZERO);
        assert!(!t.caps_label(0));
        assert_eq!(t.profit_factor_penalty(1), Decimal::ZERO);
        assert_eq!(t.profit_factor_penalty(2), d("0.1"));
        assert_eq!(t.profit_factor_penalty(10), d("0.1"));
        assert_eq!(t.profit_factor_penalty(11), d("0.2"));
        assert_eq!(t.profit_factor_penalty(50), d("0.2"));
        assert_eq!(t.profit_factor_penalty(51), d("0.3"));
        assert!(!t.caps_label(10));
        assert!(t.caps_label(11));
    }

    #[test]
    fn prior_trials_count_toward_the_penalty() {
        let c = BacktestConfig::parse_str(&edited("trials", "prior_trials", Some(i(10)))).unwrap();
        let t = &c.trials;
        // 10 prior + 1 new = 11 total: the 11-50 tier, capped at Provisional
        assert_eq!(t.total_trials(1), 11);
        assert_eq!(t.profit_factor_penalty(1), d("0.2"));
        assert!(t.caps_label(1));
        // prior alone, nothing new in this run: still the 2-10 tier
        assert_eq!(t.profit_factor_penalty(0), d("0.1"));
        assert!(!t.caps_label(0));
        // saturating, never wraps
        assert_eq!(t.total_trials(u32::MAX), u32::MAX);
        assert_eq!(t.profit_factor_penalty(u32::MAX), d("0.3"));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let dir = std::env::temp_dir().join("bazaar-backtest-no-such-dir");
        assert!(matches!(
            BacktestConfig::load(&dir),
            Err(BacktestConfigError::Io(_))
        ));
    }
}
