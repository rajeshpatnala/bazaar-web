//! `trend_pullback`: buy a short dip in an established uptrend (spec
//! `docs/strategy/F8-first-strategies.md` section 4). Long-only, daily bars, NSE cash equity.
//!
//! * **Idea:** in an uptrend a short dip toward the short average is often bought by trend
//!   followers; entering when price turns back up gives a close stop. The other side of the trade
//!   is early profit-takers.
//! * **Entry (closed bar t):** some close in the `pullback_lookback` bars before t was below its
//!   own short average; the close at t is above the short average, above the longer average and
//!   above the high of bar t-1 (the turn). Entry zone: from that close up to
//!   `entry_zone_atr_frac` x ATR above it.
//! * **Stop:** lowest low over the lookback window and t, minus `stop_buffer_atr` x ATR; the signal
//!   is dropped when `entry_high - stop` exceeds `max_stop_atr` x ATR. The stop is fixed at entry: v1 has no trailing, so `manage` never tightens it.
//! * **Exit:** reference target `entry_low + target_r x risk`; leave at the next open when the close
//!   is under the longer average; time stop after `max_hold_bars`.
//! * **Best regime:** stock Up with Low or Normal volatility, market Up or Range (declared in
//!   `config/strategies.toml`, never in code). Suppressed in Up x High, Range, Down, Mixed,
//!   Unknown and a market Down or Breakdown.
//! * **Weaknesses:** a dip can be the start of a trend change (the stop handles it, not hope); a
//!   gap down through the stop fills at the open, worse than planned; overlaps with
//!   `donchian_breakout` in strong trends (same trade, counted once); a touch of the short average
//!   is arbitrary and shallow dips never trigger; the time stop can cut slow winners.
//!
//! Performance note (for F9): every call recomputes EMA and ATR over the whole view, O(n) per bar; a
//! streaming state inside the strategy is the follow-up if backtests get slow. Callers should supply
//! well over `min_history_bars` so the long average is more than its seed.
//!
//! Every default is UNVERIFIED (convention or proposal) and is never tuned on strategy P&L. A signal
//! is not advice. Only closed bars are used: the view ends at the decision bar and the stop, entry
//! and the lookback window use bars up to and including it.

use bazaar_core::candle::Candle;
use bazaar_core::dataview::DataView;
use bazaar_core::segment::InstrumentKind;
use bazaar_indicators::ma::Ema;
use bazaar_indicators::trend::Atr;
use bazaar_indicators::{Ohlcv, batch};
use bazaar_regime::{Fact, Fit, label};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::common::{
    Levels, Reject, RiskBasis, WHY_TEXT_LABEL, Why, check_rr, check_stop, entry_guards, entry_zone,
    need, round_levels, to_decimal, valid_until, why_text,
};
use crate::context::StrategyContext;
use crate::exit::{ExitAction, ExitError, OpenPosition};
use crate::factory::{BuildInput, StrategyFactory, meta_from};
use crate::params::{ParamError, ParamSet, parse_params};
use crate::signal::{ExitPlan, Signal};
use crate::strategy::{Strategy, StrategyMeta};

/// Strategy id, the `[strategy.<id>]` key in `config/strategies.toml`.
pub const ID: &str = "trend_pullback";
/// Bumped when the rules change (provenance).
pub const VERSION: u32 = 1;

/// `[strategy.trend_pullback.params]`. Defaults are the spec's (UNVERIFIED).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Params {
    /// Entry zone width above the signal close, in ATR.
    pub entry_zone_atr_frac: f64,
    pub atr_period: usize,
    /// Short trend average (the dip reference).
    pub ema_pullback: usize,
    /// Longer trend average (the trend filter and the exit reference).
    pub ema_trend: usize,
    pub pullback_lookback: usize,
    pub stop_buffer_atr: f64,
    pub target_r: f64,
    pub max_stop_atr: f64,
    pub max_hold_bars: u32,
    pub min_rr: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            entry_zone_atr_frac: 0.25,
            atr_period: 14,
            ema_pullback: 20,
            ema_trend: 50,
            pullback_lookback: 5,
            stop_buffer_atr: 0.5,
            target_r: 2.0,
            max_stop_atr: 3.0,
            max_hold_bars: 15,
            min_rr: 1.5,
        }
    }
}

fn bad(id: &str, key: &'static str, reason: &'static str) -> ParamError {
    ParamError::Invalid {
        id: id.to_owned(),
        key,
        reason,
    }
}

impl ParamSet for Params {
    fn validate(&self, id: &str) -> Result<(), ParamError> {
        let f = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        if !f(self.entry_zone_atr_frac, 0.1, 0.5) {
            return Err(bad(id, "entry_zone_atr_frac", "must be 0.1..=0.5"));
        }
        if !(5..=30).contains(&self.atr_period) {
            return Err(bad(id, "atr_period", "must be 5..=30"));
        }
        if !(10..=30).contains(&self.ema_pullback) {
            return Err(bad(id, "ema_pullback", "must be 10..=30"));
        }
        if !(30..=100).contains(&self.ema_trend) || self.ema_trend <= self.ema_pullback {
            return Err(bad(
                id,
                "ema_trend",
                "must be 30..=100 and above ema_pullback",
            ));
        }
        if !(3..=8).contains(&self.pullback_lookback) {
            return Err(bad(id, "pullback_lookback", "must be 3..=8"));
        }
        if !f(self.stop_buffer_atr, 0.25, 1.0) {
            return Err(bad(id, "stop_buffer_atr", "must be 0.25..=1.0"));
        }
        if !f(self.target_r, 1.5, 3.0) {
            return Err(bad(id, "target_r", "must be 1.5..=3.0"));
        }
        if !f(self.max_stop_atr, 2.0, 3.5) {
            return Err(bad(id, "max_stop_atr", "must be 2.0..=3.5"));
        }
        if !(10..=25).contains(&self.max_hold_bars) {
            return Err(bad(id, "max_hold_bars", "must be 10..=25"));
        }
        if !f(self.min_rr, 1.5, 2.0) {
            return Err(bad(id, "min_rr", "must be 1.5..=2.0"));
        }
        // Tick rounding only ever enlarges the risk, so a target at or under the floor never fires.
        if self.target_r <= self.min_rr {
            return Err(bad(id, "target_r", "must be above min_rr"));
        }
        // Indicator periods are bounded above, so these cannot fail once the ranges hold; still
        // checked so a future range change cannot leave a strategy that never fires.
        Ema::new(self.ema_pullback).map_err(|_| bad(id, "ema_pullback", "bad period"))?;
        Ema::new(self.ema_trend).map_err(|_| bad(id, "ema_trend", "bad period"))?;
        Atr::new(self.atr_period).map_err(|_| bad(id, "atr_period", "bad period"))?;
        Ok(())
    }
}

impl Params {
    /// Bars needed before every input exists: the long average at t (N bars), ATR (N+1 bars) and
    /// the short average at the oldest lookback bar. Bar t-1 lies inside all of those windows.
    #[must_use]
    pub fn min_history_bars(&self) -> usize {
        self.ema_trend
            .max(self.atr_period + 1)
            .max(self.ema_pullback + self.pullback_lookback)
    }
}

/// Why this bar is not a setup (distinct from a guard failure).
#[derive(Debug)]
enum Skip {
    Guard(Reject),
    NoSetup(&'static str),
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Guard(r) => write!(f, "{r}"),
            Self::NoSetup(s) => f.write_str(s),
        }
    }
}

impl From<Reject> for Skip {
    fn from(r: Reject) -> Self {
        Self::Guard(r)
    }
}

/// The strategy.
#[derive(Debug)]
pub struct TrendPullback {
    meta: StrategyMeta,
    params: Params,
    params_hash: String,
}

fn ema_series(bars: &[Candle], period: usize) -> Result<Vec<Option<f64>>, Reject> {
    let ema = Ema::new(period).map_err(|_| Reject::Indicator("ema"))?;
    let closes = bars
        .iter()
        .map(|c| Ohlcv::try_from(c).map(|o| o.c))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Reject::NotFinite)?;
    Ok(batch(ema, closes))
}

/// The newest Wilder ATR, or the reason it does not exist yet.
fn last_atr(ohlcv: &[Ohlcv], period: usize) -> Result<f64, Reject> {
    let atr = Atr::new(period).map_err(|_| Reject::Indicator("atr"))?;
    need(
        "atr",
        batch(atr, ohlcv.iter().copied()).last().copied().flatten(),
    )
}

impl TrendPullback {
    /// Build from checked params and the provenance hash of the config table.
    #[must_use]
    pub fn new(meta: StrategyMeta, params: Params, params_hash: String) -> Self {
        Self {
            meta,
            params,
            params_hash,
        }
    }

    fn evaluate(&self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Result<Signal, Skip> {
        let p = &self.params;
        let fit = entry_guards(&self.meta, view, ctx)?;
        let bars = view.bars();
        let t = bars.len().saturating_sub(1);
        let bar = |i: usize| bars.get(i).ok_or(Reject::Indicator("bar"));
        let before = t.checked_sub(1).ok_or(Reject::Indicator("bar"))?; // guards ensure t >= 1
        let (now, prev) = (bar(t)?, bar(before)?);
        let ohlcv = bars
            .iter()
            .map(Ohlcv::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Reject::NotFinite)?;
        let atr = last_atr(&ohlcv, p.atr_period)?;
        let short = ema_series(bars, p.ema_pullback)?;
        let long = ema_series(bars, p.ema_trend)?;
        let at = |s: &[Option<f64>], i: usize| s.get(i).copied().flatten();
        let ema_short_t = need("ema_pullback", at(&short, t))?;
        let ema_long_t = need("ema_trend", at(&long, t))?;
        let close = now.close();
        let close_f = need("close", Ohlcv::try_from(now).ok().map(|o| o.c))?;
        // The turn: back above the short and long averages and above yesterday's high.
        if close_f <= ema_short_t {
            return Err(Skip::NoSetup("close is not above the short average"));
        }
        if close_f <= ema_long_t {
            return Err(Skip::NoSetup("close is not above the long average"));
        }
        if close <= prev.high() {
            return Err(Skip::NoSetup("close is not above the previous high"));
        }
        // The dip: some earlier close in the window was under its own short average.
        let first = t.saturating_sub(p.pullback_lookback);
        let days_in_pullback = days_under(&ohlcv, &short, first, t);
        if days_in_pullback == 0 {
            return Err(Skip::NoSetup(
                "no close under the short average in the window",
            ));
        }
        let window = bars.get(first..=t).ok_or(Reject::Indicator("window"))?;
        let pullback_low = window
            .iter()
            .map(Candle::low)
            .min()
            .ok_or(Reject::Indicator("window"))?;
        let l = levels(ctx, p, close, atr, pullback_low)?;
        let risk = check_stop(&l, atr, p.max_stop_atr)?;
        let rr = check_rr(&l, p.min_rr)?;

        let stock_label = label(ctx.stock_regime);
        let market_label = label(ctx.market_regime);
        let why = why_text(&Why {
            risk_rupees: risk,
            risk_basis: RiskBasis::PerShare,
            setup: &setup_text(
                &ctx.instrument.tradingsymbol,
                days_in_pullback,
                p.ema_pullback,
            ),
            regime_label: &market_label,
            proven_wrong_at: l.stop,
            closing: &closing_text(l.target, rr, p.ema_trend),
        })?;
        let rule_text = exit_rule_text(p);
        let facts = facts(&Detail {
            why,
            params: p,
            ema_short: ema_short_t,
            pullback_low,
            days: days_in_pullback,
            atr,
            rule_text: &rule_text,
            stock_label,
            market_label,
            fit,
        });

        let sig = Signal {
            strategy_id: ID,
            instrument_token: ctx.instrument.instrument_token,
            strategy_version: VERSION,
            params_hash: self.params_hash.clone(),
            signal_bar_time: now.close_time(),
            entry_low: l.entry_low,
            entry_high: l.entry_high,
            stop: l.stop,
            targets: vec![l.target],
            risk_per_share: risk,
            reference_rr: rr,
            exit_plan: ExitPlan {
                time_stop_bars: p.max_hold_bars,
                rule_text,
            },
            facts,
            valid_until: valid_until(view, ctx, InstrumentKind::CashNonFno)?,
        };
        sig.validate()
            .map_err(|_| Skip::NoSetup("signal broke the contract"))?;
        Ok(sig)
    }
}

/// Raw entry zone, stop and reference target, rounded to the tick (stop down, zone top up).
fn levels(
    ctx: &StrategyContext<'_>,
    p: &Params,
    close: Decimal,
    atr: f64,
    pullback_low: Decimal,
) -> Result<Levels, Reject> {
    let stop_raw = pullback_low
        .checked_sub(to_decimal(p.stop_buffer_atr * atr)?)
        .ok_or(Reject::NotFinite)?;
    let (entry_low, entry_high) = entry_zone(close, atr, p.entry_zone_atr_frac)?;
    let risk_raw = entry_high.checked_sub(stop_raw).ok_or(Reject::NotFinite)?;
    let target_raw = entry_low
        .checked_add(
            to_decimal(p.target_r)?
                .checked_mul(risk_raw)
                .ok_or(Reject::NotFinite)?,
        )
        .ok_or(Reject::NotFinite)?;
    round_levels(
        ctx,
        &Levels {
            entry_low,
            entry_high,
            stop: stop_raw,
            target: target_raw,
        },
    )
}

fn exit_rule_text(p: &Params) -> String {
    format!(
        "Leave at the next open if a close is under the {}-day average, or after {} bars.",
        p.ema_trend, p.max_hold_bars
    )
}

/// Closes in `first..t` that were under their own short average.
fn days_under(ohlcv: &[Ohlcv], short: &[Option<f64>], first: usize, t: usize) -> usize {
    (first..t)
        .filter(|&k| match (ohlcv.get(k), short.get(k).copied().flatten()) {
            (Some(o), Some(e)) => o.c < e,
            _ => false,
        })
        .count()
}

/// Inputs of the Why-panel facts.
struct Detail<'a> {
    why: String,
    params: &'a Params,
    ema_short: f64,
    pullback_low: Decimal,
    days: usize,
    atr: f64,
    rule_text: &'a str,
    stock_label: String,
    market_label: String,
    fit: Fit,
}

fn facts(d: &Detail<'_>) -> Vec<Fact> {
    let p = d.params;
    let mut out = vec![
        Fact::new(
            WHY_TEXT_LABEL,
            d.why.clone(),
            "Plain-English reason, risk first.",
        ),
        Fact::new(
            "ema_pullback",
            format!("{:.2}", d.ema_short),
            format!("{}-day average, the dip reference", p.ema_pullback),
        ),
        Fact::new(
            "pullback_low",
            format!("{:.2}", d.pullback_low),
            "Lowest low from the dip window through today.",
        ),
        Fact::new(
            "days_in_pullback",
            d.days.to_string(),
            "Closes under the short average in the window.",
        ),
        Fact::new("atr", format!("{:.2}", d.atr), "Average daily range."),
        Fact::new(
            "stop_basis",
            format!("{} x ATR under the pullback low", p.stop_buffer_atr),
            "How the stop was set.",
        ),
        Fact::new(
            "target_r",
            p.target_r.to_string(),
            "Reference reward in multiples of the risk.",
        ),
        Fact::new("exit_rule", d.rule_text, "How the idea ends."),
        Fact::new(
            "stock_regime",
            d.stock_label.clone(),
            "Stock label at the decision bar.",
        ),
        Fact::new(
            "market_regime",
            d.market_label.clone(),
            "Market label at the decision bar.",
        ),
    ];
    if let Fit::Penalised(f) = &d.fit {
        out.push(f.clone());
    }
    out
}

/// Setup sentence of the why-text (snapshot-tested). Periods come from the params.
#[must_use]
pub fn setup_text(symbol: &str, days: usize, ema_pullback: usize) -> String {
    format!(
        "Setup: {symbol} is in an uptrend and dipped for {days} day(s), then closed back above its {ema_pullback}-day average. Why now: today's close is above yesterday's high."
    )
}

/// Closing sentence of the why-text (snapshot-tested).
#[must_use]
pub fn closing_text(target: Decimal, rr: Decimal, ema_trend: usize) -> String {
    format!(
        "Target: {target:.2} (about {rr:.2} times the risk), or we leave if it falls under its {ema_trend}-day average."
    )
}

impl Strategy for TrendPullback {
    fn meta(&self) -> &StrategyMeta {
        &self.meta
    }

    fn on_bar(&mut self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<Signal> {
        match self.evaluate(view, ctx) {
            Ok(s) => Some(s),
            Err(skip) => {
                // Lazy `%` formatting: nothing is allocated unless debug logging is on.
                tracing::debug!(strategy = ID, reason = %skip, "no signal");
                None
            }
        }
    }

    /// Exit at the next open when the close is under the longer average, or at the time stop.
    /// When the average is not available the position is held (its stop still protects it).
    fn manage(
        &mut self,
        view: &DataView<'_>,
        _ctx: &StrategyContext<'_>,
        position: &OpenPosition,
    ) -> Result<ExitAction, ExitError> {
        let p = &self.params;
        if position.bars_held >= p.max_hold_bars {
            return Ok(ExitAction::Exit(format!(
                "Time stop: held {} bars.",
                p.max_hold_bars
            )));
        }
        // Unusable data is a fault the engine must see, never a silent hold (as donchian does).
        // The caller logs the returned error; nothing is logged here.
        let fault = |why: String| ExitError::Context(why);
        if view.bars().iter().any(|c| !c.adjusted()) {
            return Err(fault("candles are not adjusted".into()));
        }
        let series = ema_series(view.bars(), p.ema_trend).map_err(|r| fault(r.to_string()))?;
        let last = view
            .last()
            .ok_or_else(|| fault("the view has no bars".into()))?;
        let close = Ohlcv::try_from(last)
            .map_err(|_| fault("the last candle is not finite".into()))?
            .c;
        let Some(Some(avg)) = series.last() else {
            tracing::debug!(
                strategy = ID,
                "trend exit not evaluated: average still warming up, holding"
            );
            return Ok(ExitAction::Hold);
        };
        if close < *avg {
            return Ok(ExitAction::Exit(format!(
                "Closed under its {}-day average: the uptrend is in doubt.",
                p.ema_trend
            )));
        }
        Ok(ExitAction::Hold)
    }
}

impl TrendPullback {
    /// The reason a bar gave no signal, for tests and diagnostics; `None` when it fires.
    #[must_use]
    pub fn why_none(&self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<String> {
        match self.evaluate(view, ctx) {
            Ok(_) => None,
            Err(skip) => Some(skip.to_string()),
        }
    }
}

/// Factory the registry lists.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrendPullbackFactory;

impl StrategyFactory for TrendPullbackFactory {
    fn id(&self) -> &'static str {
        ID
    }

    fn build(&self, input: &BuildInput<'_>) -> Result<Box<dyn Strategy>, ParamError> {
        let params: Params = parse_params(ID, input.params)?;
        let meta = meta_from(
            ID,
            "Trend pullback",
            VERSION,
            params.min_history_bars(),
            input.config,
        )?;
        Ok(Box::new(TrendPullback::new(
            meta,
            params,
            input.params_hash.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::on_tick;
    use crate::context::PortfolioView;
    use crate::params::StrategyConfig;
    use crate::testkit::{DRow, World, bars_dec, meta, session_time};
    use bazaar_core::config::StrategiesConfig;
    use bazaar_regime::{Trend as T, VolState as V};

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    /// Rows of `fixtures/strategies/trend_pullback_<name>.csv`: `open,high,low,close,volume`.
    fn load(name: &str) -> Vec<DRow> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../fixtures/strategies/trend_pullback_{name}.csv"
        ));
        let text = std::fs::read_to_string(path).unwrap();
        text.lines()
            .skip(1)
            .map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                ([d(f[0]), d(f[1]), d(f[2]), d(f[3])], f[4].parse().unwrap())
            })
            .collect()
    }

    fn strat() -> TrendPullback {
        let p = Params::default();
        TrendPullback::new(meta(p.min_history_bars()), p, "hash".into())
    }

    fn world(i: usize) -> World {
        World::new(T::Up, V::Normal, T::Up, V::Normal).at(i)
    }

    fn at(s: &mut TrendPullback, bars: &[Candle], i: usize, w: &World) -> Option<Signal> {
        s.on_bar(&DataView::ending_at(bars, i).unwrap(), &w.ctx())
    }

    fn why(s: &TrendPullback, bars: &[Candle], i: usize, w: &World) -> String {
        s.why_none(&DataView::ending_at(bars, i).unwrap(), &w.ctx())
            .unwrap()
    }

    /// Bar index of the hand-computed entry in `entry.csv` (the 60th bar).
    const K: usize = 59;

    #[test]
    fn fires_exactly_on_bar_k_with_hand_computed_levels() {
        let b = bars_dec(&load("entry"), true);
        let mut s = strat();
        let fired: Vec<usize> = (0..b.len())
            .filter(|&i| at(&mut s, &b, i, &world(i)).is_some())
            .collect();
        assert_eq!(fired, vec![K]);
        let sig = at(&mut s, &b, K, &world(K)).unwrap();
        // Bar K: close 114.00, ATR(14) 2.6469, pullback low 109.00, 2 dip closes.
        // stop 109 - 0.5 x 2.6469 = 107.677 -> down to 107.65; zone top 114.6617 -> up to 114.70;
        // target 114 + 2 x (114.6617 - 107.677) = 127.970 -> 127.95; risk 7.05; rr 13.95 / 7.05.
        assert_eq!(sig.entry_low, d("114.00"));
        assert_eq!(sig.entry_high, d("114.70"));
        assert_eq!(sig.stop, d("107.65"));
        assert_eq!(sig.targets, vec![d("127.95")]);
        assert_eq!(sig.risk_per_share, d("7.05"));
        assert_eq!(sig.reference_rr, d("1.97"));
        assert_eq!(sig.exit_plan.time_stop_bars, 15);
        assert_eq!(sig.signal_bar_time, session_time(K));
        assert!(sig.valid_until > sig.signal_bar_time);
        assert_eq!((sig.strategy_id, sig.strategy_version), (ID, VERSION));
        assert_eq!(sig.params_hash, "hash");
        assert_eq!(sig.validate(), Ok(()));
        let w = world(K);
        for p in [sig.entry_low, sig.entry_high, sig.stop, sig.targets[0]] {
            assert!(on_tick(&w.ctx(), p).unwrap(), "{p} is on the tick");
        }
        let fact = |l: &str| {
            sig.facts
                .iter()
                .find(|f| f.label == l)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(fact("days_in_pullback"), "2");
        assert_eq!(fact("pullback_low"), "109.00");
        assert_eq!(fact("atr"), "2.65");
        assert!(
            fact(WHY_TEXT_LABEL).starts_with("Risk first: you could lose about Rs 7.05 per share.")
        );
    }

    #[test]
    fn no_signal_without_a_dip_and_when_the_stop_is_too_wide() {
        let mut s = strat();
        let nodip = bars_dec(&load("nodip"), true);
        for i in 0..nodip.len() {
            assert!(at(&mut s, &nodip, i, &world(i)).is_none(), "bar {i}");
        }
        assert!(why(&s, &nodip, 62, &world(62)).contains("previous high"));
        // A gap-up close above everything still has no dip behind it.
        let mut rows = load("nodip");
        let last = rows.len() - 1;
        rows[last] = ([d("116"), d("121"), d("115"), d("120")], 100_000);
        let gap = bars_dec(&rows, true);
        assert!(why(&s, &gap, last, &world(last)).contains("no close under"));
        let deep = bars_dec(&load("deep"), true);
        assert!(at(&mut s, &deep, K, &world(K)).is_none());
        assert!(why(&s, &deep, K, &world(K)).contains("stop is"));
    }

    #[test]
    fn guards_return_none_with_a_reason() {
        let b = bars_dec(&load("entry"), true);
        let s = strat();
        let w = world(K);
        assert!(
            why(&s, &b, 40, &world(40)).contains("bars"),
            "short history"
        );
        let un = bars_dec(&load("entry"), false);
        assert!(why(&s, &un, K, &w).contains("adjusted"));
        let mut held = world(K);
        held.portfolio = PortfolioView {
            positions: vec![OpenPosition {
                instrument_token: held.instrument.instrument_token,
                entry_price: d("100"),
                stop: d("90"),
                opened_at: session_time(50),
                bars_held: 9,
            }],
        };
        assert!(why(&s, &b, K, &held).contains("held"));
        for (m, st) in [
            ((T::Down, V::Normal), (T::Up, V::Normal)),
            ((T::Up, V::Normal), (T::Range, V::Normal)),
            ((T::Up, V::Normal), (T::Mixed, V::Normal)),
            ((T::Up, V::Normal), (T::Up, V::High)),
            ((T::Up, V::Normal), (T::Unknown, V::Unknown)),
        ] {
            let w = World::new(m.0, m.1, st.0, st.1).at(K);
            let mut s = strat();
            let got = at(&mut s, &b, K, &w);
            // Up x High is declared weaker by the test meta (penalised, still allowed).
            if st == (T::Up, V::High) {
                assert!(got.is_some());
            } else {
                assert!(got.is_none(), "{m:?} {st:?}");
            }
        }
    }

    #[test]
    fn lookahead_later_bars_change_nothing_and_a_future_regime_is_refused() {
        let full = bars_dec(&load("entry"), true);
        let mut s = strat();
        let pos = OpenPosition {
            instrument_token: 7,
            entry_price: d("114.00"),
            stop: d("107.65"),
            opened_at: session_time(K),
            bars_held: 3,
        };
        for end in 0..full.len() {
            let cut = &full[..=end];
            let w = world(end);
            let on_cut = at(&mut s, cut, end, &w);
            let on_full = at(&mut s, &full, end, &w);
            assert_eq!(on_cut, on_full, "bar {end}");
            let run = |s: &mut TrendPullback, bars: &[Candle]| {
                s.manage(&DataView::ending_at(bars, end).unwrap(), &w.ctx(), &pos)
            };
            assert_eq!(run(&mut s, cut), run(&mut s, &full), "manage bar {end}");
        }
        // Regime as of a later session than the decision bar: refused, not trusted.
        let future = world(K + 2);
        assert!(why(&s, &full, K, &future).contains("look-ahead"));
        assert!(at(&mut s, &full, K, &future).is_none());
    }

    #[test]
    fn manage_exits_under_the_long_average_and_at_the_time_stop() {
        let b = bars_dec(&load("entry"), true);
        let mut s = strat();
        let w = world(66);
        let pos = |held| OpenPosition {
            instrument_token: 7,
            entry_price: d("114.00"),
            stop: d("107.65"),
            opened_at: session_time(K),
            bars_held: held,
        };
        let run = |s: &mut TrendPullback, i: usize, held| {
            s.manage(&DataView::ending_at(&b, i).unwrap(), &w.ctx(), &pos(held))
                .unwrap()
        };
        // Close 112 is above the 50-day average 109.67; close 108 is under 109.61.
        assert_eq!(run(&mut s, 65, 6), ExitAction::Hold);
        assert!(matches!(run(&mut s, 66, 7), ExitAction::Exit(m) if m.contains("50-day")));
        assert!(matches!(run(&mut s, 62, 15), ExitAction::Exit(m) if m.contains("Time stop")));
        assert_eq!(run(&mut s, 62, 14), ExitAction::Hold);
        // Too little history for the average: hold, the stop still protects.
        assert_eq!(run(&mut s, 10, 1), ExitAction::Hold);
        // Unadjusted candles could fake a crash: a data fault, not a hold; the time stop still wins.
        let raw = bars_dec(&load("entry"), false);
        let view = DataView::ending_at(&raw, 66).unwrap();
        assert!(matches!(
            s.manage(&view, &w.ctx(), &pos(7)),
            Err(ExitError::Context(_))
        ));
        assert!(matches!(
            s.manage(&view, &w.ctx(), &pos(15)),
            Ok(ExitAction::Exit(_))
        ));
    }

    #[test]
    fn why_text_template_names_risk_first_regime_and_proven_wrong() {
        let setup = setup_text("ABC", 2, 20);
        let closing = closing_text(d("127.95"), d("1.97"), 50);
        let text = why_text(&Why {
            risk_rupees: d("7.05"),
            risk_basis: RiskBasis::PerShare,
            setup: &setup,
            regime_label: "market is rising",
            proven_wrong_at: d("107.65"),
            closing: &closing,
        })
        .unwrap();
        assert_eq!(
            text,
            "Risk first: you could lose about Rs 7.05 per share. Setup: ABC is in an uptrend and dipped for 2 day(s), then closed back above its 20-day average. Why now: today's close is above yesterday's high. Market today: market is rising. Proven wrong if: price falls to or below 107.65. Target: 127.95 (about 1.97 times the risk), or we leave if it falls under its 50-day average."
        );
    }

    fn params_table(k: &str, v: &str) -> toml::Table {
        let base = "entry_zone_atr_frac=0.25\natr_period=14\nema_pullback=20\nema_trend=50\npullback_lookback=5\nstop_buffer_atr=0.5\ntarget_r=2.0\nmax_stop_atr=3.0\nmax_hold_bars=15\nmin_rr=1.5\n";
        let key = format!("{k}=");
        let mut lines: Vec<String> = base
            .lines()
            .filter(|l| !l.starts_with(&key))
            .map(str::to_owned)
            .collect();
        lines.push(format!("{key}{v}"));
        lines.join("\n").parse().unwrap()
    }

    #[test]
    fn params_defaults_are_valid_and_bad_tables_are_load_errors() {
        assert_eq!(Params::default().validate(ID), Ok(()));
        assert!(parse_params::<Params>(ID, &toml::Table::new()).is_err());
        assert!(parse_params::<Params>(ID, &params_table("max_hold_bars", "15")).is_ok());
        for (k, v) in [
            ("pullback_lookback", "2"),
            ("pullback_lookback", "9"),
            ("target_r", "1.0"),
            ("target_r", "1.5"),
            ("min_rr", "2.5"),
            ("max_stop_atr", "4.0"),
            ("ema_trend", "20"),
            ("entry_zone_atr_frac", "0.9"),
            ("max_hold_bars", "30"),
            ("stop_buffer_atr", "nan"),
        ] {
            assert!(
                parse_params::<Params>(ID, &params_table(k, v)).is_err(),
                "{k}={v}"
            );
        }
        let mut unknown = params_table("min_rr", "1.5");
        unknown.insert("surprise".into(), 1.into());
        assert!(matches!(
            parse_params::<Params>(ID, &unknown),
            Err(ParamError::Parse { .. })
        ));
    }

    #[test]
    fn shipped_config_builds_and_matches_the_defaults() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/strategies.toml");
        let cfg: StrategiesConfig =
            toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let table = cfg.strategy.get(ID).unwrap();
        let sc = StrategyConfig::from_table(ID, table).unwrap();
        assert!(!sc.enabled);
        let parsed: Params = parse_params(ID, &sc.params).unwrap();
        assert_eq!(parsed, Params::default());
        let input = BuildInput {
            params: &sc.params,
            params_hash: "h".into(),
            config: &sc,
        };
        let built = TrendPullbackFactory.build(&input).unwrap();
        assert_eq!(built.meta().id, ID);
        assert_eq!(built.meta().min_history_bars, 50);
    }

    #[test]
    fn reward_to_risk_below_the_floor_and_missing_indicators_give_none() {
        let b = bars_dec(&load("entry"), true);
        // rr at bar K is 1.97; a floor of 2.0 rejects it (built directly, validate() would refuse
        // target_r 2.0 <= min_rr 2.0).
        let p = Params {
            min_rr: 2.0,
            ..Params::default()
        };
        let mut s = TrendPullback::new(meta(p.min_history_bars()), p, "h".into());
        assert!(at(&mut s, &b, K, &world(K)).is_none());
        assert!(why(&s, &b, K, &world(K)).contains("reward:risk"));
        // ATR(14) needs 15 bars: with 10 allowed by the meta it is None.
        let s = TrendPullback::new(meta(10), Params::default(), "h".into());
        assert!(why(&s, &b, 9, &world(9)).contains("atr"));
    }

    #[test]
    fn shipped_config_regime_cells_gate_the_real_strategy() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/strategies.toml");
        let cfg: StrategiesConfig =
            toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let sc = StrategyConfig::from_table(ID, cfg.strategy.get(ID).unwrap()).unwrap();
        let input = BuildInput {
            params: &sc.params,
            params_hash: "h".into(),
            config: &sc,
        };
        let b = bars_dec(&load("entry"), true);
        let fires = |w: &World| {
            let mut s = TrendPullbackFactory.build(&input).unwrap();
            s.on_bar(&DataView::ending_at(&b, K).unwrap(), &w.ctx())
                .is_some()
        };
        let mk = |m, st, sv| World::new(m, V::Normal, st, sv).at(K);
        assert!(fires(&mk(T::Up, T::Up, V::Normal)));
        assert!(fires(&mk(T::Range, T::Up, V::Low)));
        assert!(!fires(&mk(T::Up, T::Up, V::High)));
        for st in [T::Range, T::Down, T::Mixed, T::Unknown] {
            assert!(!fires(&mk(T::Up, st, V::Normal)), "{st:?}");
        }
        let mut stressed = mk(T::Up, T::Up, V::Normal);
        stressed.market.breakdown = bazaar_regime::Tri::True;
        assert!(!fires(&stressed));
    }
}
