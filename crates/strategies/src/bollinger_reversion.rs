//! `bollinger_reversion`: buy a close back inside the lower band in a sideways stock (spec
//! `docs/strategy/F8-first-strategies.md` section 5). Long-only, daily bars, NSE cash equity.
//!
//! * **Idea:** in a sideways stock a close under the lower band that comes back inside often
//!   drifts to the average; short-term overreaction fades. The other side of the trade is panicked
//!   sellers.
//! * **Entry (closed bar t):** `close[t-1] < lower[t-1]`, `close[t] >= lower[t]` and
//!   `close[t] > open[t]`, with bands `SMA(bb_n) -/+ bb_k x stdev`. Entry zone: from that close up
//!   to `entry_zone_atr_frac` x ATR above it.
//! * **Stop:** `min(low[t], low[t-1]) - stop_buffer_atr x ATR`; dropped when `entry_high - stop`
//!   exceeds `max_stop_atr` x ATR.
//! * **Exit:** reference target is the average `SMA(bb_n)` at t; `reference_rr` must be at least
//!   `min_rr` (many setups are dropped here, by design); it is measured from the bottom of the entry
//!   zone, so it is optimistic (fact `rr_basis`). `manage` leaves at the next open when the
//!   stock's trend is confirmed Down, or after `max_hold_bars`. The stop is fixed at entry.
//! * **Best regime:** stock Range with Low or Normal volatility, market Up or Range (declared in
//!   `config/strategies.toml`, never in code). Suppressed in Range x High, Up, Down, Mixed, Unknown
//!   and a market Down or Breakdown.
//! * **Weaknesses:** falling knives when a range breaks (a gap down through the stop fills at the
//!   open, worse than planned); bands widen in a real trend so "cheap" keeps getting cheaper;
//!   targets are small so costs and slippage eat a large share of the edge; results and news gaps.
//!
//! **Low evidence:** Range x Normal is rare, so signals are few and nothing here is backtested; the
//! signal carries a fact saying so. Every default is UNVERIFIED and never tuned on strategy P&L.
//! A signal is not advice. Only closed bars are used. The ATR is Wilder-recursive over the whole
//! view, so callers (scanner, backtest) must pass views of one fixed lookback (the same in both, at
//! least 100 bars) or the stop-cap decision can differ between them.
//!
//! `manage` needs the stock regime (the Down exit), so a regime dated for another bar is an
//! `ExitError::Context`, never a silent hold; unadjusted candles are refused the same way.

use bazaar_core::candle::Candle;
use bazaar_core::dataview::DataView;
use bazaar_core::segment::InstrumentKind;
use bazaar_indicators::bands::Bollinger;
use bazaar_indicators::trend::Atr;
use bazaar_indicators::{Ohlcv, batch};
use bazaar_regime::{Fact, Fit, Trend, label};
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

/// Config id of this strategy.
pub const ID: &str = "bollinger_reversion";
/// Bump when the rules change (provenance).
const VERSION: u32 = 1;
/// Daily-bar NSE cash equity only; the instrument master carries no F&O flag to derive it from.
const SESSION_KIND: InstrumentKind = InstrumentKind::CashNonFno;

/// `[strategy.bollinger_reversion.params]`. Defaults are the spec's; ranges are checked at load.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BollingerParams {
    pub entry_zone_atr_frac: f64,
    pub atr_period: usize,
    pub bb_n: usize,
    pub bb_k: f64,
    pub stop_buffer_atr: f64,
    pub max_stop_atr: f64,
    pub max_hold_bars: u32,
    pub min_rr: f64,
}

impl Default for BollingerParams {
    fn default() -> Self {
        Self {
            entry_zone_atr_frac: 0.25,
            atr_period: 14,
            bb_n: 20,
            bb_k: 2.0,
            stop_buffer_atr: 0.25,
            max_stop_atr: 2.0,
            max_hold_bars: 8,
            min_rr: 1.2,
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

impl ParamSet for BollingerParams {
    /// Ranges are the spec's. No accepted combination is provably empty (the zone and buffer
    /// together use at most 1.0 ATR of the smallest cap, 1.5), but narrow combinations fire rarely:
    /// the candle's own range must fit in what is left of the cap. A test probes the range ends.
    fn validate(&self, id: &str) -> Result<(), ParamError> {
        let f = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        if !f(self.entry_zone_atr_frac, 0.1, 0.5) {
            return Err(bad(id, "entry_zone_atr_frac", "must be 0.1..=0.5"));
        }
        if !(5..=30).contains(&self.atr_period) {
            return Err(bad(id, "atr_period", "must be 5..=30"));
        }
        if !(15..=30).contains(&self.bb_n) {
            return Err(bad(id, "bb_n", "must be 15..=30"));
        }
        if !f(self.bb_k, 1.8, 2.5) {
            return Err(bad(id, "bb_k", "must be 1.8..=2.5"));
        }
        if !f(self.stop_buffer_atr, 0.1, 0.5) {
            return Err(bad(id, "stop_buffer_atr", "must be 0.1..=0.5"));
        }
        if !f(self.max_stop_atr, 1.5, 2.5) {
            return Err(bad(id, "max_stop_atr", "must be 1.5..=2.5"));
        }
        if !(5..=12).contains(&self.max_hold_bars) {
            return Err(bad(id, "max_hold_bars", "must be 5..=12"));
        }
        if !f(self.min_rr, 1.2, 2.0) {
            return Err(bad(id, "min_rr", "must be 1.2..=2.0"));
        }
        Bollinger::new(self.bb_n, self.bb_k).map_err(|_| bad(id, "bb_k", "bad band settings"))?;
        Atr::new(self.atr_period).map_err(|_| bad(id, "atr_period", "bad period"))?;
        Ok(())
    }
}

impl BollingerParams {
    /// Bars needed: the band at t-1 and t (`bb_n` + 1 closes) and the ATR (`atr_period` + 1).
    #[must_use]
    pub fn min_history_bars(&self) -> usize {
        (self.bb_n + 1).max(self.atr_period + 1)
    }
}

/// The strategy. Stateless between bars apart from its params and provenance hash.
#[derive(Debug, Clone)]
pub struct BollingerReversion {
    meta: StrategyMeta,
    params: BollingerParams,
    params_hash: String,
}

/// Builds `bollinger_reversion` from `[strategy.bollinger_reversion]`.
#[derive(Debug, Default, Clone, Copy)]
pub struct BollingerFactory;

impl StrategyFactory for BollingerFactory {
    fn id(&self) -> &'static str {
        ID
    }

    fn build(&self, input: &BuildInput<'_>) -> Result<Box<dyn Strategy>, ParamError> {
        let params: BollingerParams = parse_params(ID, input.params)?;
        let meta = meta_from(
            ID,
            "Bollinger reversion",
            VERSION,
            params.min_history_bars(),
            input.config,
        )?;
        Ok(Box::new(BollingerReversion {
            meta,
            params,
            params_hash: input.params_hash.clone(),
        }))
    }
}

/// What `detect` found on the decision bar.
struct Setup {
    prev_close: f64,
    lower: f64,
    mid: f64,
    low_pair: Decimal,
}

fn fact(label: &str, value: impl Into<String>, meaning: &str) -> Fact {
    Fact::new(label, value, meaning)
}

fn context_fault(reason: impl std::fmt::Display) -> ExitError {
    tracing::warn!(strategy = ID, %reason, "cannot manage the open position");
    ExitError::Context(reason.to_string())
}

impl BollingerReversion {
    fn evaluate(&self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Result<Signal, Reject> {
        let fit = entry_guards(&self.meta, view, ctx)?;
        let setup = self.detect(view)?;
        self.build(view, ctx, fit, &setup)
    }

    /// The cheap tests first (no allocation until the candle is an up bar), then the bands.
    fn detect(&self, view: &DataView<'_>) -> Result<Setup, Reject> {
        let p = &self.params;
        let bars = view.bars();
        let now = view.last().ok_or(Reject::Indicator("bars"))?;
        if now.close() <= now.open() {
            return Err(Reject::Setup("the day did not close above its open"));
        }
        let first = bars.len().saturating_sub(p.bb_n + 1);
        let recent = bars.get(first..).ok_or(Reject::Indicator("bars"))?;
        let closes: Vec<f64> = recent
            .iter()
            .map(|c| Ohlcv::try_from(c).map(|o| o.c))
            .collect::<Result<_, _>>()
            .map_err(|_| Reject::NotFinite)?;
        let bands = Bollinger::new(p.bb_n, p.bb_k).map_err(|_| Reject::Indicator("bands"))?;
        let mut out = batch(bands, closes.iter().copied()).into_iter().rev();
        let band_t = out
            .next()
            .flatten()
            .map(|b| (b.lower, b.mid))
            .ok_or(Reject::Indicator("bands"))?;
        let lower_prev = need("bands", out.next().flatten().map(|b| b.lower))?;
        let close_t = need("close", closes.last().copied())?;
        let prev_close = need(
            "close",
            closes
                .len()
                .checked_sub(2)
                .and_then(|i| closes.get(i))
                .copied(),
        )?;
        if prev_close >= lower_prev {
            return Err(Reject::Setup(
                "yesterday did not close under the lower band",
            ));
        }
        if close_t < band_t.0 {
            return Err(Reject::Setup("today did not close back inside the band"));
        }
        let prev = recent
            .get(recent.len().saturating_sub(2))
            .ok_or(Reject::Indicator("bars"))?;
        Ok(Setup {
            prev_close,
            lower: band_t.0,
            mid: band_t.1,
            low_pair: now.low().min(prev.low()),
        })
    }

    /// Levels, why-text and facts for a detected setup.
    fn build(
        &self,
        view: &DataView<'_>,
        ctx: &StrategyContext<'_>,
        fit: Fit,
        s: &Setup,
    ) -> Result<Signal, Reject> {
        let p = &self.params;
        let now = view.last().ok_or(Reject::Indicator("bars"))?;
        let ohlcv: Vec<Ohlcv> = view
            .bars()
            .iter()
            .map(Ohlcv::try_from)
            .collect::<Result<_, _>>()
            .map_err(|_| Reject::NotFinite)?;
        let atr = Atr::new(p.atr_period).map_err(|_| Reject::Indicator("atr"))?;
        let atr = need("atr", batch(atr, ohlcv).pop().flatten())?;

        let (entry_low, entry_high) = entry_zone(now.close(), atr, p.entry_zone_atr_frac)?;
        let stop = s
            .low_pair
            .checked_sub(to_decimal(p.stop_buffer_atr * atr)?)
            .ok_or(Reject::NotFinite)?;
        let l = round_levels(
            ctx,
            &Levels {
                entry_low,
                entry_high,
                stop,
                target: to_decimal(s.mid)?,
            },
        )?;
        let risk = check_stop(&l, atr, p.max_stop_atr)?;
        let rr = check_rr(&l, p.min_rr)?;

        let (stock_label, market_label) = (label(ctx.stock_regime), label(ctx.market_regime));
        let setup = format!(
            "Setup: {} has been moving sideways, fell below its usual range yesterday and closed back inside today. Why now: prices like this have tended to drift back to the average ({:.2}).",
            ctx.instrument.tradingsymbol, l.target
        );
        let why = why_text(&Why {
            risk_rupees: risk,
            risk_basis: RiskBasis::PerShare,
            setup: &setup,
            regime_label: &market_label,
            proven_wrong_at: l.stop,
            closing: "Watch out: if the stock starts a real downtrend this idea loses; we leave if it does.",
        })?;
        let rule_text = format!(
            "Leave at the next open if the stock's trend is confirmed down, or after {} bars. Reward:risk is measured from the bottom of the entry zone, so it is optimistic.",
            p.max_hold_bars
        );
        let mut facts = band_facts(s, atr, p, &rule_text);
        facts.push(fact(
            WHY_TEXT_LABEL,
            why,
            "Plain-English reason, risk first.",
        ));
        facts.push(fact(
            "stock_regime",
            stock_label,
            "Stock label at the decision bar.",
        ));
        facts.push(fact(
            "market_regime",
            market_label,
            "Market label at the decision bar.",
        ));
        if let Fit::Penalised(f) = fit {
            facts.push(f);
        }
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
            valid_until: valid_until(view, ctx, SESSION_KIND)?,
        };
        sig.validate()
            .map_err(|_| Reject::Setup("signal broke the contract"))?;
        Ok(sig)
    }
}

/// The numbers behind the setup, for the Why panel.
fn band_facts(s: &Setup, atr: f64, p: &BollingerParams, rule_text: &str) -> Vec<Fact> {
    vec![
        fact(
            "lower_band",
            format!("{:.2}", s.lower),
            "Lower band at the decision bar.",
        ),
        fact(
            "prev_close",
            format!("{:.2}", s.prev_close),
            "Yesterday's close, under its band.",
        ),
        fact(
            "middle_band",
            format!("{:.2}", s.mid),
            "Average, the reference target.",
        ),
        fact("atr", format!("{atr:.2}"), "Average daily range."),
        fact(
            "stop_basis",
            format!("{} x ATR under the two-day low", p.stop_buffer_atr),
            "How the stop was set.",
        ),
        fact("exit_rule", rule_text, "How the idea ends."),
        fact(
            "rr_basis",
            "from the bottom of the entry zone (optimistic)",
            "A fill higher in the zone has a lower reward:risk.",
        ),
        fact(
            "time_stop_bars",
            p.max_hold_bars.to_string(),
            "Longest hold in bars.",
        ),
        fact(
            "evidence",
            "low evidence: sideways calm stocks are rare and nothing is backtested yet",
            "Read this as a hypothesis, not a track record.",
        ),
    ]
}

impl Strategy for BollingerReversion {
    fn meta(&self) -> &StrategyMeta {
        &self.meta
    }

    fn on_bar(&mut self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<Signal> {
        match self.evaluate(view, ctx) {
            Ok(s) => Some(s),
            Err(reason) => {
                // Lazy `%` formatting: nothing is allocated unless debug logging is on.
                tracing::debug!(strategy = ID, %reason, "no signal");
                None
            }
        }
    }

    /// Time stop, else leave at the next open when the stock's trend is confirmed Down. The stop is
    /// fixed at entry, so this never tightens it.
    fn manage(
        &mut self,
        view: &DataView<'_>,
        ctx: &StrategyContext<'_>,
        position: &OpenPosition,
    ) -> Result<ExitAction, ExitError> {
        if view.is_empty() {
            return Err(context_fault(Reject::Indicator("bars")));
        }
        let p = &self.params;
        // The time stop needs only `bars_held`, so it comes before any data-quality check.
        if position.bars_held >= p.max_hold_bars {
            return Ok(ExitAction::Exit(format!(
                "time stop: held {} days",
                position.bars_held
            )));
        }
        if !view.bars().iter().all(Candle::adjusted) {
            return Err(context_fault(Reject::Unadjusted));
        }
        ctx.check_closed_bar(view).map_err(context_fault)?;
        if ctx.stock_regime.trend == Trend::Down {
            return Ok(ExitAction::Exit(
                "the stock's trend is confirmed down".to_owned(),
            ));
        }
        Ok(ExitAction::Hold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::on_tick;
    use crate::context::{ContextError, PortfolioView};
    use crate::params::{StrategyConfig, params_hash};
    use crate::testkit::{DRow, World, bars_dec, session_time};
    use bazaar_regime::{Trend as T, VolState as V};

    /// Decision bar of `bollinger_entry.csv` (the 22nd bar).
    const K: usize = 21;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn rows() -> Vec<DRow> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/strategies/bollinger_entry.csv");
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .skip(1)
            .map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                ([d(f[0]), d(f[1]), d(f[2]), d(f[3])], f[4].parse().unwrap())
            })
            .collect()
    }

    fn table() -> toml::Table {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let cfg: bazaar_core::config::StrategiesConfig = bazaar_core::config::load(&dir).unwrap();
        cfg.strategy[ID].clone()
    }

    /// Built through the factory from the shipped config, so the regime fit is the file's.
    fn build_from(t: &toml::Table) -> Result<Box<dyn Strategy>, ParamError> {
        let config = StrategyConfig::from_table(ID, t)?;
        BollingerFactory.build(&BuildInput {
            params: &config.params,
            params_hash: params_hash(ID, t)?,
            config: &config,
        })
    }

    fn strat() -> Box<dyn Strategy> {
        build_from(&table()).unwrap()
    }

    fn world(k: usize) -> World {
        World::new(T::Up, V::Normal, T::Range, V::Normal).at(k)
    }

    fn fire(w: &World, b: &[Candle], k: usize) -> Option<Signal> {
        strat().on_bar(&DataView::ending_at(b, k).unwrap(), &w.ctx())
    }

    fn pos(held: u32, token: u64) -> OpenPosition {
        OpenPosition {
            instrument_token: token,
            entry_price: d("96.8"),
            stop: d("94.40"),
            opened_at: session_time(K),
            bars_held: held,
        }
    }

    fn manage(b: &[Candle], k: usize, w: &World, held: u32) -> Result<ExitAction, ExitError> {
        let p = pos(held, 8);
        strat().manage(&DataView::ending_at(b, k).unwrap(), &w.ctx(), &p)
    }

    #[test]
    fn fires_exactly_on_bar_21_with_hand_computed_levels() {
        // ATR(14) at t = 1.5838 (seed over TR 1..14 = 19.5 / 14), bands: mid 101.99, lower 95.0038;
        // prev close 95.00 < 95.6832.
        // zone top 96.8 + 0.25 x ATR = 97.1959 -> 97.20; stop 94.8 - 0.25 x ATR = 94.4041 -> 94.40
        // (the low of t-1 is the lower of the two lows); target 101.99 -> 102.00 (nearest 0.05);
        // risk 2.80; rr 5.20 / 2.80 = 1.857 -> 1.85.
        let b = bars_dec(&rows(), true);
        let fired: Vec<usize> = (0..b.len())
            .filter(|&k| fire(&world(k), &b, k).is_some())
            .collect();
        assert_eq!(fired, vec![K]);
        let s = fire(&world(K), &b, K).unwrap();
        assert_eq!(
            (s.entry_low, s.entry_high, s.stop),
            (d("96.80"), d("97.20"), d("94.40"))
        );
        assert_eq!(s.targets, vec![d("102.00")]);
        assert_eq!((s.risk_per_share, s.reference_rr), (d("2.80"), d("1.85")));
        assert_eq!((s.exit_plan.time_stop_bars, s.strategy_version), (8, 1));
        assert_eq!(s.signal_bar_time, session_time(K));
        assert!(s.valid_until > s.signal_bar_time);
        assert_eq!(s.validate(), Ok(()));
        let w = world(K);
        for p in [s.entry_low, s.entry_high, s.stop, s.targets[0]] {
            assert!(on_tick(&w.ctx(), p).unwrap(), "{p}");
        }
        let fact = |l: &str| s.facts.iter().find(|f| f.label == l).unwrap().value.clone();
        assert_eq!(fact("lower_band"), "95.00");
        assert_eq!(fact("middle_band"), "101.99");
        assert_eq!(fact("atr"), "1.58");
        assert!(fact("evidence").starts_with("low evidence"));
        let why = fact(WHY_TEXT_LABEL);
        assert_eq!(
            why,
            "Risk first: you could lose about Rs 2.80 per share. Setup: ABC has been moving sideways, fell below its usual range yesterday and closed back inside today. Why now: prices like this have tended to drift back to the average (102.00). Market today: Uptrend, normal swings. Proven wrong if: price falls to or below 94.40. Watch out: if the stock starts a real downtrend this idea loses; we leave if it does."
        );
        assert!(fact("rr_basis").contains("bottom of the entry zone"));
        assert!(s.exit_plan.rule_text.contains("bottom of the entry zone"));
        let lower = why.to_lowercase();
        assert!(
            ["sure", "safe", "guarantee"]
                .iter()
                .all(|w| !lower.contains(w))
        );
    }

    #[test]
    fn tick_rounding_follows_the_instrument_tick() {
        let b = bars_dec(&rows(), true);
        let mut w = world(K);
        w.instrument.tick_size = d("0.25");
        let s = fire(&w, &b, K).unwrap();
        // 96.8 -> 96.75; 97.1959 up -> 97.25; 94.4041 down -> 94.25; 101.99 -> 102.00; risk 3.00.
        assert_eq!(
            (s.entry_low, s.entry_high, s.stop, s.targets[0]),
            (d("96.75"), d("97.25"), d("94.25"), d("102.00"))
        );
        assert_eq!((s.risk_per_share, s.reference_rr), (d("3.00"), d("1.75")));
    }

    #[test]
    fn regime_fit_comes_from_the_shipped_config() {
        let b = bars_dec(&rows(), true);
        let mk = |mt, st, sv| World::new(mt, V::Normal, st, sv).at(K);
        for (mt, sv) in [
            (T::Up, V::Normal),
            (T::Range, V::Low),
            (T::Range, V::Normal),
        ] {
            assert!(
                fire(&mk(mt, T::Range, sv), &b, K).is_some(),
                "{mt:?} {sv:?}"
            );
        }
        for (mt, st, sv) in [
            (T::Up, T::Range, V::High),
            (T::Up, T::Up, V::Normal),
            (T::Up, T::Down, V::Normal),
            (T::Up, T::Mixed, V::Normal),
            (T::Up, T::Unknown, V::Normal),
            (T::Down, T::Range, V::Normal),
            (T::Mixed, T::Range, V::Normal),
        ] {
            assert!(
                fire(&mk(mt, st, sv), &b, K).is_none(),
                "{mt:?} {st:?} {sv:?}"
            );
        }
        for tri in [bazaar_regime::Tri::True, bazaar_regime::Tri::Unknown] {
            let mut stress = mk(T::Up, T::Range, V::Normal);
            stress.market.breakdown = tri;
            assert!(fire(&stress, &b, K).is_none(), "breakdown {tri:?}");
        }
        assert!(fire(&mk(T::Unknown, T::Range, V::Normal), &b, K).is_none());
    }

    #[test]
    fn no_signal_for_each_failed_condition() {
        let base = rows();
        let run = |edit: &dyn Fn(&mut Vec<DRow>), w: &World, adjusted: bool| {
            let mut r = base.clone();
            edit(&mut r);
            fire(w, &bars_dec(&r, adjusted), K)
        };
        let noop = |_: &mut Vec<DRow>| {};
        assert!(run(&noop, &world(K), true).is_some());
        // Yesterday closed inside the band (98 is above the lower band).
        let inside = |r: &mut Vec<DRow>| r[20] = ([d("99.8"), d("100"), d("97.8"), d("98")], 1000);
        assert!(run(&inside, &world(K), true).is_none());
        // Today stays under the lower band (92.0, far below it).
        let under = |r: &mut Vec<DRow>| r[K] = ([d("91.5"), d("92.5"), d("91"), d("92")], 1000);
        assert!(run(&under, &world(K), true).is_none());
        // Today is a down candle (open 97.0 above the close 96.8).
        let down = |r: &mut Vec<DRow>| r[K] = ([d("97"), d("97.2"), d("95"), d("96.8")], 1000);
        assert!(run(&down, &world(K), true).is_none());
        // A deep low on t-1 widens the stop past 2 ATR.
        let deep = |r: &mut Vec<DRow>| r[20] = ([d("99.8"), d("100"), d("85"), d("95")], 1000);
        assert!(run(&deep, &world(K), true).is_none());
        // Unadjusted candles.
        assert!(run(&noop, &world(K), false).is_none());
        // Already held.
        let mut held = world(K);
        held.portfolio = PortfolioView {
            positions: vec![pos(1, held.instrument.instrument_token)],
        };
        assert!(run(&noop, &held, true).is_none());
        // Too little history (21 bars are needed; this view has 21 - 1).
        let b = bars_dec(&base[2..], true);
        assert!(fire(&world(b.len() - 1), &b, b.len() - 1).is_none());
    }

    #[test]
    fn reward_to_risk_below_the_floor_gives_no_signal() {
        let mut t = table();
        t["params"]
            .as_table_mut()
            .unwrap()
            .insert("min_rr".into(), 2.0.into());
        let b = bars_dec(&rows(), true);
        let mut s = build_from(&t).unwrap();
        let view = DataView::ending_at(&b, K).unwrap();
        assert!(s.on_bar(&view, &world(K).ctx()).is_none());
    }

    #[test]
    fn lookahead_sweep_covers_on_bar_and_manage_on_every_bar() {
        let b = bars_dec(&rows(), true);
        for k in 0..b.len() {
            let w = world(k);
            let prefix = &b[..=k];
            // Same bars up to k, wild values after k: the output must not change.
            let mut r = rows();
            for row in r.iter_mut().skip(k + 1) {
                *row = ([d("500"), d("900"), d("1"), d("700")], 9);
            }
            let mutated = bars_dec(&r, true);
            assert_eq!(fire(&w, &b, k), fire(&w, prefix, k), "on_bar {k}");
            assert_eq!(fire(&w, &b, k), fire(&w, &mutated, k), "on_bar mutated {k}");
            for held in [1, 8] {
                assert_eq!(
                    manage(&b, k, &w, held),
                    manage(prefix, k, &w, held),
                    "manage {k}"
                );
                assert_eq!(
                    manage(&b, k, &w, held),
                    manage(&mutated, k, &w, held),
                    "manage mutated {k}"
                );
            }
        }
    }

    #[test]
    fn a_regime_from_after_the_decision_bar_is_refused_with_the_lookahead_error() {
        let b = bars_dec(&rows(), true);
        let mut w = world(K);
        assert!(fire(&w, &b, K).is_some());
        w.market.as_of = session_time(K + 1);
        let view = DataView::ending_at(&b, K).unwrap();
        assert!(matches!(
            w.ctx().check_closed_bar(&view),
            Err(ContextError::RegimeFromTheFuture(_))
        ));
        assert!(fire(&w, &b, K).is_none());
        assert!(matches!(manage(&b, K, &w, 1), Err(ExitError::Context(_))));
    }

    #[test]
    fn manage_exits_on_time_or_a_confirmed_down_trend_and_otherwise_holds() {
        let b = bars_dec(&rows(), true);
        assert_eq!(manage(&b, K, &world(K), 1), Ok(ExitAction::Hold));
        assert_eq!(manage(&b, K, &world(K), 7), Ok(ExitAction::Hold));
        let t = manage(&b, K, &world(K), 8).unwrap();
        assert!(matches!(t, ExitAction::Exit(r) if r.contains("time stop")));
        let down = World::new(T::Up, V::Normal, T::Down, V::Normal).at(K);
        let e = manage(&b, K, &down, 1).unwrap();
        assert!(matches!(e, ExitAction::Exit(r) if r.contains("confirmed down")));
    }

    #[test]
    fn manage_surfaces_bad_context_and_unadjusted_bars_instead_of_holding() {
        let b = bars_dec(&rows(), true);
        // A regime dated for another bar cannot answer "is the trend Down": an error, not Hold;
        // the time stop needs no regime and still fires.
        let stale = world(K - 3);
        assert!(matches!(
            manage(&b, K, &stale, 1),
            Err(ExitError::Context(_))
        ));
        assert!(matches!(manage(&b, K, &stale, 8), Ok(ExitAction::Exit(_))));
        let raw = bars_dec(&rows(), false);
        assert!(matches!(
            manage(&raw, K, &world(K), 1),
            Err(ExitError::Context(_))
        ));
        // A due time stop needs no data-quality check: it fires on unadjusted bars too.
        assert!(matches!(
            manage(&raw, K, &world(K), 8),
            Ok(ExitAction::Exit(_))
        ));
    }

    #[test]
    fn params_are_validated_and_unknown_keys_rejected() {
        let with = |k: &str, v: toml::Value| {
            let mut t = table();
            t["params"].as_table_mut().unwrap().insert(k.into(), v);
            build_from(&t).err()
        };
        for (k, v) in [
            ("bb_n", toml::Value::from(5)),
            ("bb_k", 0.5.into()),
            ("stop_buffer_atr", 0.9.into()),
            ("max_stop_atr", 3.0.into()),
            ("max_hold_bars", 30.into()),
            ("min_rr", 0.5.into()),
            ("entry_zone_atr_frac", 0.9.into()),
            ("atr_period", 2.into()),
            ("nope", 1.into()),
        ] {
            assert!(with(k, v).is_some(), "{k}");
        }
        assert!(with("bb_n", 25.into()).is_none());
        assert_eq!(strat().meta().min_history_bars, 21);
    }

    #[test]
    fn validate_accepts_the_range_ends_and_rejects_just_outside() {
        let ok = |edit: &dyn Fn(&mut BollingerParams)| {
            let mut p = BollingerParams::default();
            edit(&mut p);
            p.validate(ID).is_ok()
        };
        assert!(ok(&|p| p.entry_zone_atr_frac = 0.1));
        assert!(ok(&|p| p.entry_zone_atr_frac = 0.5));
        assert!(!ok(&|p| p.entry_zone_atr_frac = 0.51));
        assert!(!ok(&|p| p.entry_zone_atr_frac = 0.09));
        assert!(ok(&|p| p.stop_buffer_atr = 0.5));
        assert!(!ok(&|p| p.stop_buffer_atr = 0.51));
        assert!(ok(&|p| p.max_stop_atr = 1.5));
        assert!(!ok(&|p| p.max_stop_atr = 1.49));
        assert!(ok(&|p| p.max_stop_atr = 2.5));
        assert!(!ok(&|p| p.max_stop_atr = 2.51));
        assert!(ok(&|p| p.min_rr = 1.2));
        assert!(!ok(&|p| p.min_rr = 1.19));
        // The widest zone + buffer at the narrowest cap is accepted (it just fires rarely).
        assert!(ok(&|p| (
            p.entry_zone_atr_frac,
            p.stop_buffer_atr,
            p.max_stop_atr
        ) = (0.5, 0.5, 1.5)));
        assert!(!ok(&|p| p.bb_k = f64::NAN));
    }

    #[test]
    fn a_target_below_the_entry_or_missing_bands_give_a_reject_not_a_signal() {
        // Today's close (103.0) is above the middle band (about 102): the target is on the wrong
        // side of the entry, so reward:risk fails.
        let mut r = rows();
        r[K] = ([d("95.3"), d("103.5"), d("95"), d("103")], 1000);
        assert!(fire(&world(K), &bars_dec(&r, true), K).is_none());
        // Fewer than bb_n + 1 closes: no band at t-1, so `detect` reports the indicator.
        let s = BollingerReversion {
            meta: strat().meta().clone(),
            params: BollingerParams::default(),
            params_hash: String::new(),
        };
        let b = bars_dec(&rows()[2..], true);
        let view = DataView::ending_at(&b, b.len() - 1).unwrap();
        assert!(matches!(s.detect(&view), Err(Reject::Indicator("bands"))));
    }

    #[test]
    fn shipped_defaults_equal_the_params_default() {
        let p: BollingerParams = parse_params(ID, table()["params"].as_table().unwrap()).unwrap();
        assert_eq!(p, BollingerParams::default());
        assert_eq!(p.validate(ID), Ok(()));
    }
}
