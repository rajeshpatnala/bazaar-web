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
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_regime::{Extras, Fit, RegimeParams, RegimeState, Series, classify};
use bazaar_strategies::{ParamError, PortfolioView, Signal, Strategy, StrategyContext};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::ports::BanList;
use crate::universe::{DropReason, MEDIAN_WINDOW_BARS, banned_set, median_traded_value, normalise};

/// Circuit-limit state of a name on the decision day. `Unknown` drops the name (fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Locked,
    Free,
    /// No data source: the caller passes this until one exists, so a scan lists nothing as tradable.
    Unknown,
}

/// One name of the universe with its daily bars (oldest first). Prices and volumes must be
/// adjusted for corporate actions together: traded value is `close * volume`.
#[derive(Debug, Clone)]
pub struct UniverseEntry {
    pub instrument: Instrument,
    pub bars: Vec<Candle>,
    /// From the day's data, supplied by the caller.
    pub circuit: CircuitState,
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
    #[error("the market's decision bar closes after the clock: it is not a closed bar")]
    DecisionBarOpen,
    #[error("market regime: {0}")]
    MarketRegime(String),
    #[error("decision date {decision} is not the latest completed session ({latest})")]
    StaleDecisionDate {
        decision: NaiveDate,
        latest: NaiveDate,
    },
    #[error("no strategy is enabled, so no family count exists")]
    NoStrategiesEnabled,
    #[error("calendar: {0}")]
    Calendar(String),
}

/// Everything the runner reads besides the universe.
pub struct ScanContext<'a> {
    pub cfg: &'a ScannerConfig,
    pub regime: &'a RegimeParams,
    pub calendar: &'a MarketCalendar,
    pub portfolio: &'a PortfolioView,
    pub ban_list: &'a dyn BanList,
    /// Effective ban-list age limit, see [`effective_ban_age_hours`]; the runner uses only this.
    pub ban_list_max_age_hours: u32,
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
    /// Distinct families among the ENABLED strategies (>= 1), never counted from hits.
    pub families_total: u32,
}

/// The strictest of the three ban-list age limits: `[scanner]`, risk.toml `events` and
/// risk.toml `hard_limits`. A looser scanner value can never widen the gate's limit.
#[must_use]
pub fn effective_ban_age_hours(scanner: u32, risk_events: u32, risk_hard_limit: u32) -> u32 {
    scanner.min(risk_events).min(risk_hard_limit)
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

/// Distinct families of the enabled strategies (the list `build` made from the config).
fn count_families(strategies: &[Box<dyn Strategy>]) -> Result<u32, ScanError> {
    let fams: std::collections::BTreeSet<&str> =
        strategies.iter().map(|s| family_of(s.meta().id)).collect();
    match u32::try_from(fams.len()) {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(ScanError::NoStrategiesEnabled),
    }
}

/// The latest equity session that has closed at `now`: today when it is a trading day past the
/// close, else the previous trading day.
fn latest_completed_session(
    cal: &MarketCalendar,
    now: chrono::DateTime<chrono_tz::Tz>,
) -> Result<NaiveDate, ScanError> {
    let cal_err = |e: bazaar_core::calendar::CalendarError| ScanError::Calendar(e.to_string());
    let today = now.date_naive();
    if cal
        .is_trading_day(today, Segment::Equity)
        .map_err(cal_err)?
    {
        let close = cal
            .close_time(today, Segment::Equity, InstrumentKind::CashNonFno)
            .map_err(cal_err)?;
        if now.with_timezone(&chrono::Utc) >= close {
            return Ok(today);
        }
    }
    cal.prev_trading_day(today, Segment::Equity)
        .map_err(cal_err)
}

fn upto_date(bars: &[Candle], date: NaiveDate) -> Option<usize> {
    bars.iter()
        .rposition(|b| b.close_time().date_naive() == date)
}

/// Market bars up to and including the decision bar: the decision date is the latest completed session or
/// today's still-open one (reported as `DecisionBarOpen`), and its bar has closed by `now`.
fn market_bars_to<'a>(
    cx: &ScanContext<'_>,
    market_bars: &'a [Candle],
    decision_date: NaiveDate,
    now: chrono::DateTime<chrono_tz::Tz>,
) -> Result<&'a [Candle], ScanError> {
    let latest = latest_completed_session(cx.calendar, now)?;
    // A date after `latest` is today's open bar (or the future): `DecisionBarOpen` below says so.
    if decision_date < latest {
        return Err(ScanError::StaleDecisionDate {
            decision: decision_date,
            latest,
        });
    }
    let m_end =
        upto_date(market_bars, decision_date).ok_or(ScanError::NoMarketBar(decision_date))?;
    if market_bars.get(m_end).is_none_or(|b| b.close_time() > now) {
        return Err(ScanError::DecisionBarOpen);
    }
    Ok(market_bars.get(..=m_end).unwrap_or(market_bars))
}

/// Run `strategies` over `universe` at the closed bar of `decision_date`.
///
/// Strategies are reused across names, so `on_bar` must keep no per-instrument state. Only Buy
/// signals exist today: `Signal` is long-only by type (no side, stop below entry; a short-shaped one
/// fails `Signal::validate`), so no conflicting Buy/Sell pair can arise. The first slice that adds a
/// sell side must add `Warning::ConflictingSignals` reporting here (CR-114-3). The market regime uses no
/// India VIX or breadth (`Extras::default()`): vol comes from realised vol only.
///
/// # Errors
/// `decision_date` older than the latest completed session at the clock, no market bar on that
/// date, the market bar closes after the clock (a bar still open), no enabled strategy, or the
/// market regime cannot be built.
pub fn run(
    cx: &ScanContext<'_>,
    strategies: &mut [Box<dyn Strategy>],
    universe: &[UniverseEntry],
    market_bars: &[Candle],
    decision_date: NaiveDate,
) -> Result<ScanOutput, ScanError> {
    let now = cx.clock.now_ist();
    let families_total = count_families(strategies)?;
    let m_bars = market_bars_to(cx, market_bars, decision_date, now)?;
    let market = classify(m_bars, cx.regime, Series::Market, Extras::default())
        .map_err(|e| ScanError::MarketRegime(e.to_string()))?;
    let banned = banned_set(&cx.ban_list.status(), now, cx.ban_list_max_age_hours);
    let mut sorted: Vec<&UniverseEntry> = universe.iter().collect();
    sorted.sort_by(|a, b| a.instrument.tradingsymbol.cmp(&b.instrument.tradingsymbol));
    let mut out = ScanOutput {
        families_total,
        ..ScanOutput::default()
    };
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
        if e.bars.get(end).is_none_or(|b| b.close_time() > now) {
            drop(&mut out, DropReason::DecisionBarNotClosed);
            continue;
        }
        let bars = e.bars.get(..=end).unwrap_or(&e.bars);
        let needed = usize::try_from(cx.cfg.min_history_bars)
            .unwrap_or(usize::MAX)
            .max(MEDIAN_WINDOW_BARS);
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
        match e.circuit {
            CircuitState::Free => {}
            CircuitState::Locked => {
                drop(&mut out, DropReason::CircuitLocked);
                continue;
            }
            CircuitState::Unknown => {
                drop(&mut out, DropReason::CircuitStatusUnknown);
                continue;
            }
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
            drop(&mut out, DropReason::ViewUnavailable);
            continue;
        };
        let ctx = StrategyContext {
            instrument: &e.instrument,
            market_regime: &market,
            stock_regime: &stock,
            portfolio: cx.portfolio,
            calendar: cx.calendar,
        };
        let (hits, expired) = merge_by_family(strategies, &view, &ctx, (now, &sym));
        out.dropped.extend(expired);
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

/// Run every strategy once; keep the best signal per family (an `Allowed` fit before a `Penalised`
/// one, then highest reference reward:risk, then lowest strategy id) and list the others as merged. A cell the regime suppresses is dropped.
fn merge_by_family(
    strategies: &mut [Box<dyn Strategy>],
    view: &DataView<'_>,
    ctx: &StrategyContext<'_>,
    (now, sym): (chrono::DateTime<chrono_tz::Tz>, &str),
) -> (Vec<Hit>, Vec<(String, DropReason)>) {
    let (market, stock) = (ctx.market_regime, ctx.stock_regime);
    let mut expired = Vec::new();
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
            if signal.valid_until <= now {
                expired.push((
                    sym.to_owned(),
                    DropReason::SignalExpired {
                        strategy_id: signal.strategy_id.to_owned(),
                    },
                ));
                continue;
            }
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
    let hits = fam
        .into_values()
        .filter_map(|mut v| {
            v.sort_by(|a, b| {
                matches!(a.fit, Fit::Penalised(_))
                    .cmp(&matches!(b.fit, Fit::Penalised(_)))
                    .then(b.signal.reference_rr.cmp(&a.signal.reference_rr))
                    .then(a.signal.strategy_id.cmp(b.signal.strategy_id))
            });
            let mut it = v.into_iter();
            let mut best = it.next()?;
            best.merged_ids = it.map(|h| h.signal.strategy_id).collect();
            Some(best)
        })
        .collect();
    (hits, expired)
}

#[cfg(test)]
mod tests;
