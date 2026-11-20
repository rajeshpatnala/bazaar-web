//! `donchian_breakout`: swing, long-only, daily bars (spec `docs/strategy/F8-first-strategies.md`
//! section 3). Every number below is a config param (UNVERIFIED conventions, never tuned on P&L).
//!
//! - **Idea:** a close above the highest high of the last N days shows buyers pushing past recent
//!   sellers; strong trends often keep going. The other side is late sellers and fade traders.
//! - **Entry (closed bar t):** `close_t > max(high over t-N..t-1)`; `volume_t / SMA(volume over
//!   t-P..t-1) >= vol_ratio_min`; `close_t - channel_high <= max_extension_atr x ATR` (skip gap
//!   chases). Entry zone `[close_t, close_t + entry_zone_atr_frac x ATR]`, filled at the next open.
//! - **Stop:** `entry_high - stop_atr_mult x ATR`, rounded down to the tick. It only ever rises:
//!   [`Strategy::manage`] lifts it to the lowest low of the last `exit_n` bars before t once that is
//!   above it. Its distance is structural (`stop_atr_mult` plus the zone width), so no separate cap.
//! - **Exit:** `close_t < min(low over the last exit_n bars before t)` leaves at the next open, or
//!   after `max_hold_bars`. `targets[0]` is a reference for reward:risk only, not a limit order.
//! - **Best regime:** stock Up x Low/Normal; Up x High is allowed but penalised; market Up or Range.
//!   Suppressed in Range, Down, Mixed, Unknown and a market Breakdown.
//! - **Weaknesses:** many small losses and false breakouts in ranges (the filter reduces, never
//!   removes them); gap-ups fill above the zone; open profit is given back before the exit channel
//!   fires; results-day gaps are blind spots; crowded breakouts reverse in market stress.
//!
//! - **Notes:** `reward:risk` is about 2 by construction (the reference target is 2 x risk), so
//!   `min_rr` can only reject on tick rounding; it is not evidence about the setup. The stop width
//!   is capped in ATR terms only; its width as a percentage of price (a results-day ATR blow-up) is
//!   left to the gate and sizing. ATR is Wilder-recursive, so callers (scanner, backtest) must pass
//!   views of one fixed lookback; a test bounds the difference between 60 and 130 bars. Stops are
//!   resting orders that can trigger intraday; the backtest must model fills at `min(stop, open)`.
//!   The why-text is per share; the scanner rebuilds it with whole-trade risk.
//!   `manage` needs no regime: it checks only the view (adjusted candles) and the tick.
//!
//! A signal is not a recommendation and not advice.

use bazaar_core::candle::Candle;
use bazaar_core::dataview::DataView;
use bazaar_core::segment::InstrumentKind;
use bazaar_indicators::ma::Sma;
use bazaar_indicators::trend::Atr;
use bazaar_indicators::{Ohlcv, batch};
use bazaar_regime::{Fact, Fit, label};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::common::{
    Levels, Reject, RiskBasis, WHY_TEXT_LABEL, Why, check_rr, entry_guards, entry_zone,
    finish_levels, floor_to_tick, need, to_decimal, why_text,
};
use crate::context::StrategyContext;
use crate::exit::{ExitAction, ExitError, OpenPosition};
use crate::factory::{BuildInput, StrategyFactory, meta_from};
use crate::params::{ParamError, ParamSet, parse_params};
use crate::signal::{ExitPlan, Signal, risk_per_share};
use crate::strategy::{Strategy, StrategyMeta};

/// Config id of this strategy.
pub const ID: &str = "donchian_breakout";
/// Bump when the rules change (provenance).
const VERSION: u32 = 1;
/// Session kind for the next-session close: this strategy trades daily-bar NSE cash equity only,
/// and the instrument master carries no F&O-membership flag to derive it from (spec section 2).
const SESSION_KIND: InstrumentKind = InstrumentKind::CashNonFno;
/// Reference target in multiples of risk (spec section 3: `entry_low + 2 x risk`). Shown for
/// reward:risk only; no order is placed at it, so it is not a tunable threshold.
const REFERENCE_TARGET_R: Decimal = Decimal::TWO;

/// `[strategy.donchian_breakout.params]`. Defaults are the spec's; ranges are checked at load.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DonchianParams {
    pub entry_zone_atr_frac: f64,
    pub atr_period: usize,
    pub entry_n: usize,
    pub exit_n: usize,
    pub stop_atr_mult: f64,
    pub volume_sma_period: usize,
    pub vol_ratio_min: f64,
    pub max_extension_atr: f64,
    pub max_hold_bars: u32,
    pub min_rr: f64,
}

impl Default for DonchianParams {
    fn default() -> Self {
        Self {
            entry_zone_atr_frac: 0.25,
            atr_period: 14,
            entry_n: 20,
            exit_n: 10,
            stop_atr_mult: 2.5,
            volume_sma_period: 20,
            vol_ratio_min: 1.2,
            max_extension_atr: 1.0,
            max_hold_bars: 60,
            min_rr: 1.5,
        }
    }
}

fn bad(key: &'static str, reason: &'static str) -> ParamError {
    ParamError::Invalid {
        id: ID.to_owned(),
        key,
        reason,
    }
}

fn in_f(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && v >= lo && v <= hi
}

impl ParamSet for DonchianParams {
    fn validate(&self, _id: &str) -> Result<(), ParamError> {
        let checks = [
            (
                in_f(self.entry_zone_atr_frac, 0.1, 0.5),
                "entry_zone_atr_frac",
                "must be 0.1-0.5",
            ),
            (
                (5..=30).contains(&self.atr_period),
                "atr_period",
                "must be 5-30",
            ),
            (
                (15..=30).contains(&self.entry_n),
                "entry_n",
                "must be 15-30",
            ),
            ((7..=14).contains(&self.exit_n), "exit_n", "must be 7-14"),
            (
                in_f(self.stop_atr_mult, 2.0, 3.0),
                "stop_atr_mult",
                "must be 2.0-3.0",
            ),
            (
                (10..=30).contains(&self.volume_sma_period),
                "volume_sma_period",
                "must be 10-30",
            ),
            (
                in_f(self.vol_ratio_min, 1.0, 1.5),
                "vol_ratio_min",
                "must be 1.0-1.5",
            ),
            (
                in_f(self.max_extension_atr, 0.5, 1.5),
                "max_extension_atr",
                "must be 0.5-1.5",
            ),
            (
                (40..=80).contains(&self.max_hold_bars),
                "max_hold_bars",
                "must be 40-80",
            ),
            (in_f(self.min_rr, 1.5, 2.0), "min_rr", "must be 1.5-2.0"),
        ];
        match checks.into_iter().find(|(ok, _, _)| !ok) {
            Some((_, key, reason)) => Err(bad(key, reason)),
            None => Ok(()),
        }
    }
}

impl DonchianParams {
    /// Closed bars needed: the longest lookback plus the decision bar (ATR needs one extra).
    #[must_use]
    pub fn min_history_bars(&self) -> usize {
        self.entry_n
            .max(self.volume_sma_period)
            .max(self.exit_n)
            .max(self.atr_period)
            + 1
    }
}

/// The strategy. Stateless between bars apart from its params and provenance hash.
#[derive(Debug, Clone)]
pub struct DonchianBreakout {
    meta: StrategyMeta,
    params: DonchianParams,
    params_hash: String,
}

/// Builds `donchian_breakout` from `[strategy.donchian_breakout]`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DonchianFactory;

impl StrategyFactory for DonchianFactory {
    fn id(&self) -> &'static str {
        ID
    }

    fn build(&self, input: &BuildInput<'_>) -> Result<Box<dyn Strategy>, ParamError> {
        let params: DonchianParams = parse_params(ID, input.params)?;
        let meta = meta_from(
            ID,
            "Donchian breakout",
            VERSION,
            params.min_history_bars(),
            input.config,
        )?;
        Ok(Box::new(DonchianBreakout {
            meta,
            params,
            params_hash: input.params_hash.clone(),
        }))
    }
}

/// The `n` bars before the last one (the decision bar is excluded: closed-bar channel).
fn before_last<T>(xs: &[T], n: usize) -> Option<&[T]> {
    let end = xs.len().checked_sub(1)?;
    xs.get(end.checked_sub(n)?..end)
}

/// What `detect` found on the decision bar.
struct Setup {
    channel_high: Decimal,
    close: Decimal,
    volume_ratio: f64,
    atr: f64,
}

fn fact(label: &str, value: impl Into<String>, meaning: &str) -> Fact {
    Fact::new(label, value, meaning)
}

impl DonchianBreakout {
    fn evaluate(&self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Result<Signal, Reject> {
        let fit = entry_guards(&self.meta, view, ctx)?;
        let setup = self.detect(view)?;
        self.build(view, ctx, fit, &setup)
    }

    /// The breakout test itself: channel, volume and extension, on closed bars only.
    fn detect(&self, view: &DataView<'_>) -> Result<Setup, Reject> {
        let p = &self.params;
        let bars = view.bars();
        let last = view.last().ok_or(Reject::Indicator("bars"))?;

        let ohlcv: Vec<Ohlcv> = bars
            .iter()
            .map(Ohlcv::try_from)
            .collect::<Result<_, _>>()
            .map_err(|_| Reject::NotFinite)?;
        let atr = Atr::new(p.atr_period).map_err(|_| Reject::Indicator("atr"))?;
        let atr_v = need("atr", batch(atr, ohlcv.iter().copied()).pop().flatten())?;

        let channel = before_last(bars, p.entry_n).ok_or(Reject::Indicator("channel"))?;
        let channel_high = channel
            .iter()
            .map(Candle::high)
            .max()
            .ok_or(Reject::Indicator("channel"))?;
        let close = last.close();
        if close <= channel_high {
            return Err(Reject::Setup("close is not above the channel high"));
        }

        let vols: Vec<f64> = before_last(&ohlcv, p.volume_sma_period)
            .ok_or(Reject::Indicator("volume_sma"))?
            .iter()
            .map(|o| o.v)
            .collect();
        let sma = Sma::new(p.volume_sma_period).map_err(|_| Reject::Indicator("volume_sma"))?;
        let vol_avg = need("volume_sma", batch(sma, vols).pop().flatten())?;
        let vol_now = ohlcv.last().map_or(0.0, |o| o.v);
        if vol_avg <= 0.0 {
            return Err(Reject::Indicator("volume_sma"));
        }
        let volume_ratio = vol_now / vol_avg;
        if volume_ratio < p.vol_ratio_min {
            return Err(Reject::Setup("volume is below the confirmation ratio"));
        }

        let atr_d = to_decimal(atr_v)?;
        let extension = close - channel_high;
        let max_ext = atr_d
            .checked_mul(to_decimal(p.max_extension_atr)?)
            .ok_or(Reject::NotFinite)?;
        if extension > max_ext {
            return Err(Reject::Setup("close is stretched above the channel"));
        }
        Ok(Setup {
            channel_high,
            close,
            volume_ratio,
            atr: atr_v,
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
        let last = view.last().ok_or(Reject::Indicator("bars"))?;
        let (close, volume_ratio, atr_v) = (s.close, s.volume_ratio, s.atr);
        let atr_d = to_decimal(atr_v)?;
        let (entry_low, entry_high) = entry_zone(close, atr_v, p.entry_zone_atr_frac)?;
        let stop_dist = atr_d
            .checked_mul(to_decimal(p.stop_atr_mult)?)
            .ok_or(Reject::NotFinite)?;
        // Risk from the top of the zone to the stop is exactly the stop distance.
        let raw_stop = entry_high - stop_dist;
        let raw_target = entry_low
            .checked_add(
                REFERENCE_TARGET_R
                    .checked_mul(stop_dist)
                    .ok_or(Reject::NotFinite)?,
            )
            .ok_or(Reject::NotFinite)?;
        let lv = finish_levels(
            ctx,
            &Levels {
                entry_low,
                entry_high,
                stop: raw_stop,
                target: raw_target,
            },
        )?;
        let reference_rr = check_rr(&lv, p.min_rr)?;
        let valid_until = ctx.next_session_close(view, SESSION_KIND)?;
        let risk = risk_per_share(lv.entry_high, lv.stop);

        let exit_rule = format!(
            "leave if it closes below its {}-day low or after {} days",
            p.exit_n, p.max_hold_bars
        );
        let regime_label = label(ctx.market_regime);
        let why = why_text(&Why {
            risk_rupees: risk,
            risk_basis: RiskBasis::PerShare,
            setup: &format!(
                "Setup: {} closed above its {}-day high on {:.1}x normal volume.",
                ctx.instrument.tradingsymbol, p.entry_n, volume_ratio
            ),
            regime_label: &regime_label,
            proven_wrong_at: lv.stop,
            closing: &format!(
                "This setup likes uptrends. Exit plan: we {exit_rule}. Watch out: breakouts fail often; many small losses are normal for this idea."
            ),
        })?;

        let fit_fact = match fit {
            Fit::Allowed(f) | Fit::Penalised(f) => f,
            Fit::Suppressed(f) => return Err(Reject::RegimeFit(f.meaning)),
        };
        let facts = self.facts(ctx, s, &exit_rule, [regime_label, why], fit_fact);

        let signal = Signal {
            strategy_id: ID,
            instrument_token: ctx.instrument.instrument_token,
            strategy_version: VERSION,
            params_hash: self.params_hash.clone(),
            signal_bar_time: last.close_time(),
            entry_low: lv.entry_low,
            entry_high: lv.entry_high,
            stop: lv.stop,
            targets: vec![lv.target],
            risk_per_share: risk,
            reference_rr,
            exit_plan: ExitPlan {
                time_stop_bars: p.max_hold_bars,
                rule_text: exit_rule,
            },
            facts,
            valid_until,
        };
        signal
            .validate()
            .map_err(|_| Reject::Setup("signal broke its contract"))?;
        Ok(signal)
    }

    /// The Why-panel facts of spec section 3, plus the fit line and the why-text itself.
    fn facts(
        &self,
        ctx: &StrategyContext<'_>,
        s: &Setup,
        exit_rule: &str,
        [market_label, why]: [String; 2],
        fit_fact: Fact,
    ) -> Vec<Fact> {
        let p = &self.params;
        vec![
            fact(
                "channel_high",
                s.channel_high.to_string(),
                "highest high of the entry channel, bars before the signal bar",
            ),
            fact(
                "breakout_close",
                s.close.to_string(),
                "close of the signal bar",
            ),
            fact(
                "volume_ratio",
                format!("{:.2}", s.volume_ratio),
                "signal-bar volume over its recent average",
            ),
            fact(
                "atr",
                format!("{:.2}", s.atr),
                "average true range, the yardstick for zone and stop",
            ),
            fact(
                "stop_basis",
                format!("{} x ATR below the top of the entry zone", p.stop_atr_mult),
                "initial stop; it can only be raised",
            ),
            fact("exit_rule", exit_rule, "how the idea ends"),
            fact(
                "time_stop_bars",
                p.max_hold_bars.to_string(),
                "latest exit, in bars",
            ),
            fact(
                "stock_regime",
                label(ctx.stock_regime),
                "stock trend and swing label",
            ),
            fact("market_regime", market_label, "market trend label"),
            fit_fact,
            fact(
                "target_basis",
                "entry_low + 2 x risk",
                "reference only, for reward:risk; no order is placed at it",
            ),
            fact(WHY_TEXT_LABEL, why, "plain-English summary"),
        ]
    }

    fn decide_exit(
        &self,
        view: &DataView<'_>,
        ctx: &StrategyContext<'_>,
        position: &OpenPosition,
    ) -> Result<ExitAction, Reject> {
        // No regime needed to manage an exit; only the view must be usable. Unadjusted candles
        // could show a split as a crash and fire a false exit.
        let last = view.last().ok_or(Reject::Indicator("bars"))?;
        if !view.bars().iter().all(Candle::adjusted) {
            return Err(Reject::Unadjusted);
        }
        let p = &self.params;
        if position.bars_held >= p.max_hold_bars {
            return Ok(ExitAction::Exit(format!(
                "time stop: held {} days",
                position.bars_held
            )));
        }
        let bars = view.bars();
        let Some(window) = before_last(bars, p.exit_n) else {
            return Ok(ExitAction::Hold);
        };
        let low = window
            .iter()
            .map(Candle::low)
            .min()
            .ok_or(Reject::Indicator("exit_channel"))?;
        if last.close() < low {
            return Ok(ExitAction::Exit(format!(
                "closed below its {}-day low",
                p.exit_n
            )));
        }
        let lifted = floor_to_tick(ctx, low)?;
        if lifted > position.stop && lifted < last.close() {
            return Ok(ExitAction::TightenStop(lifted));
        }
        Ok(ExitAction::Hold)
    }
}

impl Strategy for DonchianBreakout {
    fn meta(&self) -> &StrategyMeta {
        &self.meta
    }

    fn on_bar(&mut self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<Signal> {
        match self.evaluate(view, ctx) {
            Ok(s) => Some(s),
            Err(reason) => {
                tracing::debug!(strategy = ID, %reason, "no signal");
                None
            }
        }
    }

    fn manage(
        &mut self,
        view: &DataView<'_>,
        ctx: &StrategyContext<'_>,
        position: &OpenPosition,
    ) -> Result<ExitAction, ExitError> {
        let action = self.decide_exit(view, ctx, position).map_err(|reason| {
            tracing::warn!(strategy = ID, %reason, "cannot manage the open position");
            ExitError::Context(reason.to_string())
        })?;
        action.check(position)?;
        Ok(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::PortfolioView;
    use crate::params::{StrategyConfig, params_hash};
    use crate::testkit::{DRow, World, bars_dec, session_time};
    use bazaar_regime::{Trend as T, VolState as V};

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn csv(name: &str) -> Vec<DRow> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/strategies")
            .join(name);
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

    fn build_from(t: &toml::Table) -> Result<Box<dyn Strategy>, ParamError> {
        let config = StrategyConfig::from_table(ID, t)?;
        DonchianFactory.build(&BuildInput {
            params: &config.params,
            params_hash: params_hash(ID, t)?,
            config: &config,
        })
    }

    fn strat() -> Box<dyn Strategy> {
        build_from(&table()).unwrap()
    }

    fn world(k: usize) -> World {
        World::new(T::Up, V::Normal, T::Up, V::Normal).at(k)
    }

    fn fire(s: &mut dyn Strategy, w: &World, b: &[Candle], k: usize) -> Option<Signal> {
        s.on_bar(&DataView::ending_at(b, k).unwrap(), &w.ctx())
    }

    fn firing_bars(rows: &[DRow]) -> Vec<usize> {
        let b = bars_dec(rows, true);
        let mut s = strat();
        (0..b.len())
            .filter(|&k| fire(s.as_mut(), &world(k), &b, k).is_some())
            .collect()
    }

    #[test]
    fn fires_exactly_on_bar_30_with_hand_computed_levels() {
        // ATR(30) = (13 x 2 + 3) / 14 = 2.0714; zone top 102.5 + 0.25 x ATR = 103.0179 -> 103.05;
        // stop 103.0179 - 2.5 x ATR = 97.8393 -> 97.80; target 102.5 + 2 x 5.1786 = 112.857 -> 112.85;
        // rr = 10.35 / 5.25 = 1.97.
        let rows = csv("donchian_entry.csv");
        assert_eq!(firing_bars(&rows), vec![30]);
        let b = bars_dec(&rows, true);
        let s = fire(strat().as_mut(), &world(30), &b, 30).unwrap();
        assert_eq!(s.entry_low, d("102.50"));
        assert_eq!(s.entry_high, d("103.05"));
        assert_eq!(s.stop, d("97.80"));
        assert_eq!(s.targets, vec![d("112.85")]);
        assert_eq!(s.risk_per_share, d("5.25"));
        assert_eq!(s.reference_rr, d("1.97"));
        assert_eq!(s.signal_bar_time, session_time(30));
        assert!(s.valid_until > s.signal_bar_time);
        assert_eq!((s.exit_plan.time_stop_bars, s.strategy_version), (60, 1));
        assert_eq!(s.params_hash.len(), 64);
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn why_text_names_risk_first_regime_and_stop_without_certainty_words() {
        let b = bars_dec(&csv("donchian_entry.csv"), true);
        let s = fire(strat().as_mut(), &world(30), &b, 30).unwrap();
        let why = &s
            .facts
            .iter()
            .find(|f| f.label == WHY_TEXT_LABEL)
            .unwrap()
            .value;
        assert!(why.starts_with("Risk first: you could lose about Rs 5.25 per share."));
        assert!(why.contains("ABC closed above its 20-day high on 1.5x normal volume."));
        assert!(why.contains("Market today:"));
        assert!(why.contains("Proven wrong if: price falls to or below 97.80."));
        assert!(why.contains("closes below its 10-day low or after 60 days"));
        let lower = why.to_lowercase();
        for w in ["sure", "safe", "guarantee"] {
            assert!(!lower.contains(w));
        }
        for l in [
            "channel_high",
            "volume_ratio",
            "atr",
            "stock_regime",
            "market_regime",
            "target_basis",
        ] {
            assert!(s.facts.iter().any(|f| f.label == l), "{l}");
        }
    }

    #[test]
    fn risk_is_within_the_atr_stop_width_plus_two_ticks() {
        // stop_atr_mult 2.5 x ATR(30) = 2.5 x 29/14 = 5.1786; rounding adds at most 2 ticks
        // (entry zone top up, stop down). Width as a % of price is left to the gate.
        let b = bars_dec(&csv("donchian_entry.csv"), true);
        for tick in ["0.05", "0.5"] {
            let mut w = world(30);
            w.instrument.tick_size = d(tick);
            let s = fire(strat().as_mut(), &w, &b, 30).unwrap();
            let cap = d("5.1786") + d(tick) * Decimal::TWO;
            assert!(s.risk_per_share <= cap, "{tick}: {}", s.risk_per_share);
            assert!(s.risk_per_share >= d("5.1785"), "{tick}");
        }
    }

    #[test]
    fn atr_depends_only_weakly_on_the_view_length() {
        // Callers must pass one fixed lookback; this bounds the damage when they do not.
        let rows: Vec<Ohlcv> = (0..130_u32)
            .map(|i| {
                let w = f64::from(i % 7);
                Ohlcv {
                    ts_ms: 0,
                    o: 100.0 + w,
                    h: 103.0 + w,
                    l: 99.0 + w,
                    c: 101.0 + w,
                    v: 1000.0,
                }
            })
            .collect();
        let atr = |r: &[Ohlcv]| {
            batch(Atr::new(30).unwrap(), r.iter().copied())
                .pop()
                .flatten()
                .unwrap()
        };
        let (long, short) = (atr(&rows), atr(&rows[70..]));
        assert!((long - short).abs() < 0.05, "{long} vs {short}");
    }

    #[test]
    fn tick_rounding_follows_the_instrument_tick() {
        let b = bars_dec(&csv("donchian_entry.csv"), true);
        let mut w = world(30);
        w.instrument.tick_size = d("0.5");
        let s = fire(strat().as_mut(), &w, &b, 30).unwrap();
        // zone top 103.0179 -> 103.5 (up); stop 97.8393 -> 97.5 (down); target 112.857 -> 113.0.
        assert_eq!(
            (s.entry_low, s.entry_high, s.stop, s.targets[0]),
            (d("102.5"), d("103.5"), d("97.5"), d("113.0"))
        );
        assert_eq!(s.reference_rr, d("1.75"));
    }

    #[test]
    fn no_signal_for_each_failed_condition() {
        let mut rows = csv("donchian_entry.csv");
        let last = 30;
        let base = rows[last];
        let run = |rows: &[DRow], w: &World, adjusted: bool| {
            let b = bars_dec(rows, adjusted);
            fire(strat().as_mut(), w, &b, last)
        };
        // Low volume (1100 / 1000 = 1.1 < 1.2).
        rows[last].1 = 1100;
        assert!(run(&rows, &world(last), true).is_none());
        // Stretched: close 104 is 3 above the channel, over 1 x ATR.
        rows[last] = ([d("101"), d("105"), d("100"), d("104")], 1500);
        assert!(run(&rows, &world(last), true).is_none());
        // Close not above the channel high (101).
        rows[last] = ([d("100"), d("101"), d("99"), d("101")], 1500);
        assert!(run(&rows, &world(last), true).is_none());
        rows[last] = base;
        assert!(run(&rows, &world(last), true).is_some());
        // Unadjusted candles, already held, wrong regimes.
        assert!(run(&rows, &world(last), false).is_none());
        let mut held = world(last);
        held.portfolio = PortfolioView {
            positions: vec![OpenPosition {
                instrument_token: held.instrument.instrument_token,
                entry_price: d("100"),
                stop: d("90"),
                opened_at: session_time(1),
                bars_held: 1,
            }],
        };
        assert!(run(&rows, &held, true).is_none());
        for (mt, st) in [
            (T::Down, T::Up),
            (T::Up, T::Range),
            (T::Up, T::Down),
            (T::Up, T::Mixed),
            (T::Up, T::Unknown),
            (T::Mixed, T::Up),
        ] {
            let w = World::new(mt, V::Normal, st, V::Normal).at(last);
            assert!(run(&rows, &w, true).is_none(), "{mt:?} {st:?}");
        }
        let mut stress = world(last);
        stress.market.breakdown = bazaar_regime::Tri::True;
        assert!(run(&rows, &stress, true).is_none());
        // Up x High is allowed (penalised), not suppressed.
        let wild = World::new(T::Up, V::Normal, T::Up, V::High).at(last);
        assert!(run(&rows, &wild, true).is_some());
    }

    #[test]
    fn short_history_and_low_reward_risk_give_no_signal() {
        let rows = csv("donchian_entry.csv");
        // Fewer than min_history_bars (21) closed bars.
        let b = bars_dec(&rows[..20], true);
        assert!(fire(strat().as_mut(), &world(19), &b, 19).is_none());
        // rr 1.97 is below a floor of 2.0 (the top of the allowed range).
        let mut t = table();
        t["params"]
            .as_table_mut()
            .unwrap()
            .insert("min_rr".into(), 2.0.into());
        let mut s = build_from(&t).unwrap();
        let b = bars_dec(&rows, true);
        assert!(fire(s.as_mut(), &world(30), &b, 30).is_none());
    }

    #[test]
    fn lookahead_later_bars_never_change_earlier_outputs() {
        let rows = csv("donchian_exit.csv");
        let b = bars_dec(&rows, true);
        let all: Vec<Option<Signal>> = {
            let mut s = strat();
            (0..b.len())
                .map(|k| fire(s.as_mut(), &world(k), &b, k))
                .collect()
        };
        for cut in [25, 30, 31, 35] {
            let part = bars_dec(&rows[..=cut], true);
            let mut s = strat();
            for (k, want) in all.iter().enumerate().take(cut + 1) {
                assert_eq!(
                    fire(s.as_mut(), &world(k), &part, k),
                    *want,
                    "cut {cut} bar {k}"
                );
            }
        }
        assert_eq!(all.iter().position(Option::is_some), Some(30));
    }

    #[test]
    fn a_regime_from_after_the_decision_bar_is_refused_with_the_lookahead_error() {
        let b = bars_dec(&csv("donchian_entry.csv"), true);
        let mut w = world(30);
        assert!(fire(strat().as_mut(), &w, &b, 30).is_some());
        w.market.as_of = session_time(31);
        let view = DataView::ending_at(&b, 30).unwrap();
        assert!(matches!(
            w.ctx().check_closed_bar(&view),
            Err(crate::context::ContextError::RegimeFromTheFuture(_))
        ));
        assert!(fire(strat().as_mut(), &w, &b, 30).is_none());
    }

    fn pos(stop: &str, held: u32) -> OpenPosition {
        OpenPosition {
            instrument_token: 7,
            entry_price: d("102.5"),
            stop: d(stop),
            opened_at: session_time(30),
            bars_held: held,
        }
    }

    fn manage(file: &str, k: usize, p: &OpenPosition) -> Result<ExitAction, ExitError> {
        let b = bars_dec(&csv(file), true);
        strat().manage(&DataView::ending_at(&b, k).unwrap(), &world(k).ctx(), p)
    }

    #[test]
    fn manage_lifts_the_stop_to_the_exit_channel_low_and_never_lowers_it() {
        // Lows of bars 31..40 are 102..111: the 10-day low is 102, above the stop 97.80.
        assert_eq!(
            manage("donchian_trail.csv", 41, &pos("97.80", 11)),
            Ok(ExitAction::TightenStop(d("102.00")))
        );
        assert_eq!(
            manage("donchian_trail.csv", 41, &pos("102.00", 11)),
            Ok(ExitAction::Hold)
        );
        assert_eq!(
            manage("donchian_trail.csv", 41, &pos("105.00", 11)),
            Ok(ExitAction::Hold)
        );
    }

    #[test]
    fn manage_exits_on_a_close_below_the_channel_or_the_time_stop() {
        let e = manage("donchian_exit.csv", 41, &pos("97.80", 11)).unwrap();
        assert_eq!(e, ExitAction::Exit("closed below its 10-day low".into()));
        let t = manage("donchian_trail.csv", 41, &pos("97.80", 60)).unwrap();
        assert!(matches!(t, ExitAction::Exit(r) if r.contains("time stop")));
        // Short history (bar 5): too few bars for the channel, so Hold, but the time stop
        // still wins over the short-history path.
        assert_eq!(
            manage("donchian_trail.csv", 5, &pos("97.80", 1)),
            Ok(ExitAction::Hold)
        );
        let t = manage("donchian_trail.csv", 5, &pos("97.80", 60)).unwrap();
        assert!(matches!(t, ExitAction::Exit(r) if r.contains("time stop")));
        assert_eq!(
            manage("donchian_trail.csv", 5, &pos("97.80", 59)),
            Ok(ExitAction::Hold)
        );
    }

    #[test]
    fn manage_needs_no_regime_but_surfaces_bad_context_and_unadjusted_bars() {
        let rows = csv("donchian_trail.csv");
        let b = bars_dec(&rows, true);
        let view = DataView::ending_at(&b, 41).unwrap();
        // A regime dated for another bar does not matter to an exit.
        let stale = world(30);
        assert_eq!(
            strat().manage(&view, &stale.ctx(), &pos("97.80", 11)),
            Ok(ExitAction::TightenStop(d("102.00")))
        );
        // A zero tick is a data fault, not "hold".
        let mut bad = world(41);
        bad.instrument.tick_size = Decimal::ZERO;
        assert!(matches!(
            strat().manage(&view, &bad.ctx(), &pos("97.80", 11)),
            Err(ExitError::Context(_))
        ));
        // Unadjusted candles could fake a crash: refuse rather than exit.
        let raw = bars_dec(&csv("donchian_exit.csv"), false);
        let view = DataView::ending_at(&raw, 41).unwrap();
        assert!(matches!(
            strat().manage(&view, &world(41).ctx(), &pos("97.80", 11)),
            Err(ExitError::Context(_))
        ));
    }

    #[test]
    fn manage_lookahead_and_the_trailed_stop_never_decreases() {
        let rows = csv("donchian_trail.csv");
        let b = bars_dec(&rows, true);
        let run = |bars: &[Candle], k: usize, stop: Decimal| {
            let p = OpenPosition {
                stop,
                ..pos("97.80", 11)
            };
            strat().manage(&DataView::ending_at(bars, k).unwrap(), &world(k).ctx(), &p)
        };
        let mut stop = d("97.80");
        for k in 31..b.len() {
            let full = run(&b, k, stop);
            for cut in [k + 1, b.len()] {
                assert_eq!(run(&b[..cut], k, stop), full, "k {k} cut {cut}");
            }
            if let Ok(ExitAction::TightenStop(s)) = full {
                assert!(s > stop, "k {k}");
                stop = s;
            }
        }
        assert!(stop > d("97.80"));
    }

    #[test]
    fn params_are_validated_and_unknown_keys_rejected() {
        let with = |k: &str, v: toml::Value| {
            let mut t = table();
            t["params"].as_table_mut().unwrap().insert(k.into(), v);
            build_from(&t).err()
        };
        assert!(with("entry_n", 5.into()).is_some());
        assert!(with("exit_n", 20.into()).is_some());
        assert!(with("stop_atr_mult", 5.0.into()).is_some());
        assert!(with("vol_ratio_min", 0.5.into()).is_some());
        assert!(with("max_hold_bars", 5.into()).is_some());
        assert!(with("entry_zone_atr_frac", 0.9.into()).is_some());
        assert!(with("nope", 1.into()).is_some());
        assert!(with("entry_n", 25.into()).is_none());
        assert_eq!(DonchianParams::default().validate(ID), Ok(()));
        assert_eq!(strat().meta().min_history_bars, 21);
    }

    #[test]
    fn shipped_defaults_equal_the_params_default() {
        let p: DonchianParams = parse_params(ID, table()["params"].as_table().unwrap()).unwrap();
        assert_eq!(p, DonchianParams::default());
    }
}
