//! `RegimeParams`: the `[regime]` table of `config/strategies.toml` (spec section 4).
//! Every default is an UNVERIFIED convention; no threshold literal lives in classifier code.

use bazaar_core::config::StrategiesConfig;
use serde::Deserialize;

/// Where market volatility comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketVolSource {
    /// India VIX percentile when present, else index ATR%.
    VixIfAvailable,
    /// Always index ATR%.
    IndexAtr,
}

/// Why `[regime]` could not be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParamsError {
    #[error("[regime] is missing from strategies.toml")]
    Missing,
    #[error("[regime] does not parse: {0}")]
    Parse(String),
    #[error("[regime].{key}: {reason}")]
    Invalid {
        key: &'static str,
        reason: &'static str,
    },
}

/// Classifier definitions. Not fitted parameters: never tune on strategy P&L.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegimeParams {
    pub ema_mid: usize,
    pub ema_slow: usize,
    pub adx_period: usize,
    pub adx_trend_enter: f64,
    pub adx_trend_stay: f64,
    pub adx_range_max: f64,
    pub slope_bars: usize,
    pub slope_flat_max_pct: f64,
    pub confirm_bars: usize,
    pub vol_lookback_bars: usize,
    pub vol_low_pct: f64,
    pub vol_high_pct: f64,
    pub breadth_breakdown_max_pct: f64,
    pub market_vol_source: MarketVolSource,
}

impl Default for RegimeParams {
    /// Spec section 4 defaults, mirrored by `config/strategies.toml` (UNVERIFIED conventions).
    fn default() -> Self {
        Self {
            ema_mid: 50,
            ema_slow: 200,
            adx_period: 14,
            adx_trend_enter: 25.0,
            adx_trend_stay: 20.0,
            adx_range_max: 20.0,
            slope_bars: 10,
            slope_flat_max_pct: 2.0,
            confirm_bars: 3,
            vol_lookback_bars: 250,
            vol_low_pct: 25.0,
            vol_high_pct: 75.0,
            breadth_breakdown_max_pct: 30.0,
            market_vol_source: MarketVolSource::VixIfAvailable,
        }
    }
}

fn bad(key: &'static str, reason: &'static str) -> ParamsError {
    ParamsError::Invalid { key, reason }
}

fn pct(v: f64) -> bool {
    v.is_finite() && v > 0.0 && v < 100.0
}

impl RegimeParams {
    /// Parse and validate the `[regime]` table of a loaded `strategies.toml`.
    pub fn from_config(cfg: &StrategiesConfig) -> Result<Self, ParamsError> {
        let table = cfg.regime.clone().ok_or(ParamsError::Missing)?;
        let p: Self = table
            .try_into()
            .map_err(|e: toml::de::Error| ParamsError::Parse(e.message().to_owned()))?;
        p.validate()?;
        Ok(p)
    }

    /// Sanity bounds (wider than the spec's stability-sweep ranges, which are for checks, not limits).
    pub fn validate(&self) -> Result<(), ParamsError> {
        if !(2..=400).contains(&self.ema_mid) {
            return Err(bad("ema_mid", "must be 2..=400"));
        }
        if !(2..=400).contains(&self.ema_slow) || self.ema_slow <= self.ema_mid {
            return Err(bad("ema_slow", "must be 2..=400 and above ema_mid"));
        }
        if !(2..=100).contains(&self.adx_period) {
            return Err(bad("adx_period", "must be 2..=100"));
        }
        if !pct(self.adx_trend_enter) || !pct(self.adx_trend_stay) || !pct(self.adx_range_max) {
            return Err(bad("adx_*", "must be between 0 and 100 exclusive"));
        }
        if self.adx_trend_stay > self.adx_trend_enter {
            return Err(bad("adx_trend_stay", "must not exceed adx_trend_enter"));
        }
        if self.adx_range_max > self.adx_trend_enter {
            return Err(bad("adx_range_max", "must not exceed adx_trend_enter"));
        }
        if !(1..=100).contains(&self.slope_bars) {
            return Err(bad("slope_bars", "must be 1..=100"));
        }
        if !(self.slope_flat_max_pct.is_finite()
            && self.slope_flat_max_pct > 0.0
            && self.slope_flat_max_pct <= 20.0)
        {
            return Err(bad("slope_flat_max_pct", "must be in (0, 20]"));
        }
        if !(1..=20).contains(&self.confirm_bars) {
            return Err(bad("confirm_bars", "must be 1..=20"));
        }
        if !(20..=1000).contains(&self.vol_lookback_bars) {
            return Err(bad("vol_lookback_bars", "must be 20..=1000"));
        }
        if !pct(self.vol_low_pct)
            || !pct(self.vol_high_pct)
            || self.vol_low_pct >= self.vol_high_pct
        {
            return Err(bad(
                "vol_low_pct",
                "need 0 < vol_low_pct < vol_high_pct < 100",
            ));
        }
        if !pct(self.breadth_breakdown_max_pct) {
            return Err(bad(
                "breadth_breakdown_max_pct",
                "must be between 0 and 100 exclusive",
            ));
        }
        Ok(())
    }

    /// Bars needed before any label: `ema_slow + vol_lookback_bars + confirm_bars` (spec section 3).
    pub fn min_history_bars(&self) -> usize {
        self.ema_slow
            .saturating_add(self.vol_lookback_bars)
            .saturating_add(self.confirm_bars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazaar_core::config::{load, parse_str};
    use std::path::Path;

    fn cfg(body: &str) -> StrategiesConfig {
        let t = format!("schema_version = 1\nsource = \"x\"\nverified_on = \"2026-11-20\"\n{body}");
        parse_str(&t).expect("fixture parses")
    }

    fn real_regime_text() -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/strategies.toml");
        let text = std::fs::read_to_string(p).expect("config readable");
        let start = text.find("\n[regime]\n").expect("has [regime]") + 1;
        text[start..].to_owned()
    }

    #[test]
    fn real_config_loads_and_equals_defaults() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let c: StrategiesConfig = load(&dir).expect("strategies.toml loads");
        assert!(!c.placeholder);
        assert_eq!(
            c.verified_on.map(|d| d.to_string()).as_deref(),
            Some("2026-11-20")
        );
        assert!(
            c.source
                .starts_with("owner decision 2026-11-20: UNVERIFIED")
        );
        let p = RegimeParams::from_config(&c).expect("params");
        assert_eq!(p, RegimeParams::default());
        assert_eq!(p.min_history_bars(), 453);
    }

    #[test]
    fn missing_table_is_an_error() {
        assert_eq!(
            RegimeParams::from_config(&cfg("")),
            Err(ParamsError::Missing)
        );
    }

    #[test]
    fn unknown_and_missing_keys_are_errors() {
        let extra = real_regime_text().replacen("[regime]\n", "[regime]\nfoo = 1\n", 1);
        assert!(matches!(
            RegimeParams::from_config(&cfg(&extra)),
            Err(ParamsError::Parse(_))
        ));
        assert!(matches!(
            RegimeParams::from_config(&cfg("[regime]\nema_mid = 50\n")),
            Err(ParamsError::Parse(_))
        ));
    }

    fn with(key: &str, val: &str) -> Result<RegimeParams, ParamsError> {
        let body: Vec<String> = real_regime_text()
            .lines()
            .map(|l| {
                if l.starts_with(&format!("{key} ")) {
                    format!("{key} = {val}")
                } else {
                    l.to_owned()
                }
            })
            .collect();
        RegimeParams::from_config(&cfg(&body.join("\n")))
    }

    #[test]
    fn out_of_range_values_are_errors() {
        for (k, v) in [
            ("ema_mid", "1"),
            ("ema_slow", "40"),
            ("adx_period", "0"),
            ("adx_trend_enter", "150.0"),
            ("adx_trend_stay", "30.0"),
            ("adx_range_max", "26.0"),
            ("slope_bars", "0"),
            ("slope_flat_max_pct", "-1.0"),
            ("confirm_bars", "0"),
            ("vol_lookback_bars", "5"),
            ("vol_low_pct", "80.0"),
            ("vol_high_pct", "100.0"),
            ("breadth_breakdown_max_pct", "0.0"),
            ("market_vol_source", "\"guess\""),
        ] {
            assert!(with(k, v).is_err(), "{k}={v} should be refused");
        }
        assert_eq!(with("confirm_bars", "4").unwrap().confirm_bars, 4);
        assert_eq!(
            with("market_vol_source", "\"index_atr\"")
                .unwrap()
                .market_vol_source,
            MarketVolSource::IndexAtr
        );
    }

    #[test]
    fn nan_is_refused() {
        let p = RegimeParams {
            adx_trend_enter: f64::NAN,
            ..RegimeParams::default()
        };
        assert!(p.validate().is_err());
    }
}
