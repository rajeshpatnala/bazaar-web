//! The registry: the three strategy factories and the one call that builds the enabled ones from
//! `config/strategies.toml`. A new strategy is added here and nowhere else.
//!
//! `check_rr` measures reward:risk at the worst fill, `(target - entry_high) / (entry_high - stop)`
//! (QUAN-79-1). Every strategy still ships `enabled = false` until the owner turns one on.

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
}
