//! Building strategies from `config/strategies.toml`: enabled ones are built, disabled ones are not.

use bazaar_core::config::StrategiesConfig;

use crate::params::{ParamError, StrategyConfig, params_hash};
use crate::strategy::{Strategy, StrategyMeta};

/// Everything a factory gets besides the params table.
#[derive(Debug, Clone)]
pub struct BuildInput<'a> {
    /// `[strategy.<id>.params]`, not yet parsed (the factory parses it into its own struct).
    pub params: &'a toml::Table,
    pub params_hash: String,
    /// Regime cells and penalty weight, already checked.
    pub config: &'a StrategyConfig,
}

/// One factory per strategy; the registry (slice 5) lists them.
pub trait StrategyFactory {
    fn id(&self) -> &'static str;

    /// Build from validated config.
    ///
    /// # Errors
    /// `ParamError` for an unknown param key or an out-of-range value.
    fn build(&self, input: &BuildInput<'_>) -> Result<Box<dyn Strategy>, ParamError>;
}

/// Build every enabled strategy that has a factory. A `[strategy.x]` table with no factory is an
/// error (a typo must not silently disable a strategy); a factory with no table is skipped.
///
/// # Errors
/// The first bad table, bad regime declaration, bad params or unknown id.
pub fn build_enabled(
    cfg: &StrategiesConfig,
    factories: &[&dyn StrategyFactory],
) -> Result<Vec<Box<dyn Strategy>>, ParamError> {
    if let Some(id) = cfg
        .strategy
        .keys()
        .find(|k| !factories.iter().any(|f| f.id() == k.as_str()))
    {
        return Err(ParamError::UnknownStrategy(id.clone()));
    }
    let mut out = Vec::new();
    for f in factories {
        let Some(table) = cfg.strategy.get(f.id()) else {
            continue;
        };
        let config = StrategyConfig::from_table(f.id(), table)?;
        if !config.enabled {
            continue;
        }
        let input = BuildInput {
            params: &config.params,
            params_hash: params_hash(f.id(), table)?,
            config: &config,
        };
        out.push(f.build(&input)?);
    }
    Ok(out)
}

/// `StrategyMeta` helper for factories: regime cells and penalty come from the checked config.
///
/// # Errors
/// The `RegimeFitError` from `RegimeFit::check`, as a `ParamError`.
pub fn meta_from(
    id: &'static str,
    name: &'static str,
    version: u32,
    min_history_bars: usize,
    config: &StrategyConfig,
) -> Result<StrategyMeta, ParamError> {
    Ok(StrategyMeta {
        id,
        name,
        version,
        min_history_bars,
        regime_fit: config.regime_fit(id)?,
        penalty_weight: config.penalty_weight,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::StrategyContext;
    use crate::exit::{ExitAction, ExitError, OpenPosition};
    use crate::signal::Signal;
    use bazaar_core::dataview::DataView;

    struct Stub(StrategyMeta);
    impl Strategy for Stub {
        fn meta(&self) -> &StrategyMeta {
            &self.0
        }
        fn on_bar(&mut self, _: &DataView<'_>, _: &StrategyContext<'_>) -> Option<Signal> {
            None
        }
        fn manage(
            &mut self,
            _: &DataView<'_>,
            _: &StrategyContext<'_>,
            _: &OpenPosition,
        ) -> Result<ExitAction, ExitError> {
            Ok(ExitAction::Hold)
        }
    }
    struct StubFactory(&'static str);
    impl StrategyFactory for StubFactory {
        fn id(&self) -> &'static str {
            self.0
        }
        fn build(&self, i: &BuildInput<'_>) -> Result<Box<dyn Strategy>, ParamError> {
            Ok(Box::new(Stub(meta_from(self.0, "stub", 1, 30, i.config)?)))
        }
    }

    fn cfg(a: bool, b: bool) -> StrategiesConfig {
        let t = |id: &str, on: bool| {
            format!(
                "[strategy.{id}]\nenabled = {on}\npenalty_weight = 0.5\nparams = {{}}\n\
                 [strategy.{id}.regime_fit]\nmarket_trends = [\"up\"]\nstock_cells = [\"up:normal\"]\n"
            )
        };
        let text = format!(
            "schema_version = 1\nsource = \"t\"\n{}{}",
            t("a", a),
            t("b", b)
        );
        toml::from_str(&text).unwrap()
    }

    #[test]
    fn enabled_strategies_are_built_and_disabled_ones_are_not() {
        let (a, b) = (StubFactory("a"), StubFactory("b"));
        let all: [&dyn StrategyFactory; 2] = [&a, &b];
        let built = build_enabled(&cfg(true, false), &all).unwrap();
        assert_eq!(built.len(), 1);
        let m = built.first().unwrap().meta();
        assert_eq!((m.id, m.penalty_weight, m.min_history_bars), ("a", 0.5, 30));
        assert!(build_enabled(&cfg(false, false), &all).unwrap().is_empty());
    }

    #[test]
    fn a_table_without_a_factory_is_an_error() {
        let a = StubFactory("a");
        let err = build_enabled(&cfg(false, false), &[&a])
            .map(|v| v.len())
            .unwrap_err();
        assert_eq!(err, ParamError::UnknownStrategy("b".into()));
    }
}
