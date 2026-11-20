//! `config/backtest.toml`: slippage, fill, walk-forward, evidence and trial policy.
//!
//! Every value is an owner-policy convention (UNVERIFIED), carried with a `source`. Unknown keys,
//! unknown slippage classes, TOML floats for decimals and out-of-range values fail to load with a
//! typed error. Tightening is always allowed; the floors below only stop loosening.

use std::path::Path;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::types::{HistoryPolicy, SlippageClass};

/// File name inside the config directory.
pub const FILE: &str = "backtest.toml";
/// The only `schema_version` understood.
pub const SCHEMA_VERSION: u32 = 1;

/// Hard floors equal to `risk.toml` `[gate.evidence]` (G01); a backtest config may tighten, never loosen.
const FLOOR_MIN_OOS_TRADES: u32 = 30;
const FLOOR_PROFIT_FACTOR: &str = "1.3";
const FLOOR_EXPECTANCY_R: &str = "0.1";

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
    /// Extra profit factor the bar needs for `trials` distinct parameter sets (1 = none).
    #[must_use]
    pub fn profit_factor_penalty(&self, trials: u32) -> Decimal {
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

    /// True when the label is capped at Provisional (more than `tier1_max_trials`).
    #[must_use]
    pub const fn caps_label(&self, trials: u32) -> bool {
        trials > self.tier1_max_trials
    }
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestConfig {
    pub schema_version: u32,
    pub source: String,
    pub verified_on: NaiveDate,
    pub run: RunConfig,
    pub slippage: SlippageTable,
    pub fills: Fills,
    pub walk_forward: WalkForward,
    pub evidence: Evidence,
    pub trials: Trials,
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
        for (name, s) in [
            ("slippage.equity", &self.slippage.equity),
            ("slippage.index_futures", &self.slippage.index_futures),
            ("slippage.index_options", &self.slippage.index_options),
        ] {
            require_source(&format!("{name}.source"), &s.source)?;
            non_negative(&format!("{name}.entry_pct"), s.entry_pct)?;
            non_negative(&format!("{name}.stop_pct"), s.stop_pct)?;
            if s.entry_pct > Decimal::ONE_HUNDRED || s.stop_pct > Decimal::ONE_HUNDRED {
                return Err(invalid(name, "pct must be at most 100"));
            }
            // A class with neither a percent nor a tick floor would model free fills.
            if s.entry_pct.is_zero() && s.entry_min_ticks == 0 {
                return Err(invalid(name, "entry slippage must not be zero"));
            }
            if s.stop_pct.is_zero() && s.stop_min_ticks == 0 {
                return Err(invalid(name, "stop slippage must not be zero"));
            }
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
        if f.target_through_ticks == 0 {
            return Err(invalid("fills.target_through_ticks", "must be at least 1"));
        }
        if f.stress_slippage_multiplier < Decimal::TWO {
            return Err(invalid(
                "fills.stress_slippage_multiplier",
                "must be at least 2",
            ));
        }
        Ok(())
    }

    fn validate_walk_forward(&self) -> Result<(), BacktestConfigError> {
        let w = &self.walk_forward;
        require_source("walk_forward.source", &w.source)?;
        if w.in_sample_months == 0 || w.out_of_sample_months == 0 {
            return Err(invalid("walk_forward", "window lengths must be positive"));
        }
        if w.step_months < w.out_of_sample_months {
            return Err(invalid(
                "walk_forward.step_months",
                "must be at least out_of_sample_months (OOS windows must not overlap)",
            ));
        }
        if w.min_folds == 0 {
            return Err(invalid("walk_forward.min_folds", "must be at least 1"));
        }
        if w.min_pooled_oos_trades < FLOOR_MIN_OOS_TRADES {
            return Err(invalid(
                "walk_forward.min_pooled_oos_trades",
                "below the risk.toml floor of 30; loosening needs the owner",
            ));
        }
        Ok(())
    }

    fn validate_evidence(&self) -> Result<(), BacktestConfigError> {
        let e = &self.evidence;
        require_source("evidence.source", &e.source)?;
        if e.min_profit_factor < decimal_const(FLOOR_PROFIT_FACTOR) {
            return Err(invalid(
                "evidence.min_profit_factor",
                "below the risk.toml floor of 1.3",
            ));
        }
        if e.min_expectancy_r < decimal_const(FLOOR_EXPECTANCY_R) {
            return Err(invalid(
                "evidence.min_expectancy_r",
                "below the risk.toml floor of 0.1",
            ));
        }
        if e.min_oos_is_ratio <= Decimal::ZERO {
            return Err(invalid("evidence.min_oos_is_ratio", "must be positive"));
        }
        if e.min_profitable_fold_pct <= Decimal::ZERO
            || e.min_profitable_fold_pct > Decimal::ONE_HUNDRED
        {
            return Err(invalid(
                "evidence.min_profitable_fold_pct",
                "must be in (0, 100]",
            ));
        }
        if e.max_drawdown_pct <= Decimal::ZERO || e.max_drawdown_pct > Decimal::ONE_HUNDRED {
            return Err(invalid("evidence.max_drawdown_pct", "must be in (0, 100]"));
        }
        if e.provisional_min_trades < self.walk_forward.min_pooled_oos_trades {
            return Err(invalid(
                "evidence.provisional_min_trades",
                "must be at least walk_forward.min_pooled_oos_trades",
            ));
        }
        if e.trusted_min_trades <= e.provisional_min_trades {
            return Err(invalid(
                "evidence.trusted_min_trades",
                "must exceed provisional_min_trades",
            ));
        }
        if e.trusted_min_regimes == 0 {
            return Err(invalid(
                "evidence.trusted_min_regimes",
                "must be at least 1",
            ));
        }
        Ok(())
    }

    fn validate_trials(&self) -> Result<(), BacktestConfigError> {
        let t = &self.trials;
        require_source("trials.source", &t.source)?;
        if t.tier1_max_trials < 2 || t.tier2_max_trials <= t.tier1_max_trials {
            return Err(invalid("trials", "tiers must satisfy 2 <= tier1 < tier2"));
        }
        non_negative("trials.tier1_pf_add", t.tier1_pf_add)?;
        if t.tier2_pf_add < t.tier1_pf_add || t.tier3_pf_add < t.tier2_pf_add {
            return Err(invalid(
                "trials",
                "the penalty must not fall as trials grow",
            ));
        }
        Ok(())
    }
}

fn decimal_const(s: &str) -> Decimal {
    s.parse().unwrap_or(Decimal::MAX)
}

fn require_source(key: &str, s: &str) -> Result<(), BacktestConfigError> {
    if s.trim().is_empty() {
        Err(invalid(key, "must not be empty"))
    } else {
        Ok(())
    }
}

fn non_negative(key: &str, d: Decimal) -> Result<(), BacktestConfigError> {
    if d < Decimal::ZERO {
        Err(invalid(key, "must not be negative"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/backtest.toml");
        std::fs::read_to_string(p).unwrap()
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn with(from: &str, to: &str) -> Result<BacktestConfig, BacktestConfigError> {
        let text = shipped();
        assert!(text.contains(from), "fixture edit target missing: {from}");
        BacktestConfig::parse_str(&text.replacen(from, to, 1))
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
    fn evidence_floors_match_risk_toml() {
        // The floors here must equal risk.toml [gate.evidence]; a drift fails this test.
        let risk = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/risk.toml"),
        )
        .unwrap();
        let line = |k: &str| {
            risk.lines()
                .skip_while(|l| !l.starts_with("[gate.evidence]"))
                .find(|l| l.starts_with(k))
                .unwrap()
                .to_owned()
        };
        assert!(line("min_oos_trades").contains(&FLOOR_MIN_OOS_TRADES.to_string()));
        assert!(line("min_profit_factor").contains(FLOOR_PROFIT_FACTOR));
        assert!(line("min_expectancy_r").contains(FLOOR_EXPECTANCY_R));
    }

    #[test]
    fn unknown_slippage_class_is_a_parse_error() {
        let text = format!("{}\n[slippage.crypto]\nsource = \"x\"\n", shipped());
        // appended after [trials], so the table is slippage.crypto: unknown, never a zero default
        assert!(matches!(
            BacktestConfig::parse_str(&text),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn missing_slippage_class_is_a_parse_error() {
        let text = shipped();
        let cut = text.find("[slippage.index_options]").unwrap();
        let end = text.find("[fills]").unwrap();
        let text = format!("{}{}", &text[..cut], &text[end..]);
        assert!(matches!(
            BacktestConfig::parse_str(&text),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn unknown_key_and_toml_float_fail() {
        assert!(matches!(
            with(
                "entry_min_ticks = 1\nstop_pct = \"0.10\"",
                "entry_min_ticks = 1\nextra = 1\nstop_pct = \"0.10\""
            ),
            Err(BacktestConfigError::Parse(_))
        ));
        assert!(matches!(
            with("entry_pct = \"0.05\"", "entry_pct = 0.05"),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn bad_header_is_rejected() {
        assert_eq!(
            key_of(with("schema_version = 1", "schema_version = 2")),
            "schema_version"
        );
        assert_eq!(
            key_of(with("source = \"owner policy: docs", "source = \" \" #")),
            "source"
        );
        let no_date = shipped().replacen("verified_on = \"2026-11-20\"", "", 1);
        assert!(matches!(
            BacktestConfig::parse_str(&no_date),
            Err(BacktestConfigError::Parse(_))
        ));
    }

    #[test]
    fn slippage_ranges_are_checked() {
        assert_eq!(
            key_of(with("entry_pct = \"0.05\"", "entry_pct = \"-1\"")),
            "slippage.equity.entry_pct"
        );
        assert_eq!(
            key_of(with("stop_pct = \"0.10\"", "stop_pct = \"0.01\"")),
            "slippage.equity"
        );
        assert_eq!(
            key_of(with(
                "entry_pct = \"0\"\nentry_min_ticks = 1",
                "entry_pct = \"0\"\nentry_min_ticks = 0"
            )),
            "slippage.index_futures"
        );
    }

    #[test]
    fn fills_must_keep_stress_and_through_ticks() {
        assert_eq!(
            key_of(with("target_through_ticks = 1", "target_through_ticks = 0")),
            "fills.target_through_ticks"
        );
        assert_eq!(
            key_of(with(
                "stress_slippage_multiplier = \"2\"",
                "stress_slippage_multiplier = \"1\""
            )),
            "fills.stress_slippage_multiplier"
        );
    }

    #[test]
    fn walk_forward_overlap_and_floor_are_rejected() {
        assert_eq!(
            key_of(with("step_months = 6 ", "step_months = 3 ")),
            "walk_forward.step_months"
        );
        assert_eq!(
            key_of(with(
                "min_pooled_oos_trades = 30 ",
                "min_pooled_oos_trades = 20 "
            )),
            "walk_forward.min_pooled_oos_trades"
        );
        assert_eq!(
            key_of(with("in_sample_months = 24", "in_sample_months = 0")),
            "walk_forward"
        );
        assert_eq!(
            key_of(with("min_folds = 4 ", "min_folds = 0 ")),
            "walk_forward.min_folds"
        );
    }

    #[test]
    fn evidence_may_tighten_but_not_loosen() {
        assert_eq!(
            key_of(with(
                "min_profit_factor = \"1.3\"",
                "min_profit_factor = \"1.2\""
            )),
            "evidence.min_profit_factor"
        );
        assert_eq!(
            key_of(with(
                "min_expectancy_r = \"0.1\"",
                "min_expectancy_r = \"0.05\""
            )),
            "evidence.min_expectancy_r"
        );
        assert!(with("min_profit_factor = \"1.3\"", "min_profit_factor = \"1.5\"").is_ok());
        assert_eq!(
            key_of(with("trusted_min_trades = 100", "trusted_min_trades = 30")),
            "evidence.trusted_min_trades"
        );
        assert_eq!(
            key_of(with(
                "min_profitable_fold_pct = \"60\"",
                "min_profitable_fold_pct = \"101\""
            )),
            "evidence.min_profitable_fold_pct"
        );
    }

    #[test]
    fn trial_tiers_must_be_ordered() {
        assert_eq!(
            key_of(with("tier2_max_trials = 50", "tier2_max_trials = 10")),
            "trials"
        );
        assert_eq!(
            key_of(with("tier3_pf_add = \"0.3\"", "tier3_pf_add = \"0.1\"")),
            "trials"
        );
    }

    #[test]
    fn trial_penalty_matches_policy_tiers() {
        let c = BacktestConfig::parse_str(&shipped()).unwrap();
        let t = &c.trials;
        assert_eq!(t.profit_factor_penalty(0), Decimal::ZERO);
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
    fn missing_file_is_an_io_error() {
        let dir = std::env::temp_dir().join("bazaar-backtest-no-such-dir");
        assert!(matches!(
            BacktestConfig::load(&dir),
            Err(BacktestConfigError::Io(_))
        ));
    }
}
