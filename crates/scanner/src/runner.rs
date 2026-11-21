//! Runner: filters the universe, builds the regimes up to the closed decision bar, runs the
//! strategies on a `DataView` that ends there, and merges signals per instrument and family.
//! No scoring, sizing or gate here (steps 3-4). Bars after the decision date are never read.

use std::collections::BTreeMap;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::Candle;
use bazaar_core::clock::Clock;
use bazaar_core::config::{ScannerConfig, StrategiesConfig};
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::Instrument;
use bazaar_regime::{Extras, Fit, RegimeParams, RegimeState, Series, classify};
use bazaar_strategies::{ParamError, PortfolioView, Signal, Strategy, StrategyContext};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::ports::BanList;
use crate::universe::{DropReason, MEDIAN_WINDOW_BARS, banned_set, median_traded_value, normalise};

/// One name of the universe with its adjusted daily bars (oldest first).
#[derive(Debug, Clone)]
pub struct UniverseEntry {
    pub instrument: Instrument,
    pub bars: Vec<Candle>,
    /// From the day's data: the name is locked at a circuit limit and cannot be traded.
    pub circuit_locked: bool,
}

/// Why a scan could not run at all.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("strategies.toml has no [scanner] table")]
    NoScannerConfig,
    #[error("strategy config: {0}")]
    Strategies(#[from] ParamError),
    #[error("no market bar on the decision date {0}")]
    NoMarketBar(NaiveDate),
    #[error("market regime: {0}")]
    MarketRegime(String),
}

/// Everything the runner reads besides the universe.
pub struct ScanContext<'a> {
    pub cfg: &'a ScannerConfig,
    pub regime: &'a RegimeParams,
    pub calendar: &'a MarketCalendar,
    pub portfolio: &'a PortfolioView,
    pub ban_list: &'a dyn BanList,
    pub clock: &'a dyn Clock,
}

impl std::fmt::Debug for ScanContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanContext").finish_non_exhaustive()
    }
}

/// One surviving signal per family, with how the regime judged it.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub family: String,
    pub signal: Signal,
    /// `Allowed` or `Penalised`; a `Suppressed` cell never becomes a hit.
    pub fit: Fit,
    /// From the strategy's config; the scanner applies it to a `Penalised` regime component.
    pub penalty_weight: f64,
    /// Same-family strategies that also fired and were folded into this hit (counted once).
    pub merged_ids: Vec<&'static str>,
}

/// An instrument with at least one signal at the decision bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub instrument: Instrument,
    pub hits: Vec<Hit>,
    pub market_regime: RegimeState,
    pub stock_regime: RegimeState,
    pub median_traded_value: Decimal,
}

/// Result of a scan: candidates and every drop with its reason. Ordered by tradingsymbol.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScanOutput {
    pub candidates: Vec<Candidate>,
    pub dropped: Vec<(String, DropReason)>,
}

/// Family of a strategy id (F8 spec table). An unmapped id is its own family, so it is never
/// folded into another.
#[must_use]
pub fn family_of(id: &str) -> &str {
    match id {
        "donchian_breakout" => "breakout",
        "trend_pullback" => "trend",
        "bollinger_reversion" => "reversion",
        other => other,
    }
}

/// Enabled strategies plus the `[scanner]` table they were built beside.
pub type Built<'a> = (Vec<Box<dyn Strategy>>, &'a ScannerConfig);

/// Build the enabled strategies from the config and return them with the `[scanner]` table.
///
/// # Errors
/// `NoScannerConfig` or the first bad strategy table.
pub fn build(cfg: &StrategiesConfig) -> Result<Built<'_>, ScanError> {
    let scanner = cfg.scanner.as_ref().ok_or(ScanError::NoScannerConfig)?;
    Ok((bazaar_strategies::registry::build_all(cfg)?, scanner))
}

fn upto_date(bars: &[Candle], date: NaiveDate) -> Option<usize> {
    bars.iter()
        .rposition(|b| b.close_time().date_naive() == date)
}

/// Run `strategies` over `universe` at the closed bar of `decision_date`.
///
/// # Errors
/// No market bar on that date, or the market regime cannot be built.
pub fn run(
    cx: &ScanContext<'_>,
    strategies: &mut [Box<dyn Strategy>],
    universe: &[UniverseEntry],
    market_bars: &[Candle],
    decision_date: NaiveDate,
) -> Result<ScanOutput, ScanError> {
    let m_end =
        upto_date(market_bars, decision_date).ok_or(ScanError::NoMarketBar(decision_date))?;
    let m_bars = market_bars.get(..=m_end).unwrap_or(market_bars);
    let market = classify(m_bars, cx.regime, Series::Market, Extras::default())
        .map_err(|e| ScanError::MarketRegime(e.to_string()))?;
    let banned = banned_set(
        &cx.ban_list.status(),
        cx.clock.now_ist(),
        cx.cfg.max_ban_list_age_hours,
    );
    let mut sorted: Vec<&UniverseEntry> = universe.iter().collect();
    sorted.sort_by(|a, b| a.instrument.tradingsymbol.cmp(&b.instrument.tradingsymbol));
    let mut out = ScanOutput::default();
    for e in sorted {
        let sym = e.instrument.tradingsymbol.clone();
        let drop = |out: &mut ScanOutput, r| out.dropped.push((sym.clone(), r));
        let banned = match &banned {
            Err(r) => {
                drop(&mut out, r.clone());
                continue;
            }
            Ok(set) => set,
        };
        let Some(end) = upto_date(&e.bars, decision_date) else {
            drop(&mut out, DropReason::NoBarOnDecisionDate);
            continue;
        };
        let bars = e.bars.get(..=end).unwrap_or(&e.bars);
        let needed = cx.cfg.min_history_bars as usize;
        if bars.len() < needed {
            drop(
                &mut out,
                DropReason::ShortHistory {
                    bars: bars.len(),
                    needed,
                },
            );
            continue;
        }
        if banned.contains(&normalise(&sym)) {
            drop(&mut out, DropReason::Banned);
            continue;
        }
        if e.circuit_locked {
            drop(&mut out, DropReason::CircuitLocked);
            continue;
        }
        let median = median_traded_value(bars, MEDIAN_WINDOW_BARS).unwrap_or(Decimal::ZERO);
        if median < cx.cfg.min_median_traded_value {
            drop(
                &mut out,
                DropReason::Illiquid {
                    median_traded_value: median,
                },
            );
            continue;
        }
        let stock = match classify(bars, cx.regime, Series::Stock, Extras::default()) {
            Ok(s) => s,
            Err(err) => {
                drop(&mut out, DropReason::RegimeFailed(err.to_string()));
                continue;
            }
        };
        let Some(view) = DataView::ending_at(&e.bars, end) else {
            continue;
        };
        let ctx = StrategyContext {
            instrument: &e.instrument,
            market_regime: &market,
            stock_regime: &stock,
            portfolio: cx.portfolio,
            calendar: cx.calendar,
        };
        let hits = merge_by_family(strategies, &view, &ctx, &market, &stock);
        if !hits.is_empty() {
            out.candidates.push(Candidate {
                instrument: e.instrument.clone(),
                hits,
                market_regime: market.clone(),
                stock_regime: stock,
                median_traded_value: median,
            });
        }
    }
    Ok(out)
}

/// Run every strategy once; keep the best signal per family (highest reference reward:risk, then
/// lowest strategy id) and list the others as merged. A cell the regime suppresses is dropped.
fn merge_by_family(
    strategies: &mut [Box<dyn Strategy>],
    view: &DataView<'_>,
    ctx: &StrategyContext<'_>,
    market: &RegimeState,
    stock: &RegimeState,
) -> Vec<Hit> {
    let mut fam: BTreeMap<String, Vec<Hit>> = BTreeMap::new();
    for s in strategies.iter_mut() {
        let (fit_cells, penalty_weight) = {
            let m = s.meta();
            (m.regime_fit.allows(market, stock), m.penalty_weight)
        };
        if matches!(fit_cells, Fit::Suppressed(_)) {
            continue;
        }
        if let Some(signal) = s.on_bar(view, ctx) {
            fam.entry(family_of(signal.strategy_id).to_owned())
                .or_default()
                .push(Hit {
                    family: family_of(signal.strategy_id).to_owned(),
                    signal,
                    fit: fit_cells,
                    penalty_weight,
                    merged_ids: Vec::new(),
                });
        }
    }
    fam.into_values()
        .filter_map(|mut v| {
            v.sort_by(|a, b| {
                b.signal
                    .reference_rr
                    .cmp(&a.signal.reference_rr)
                    .then(a.signal.strategy_id.cmp(b.signal.strategy_id))
            });
            let mut it = v.into_iter();
            let mut best = it.next()?;
            best.merged_ids = it.map(|h| h.signal.strategy_id).collect();
            Some(best)
        })
        .collect()
}

#[cfg(test)]
mod tests;
