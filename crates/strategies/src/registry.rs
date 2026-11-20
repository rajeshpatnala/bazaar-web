//! The registry: the three strategy factories and the one call that builds the enabled ones from
//! `config/strategies.toml`. A new strategy is added here and nowhere else.

use bazaar_core::config::StrategiesConfig;

use crate::bollinger_reversion::BollingerFactory;
use crate::donchian_breakout::DonchianFactory;
use crate::factory::{StrategyFactory, build_enabled};
use crate::params::ParamError;
use crate::strategy::Strategy;
use crate::trend_pullback::TrendPullbackFactory;

/// Every strategy factory, in a fixed order.
#[must_use]
pub fn factories() -> [&'static dyn StrategyFactory; 3] {
    [&DonchianFactory, &TrendPullbackFactory, &BollingerFactory]
}

/// Build the enabled strategies from the config. Disabled ones are not built; an unknown
/// `[strategy.x]` name, an unknown param key or an out-of-range value is an error.
///
/// # Errors
/// `ParamError` from the first bad table.
pub fn build_all(cfg: &StrategiesConfig) -> Result<Vec<Box<dyn Strategy>>, ParamError> {
    build_enabled(cfg, &factories())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bollinger_reversion, donchian_breakout, trend_pullback};

    fn shipped() -> StrategiesConfig {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/strategies.toml");
        toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn the_registry_lists_the_three_strategies_once() {
        let ids: Vec<_> = factories().iter().map(|f| f.id()).collect();
        assert_eq!(
            ids,
            [
                donchian_breakout::ID,
                trend_pullback::ID,
                bollinger_reversion::ID
            ]
        );
    }

    #[test]
    fn shipped_config_builds_nothing_until_a_strategy_is_enabled() {
        let mut cfg = shipped();
        assert!(build_all(&cfg).unwrap().is_empty());
        for table in cfg.strategy.values_mut() {
            table.insert("enabled".into(), true.into());
        }
        let built = build_all(&cfg).unwrap();
        let ids: Vec<_> = built.iter().map(|s| s.meta().id).collect();
        assert_eq!(
            ids,
            [
                donchian_breakout::ID,
                trend_pullback::ID,
                bollinger_reversion::ID
            ]
        );
    }

    #[test]
    fn an_unknown_name_is_a_load_error() {
        let mut cfg = shipped();
        let table = cfg.strategy.get(donchian_breakout::ID).unwrap().clone();
        cfg.strategy.insert("donchian_breakoutt".into(), table);
        assert_eq!(
            build_all(&cfg).map(|v| v.len()).unwrap_err(),
            ParamError::UnknownStrategy("donchian_breakoutt".into())
        );
    }
}
