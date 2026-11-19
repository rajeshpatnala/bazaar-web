//! Shared plumbing for the entry rules: guards, tick rounding, entry zone, `valid_until` and the
//! why-text helper. No threshold lives here: every limit (min history, caps, `min_rr`, zone width)
//! is passed in from the strategy's params, and the tick comes from the instrument master.
//! Indicator maths is f64; [`to_decimal`] is the one boundary where it becomes money.

use bazaar_core::dataview::DataView;
use bazaar_core::segment::InstrumentKind;
use bazaar_regime::{Fit, Trend, Tri, label};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

use crate::context::{ContextError, StrategyContext};
use crate::strategy::StrategyMeta;

/// Why a strategy returned no signal. Strategies log it at `debug` and return `None`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Reject {
    #[error("closed-bar check failed: {0}")]
    ClosedBar(#[from] ContextError),
    #[error("only {have} bars, need {need}")]
    MinHistory { have: usize, need: usize },
    #[error("indicator {0} is not available")]
    Indicator(&'static str),
    #[error("candles are not corporate-action adjusted")]
    Unadjusted,
    #[error("instrument is already held")]
    Held,
    #[error("market gate: {0}")]
    MarketGate(String),
    #[error("regime fit: {0}")]
    RegimeFit(String),
    #[error("tick size must be positive")]
    BadTick,
    #[error("a price or ratio is not a finite number")]
    NotFinite,
    #[error("stop is not below the entry zone")]
    WrongSideStop,
    #[error("stop is {distance} away, cap is {cap}")]
    StopTooWide { distance: Decimal, cap: Decimal },
    #[error("reward:risk {rr} is below the floor {min_rr}")]
    RrTooLow { rr: Decimal, min_rr: Decimal },
    #[error("why-text contains the forbidden word {0:?}")]
    ForbiddenWord(&'static str),
}

/// Words a why-text must never carry (we never imply certainty).
const FORBIDDEN: [&str; 3] = ["sure", "safe", "guaranteed"];

/// The one f64 -> Decimal boundary.
///
/// # Errors
/// `NotFinite` for NaN or infinity.
pub fn to_decimal(x: f64) -> Result<Decimal, Reject> {
    if !x.is_finite() {
        return Err(Reject::NotFinite);
    }
    Decimal::from_f64(x).ok_or(Reject::NotFinite)
}

/// Unwrap a needed indicator value: `None` or non-finite means no signal.
///
/// # Errors
/// `Indicator(name)`.
pub fn need(name: &'static str, v: Option<f64>) -> Result<f64, Reject> {
    v.filter(|x| x.is_finite()).ok_or(Reject::Indicator(name))
}

/// Guards run first by every strategy, in this order: closed bar, minimum history, adjusted
/// candles, not held, market gate (trend Up or Range, never Breakdown), then the strategy's own
/// regime fit. Returns the `Fit` (`Allowed` or `Penalised`) so the caller can carry its fact.
///
/// # Errors
/// The first failed guard.
pub fn entry_guards(
    meta: &StrategyMeta,
    view: &DataView<'_>,
    ctx: &StrategyContext<'_>,
) -> Result<Fit, Reject> {
    ctx.check_closed_bar(view)?;
    if view.len() < meta.min_history_bars {
        return Err(Reject::MinHistory {
            have: view.len(),
            need: meta.min_history_bars,
        });
    }
    if !view
        .bars()
        .iter()
        .all(bazaar_core::candle::Candle::adjusted)
    {
        return Err(Reject::Unadjusted);
    }
    if ctx.portfolio.holds(ctx.instrument.instrument_token) {
        return Err(Reject::Held);
    }
    let m = ctx.market_regime;
    if !matches!(m.trend, Trend::Up | Trend::Range) || m.breakdown != Tri::False {
        return Err(Reject::MarketGate(label(m)));
    }
    match meta.regime_fit.allows(ctx.market_regime, ctx.stock_regime) {
        Fit::Suppressed(f) => Err(Reject::RegimeFit(f.meaning)),
        fit => Ok(fit),
    }
}

fn tick_of(ctx: &StrategyContext<'_>) -> Result<Decimal, Reject> {
    let t = ctx.instrument.tick_size;
    (t > Decimal::ZERO).then_some(t).ok_or(Reject::BadTick)
}

#[derive(Clone, Copy)]
enum Mode {
    Floor,
    Ceil,
    Nearest,
}

fn snap(raw: Decimal, tick: Decimal, mode: Mode) -> Result<Decimal, Reject> {
    let steps = raw.checked_div(tick).ok_or(Reject::BadTick)?;
    let n = match mode {
        Mode::Floor => steps.floor(),
        Mode::Ceil => steps.ceil(),
        Mode::Nearest => steps.round(),
    };
    n.checked_mul(tick).ok_or(Reject::BadTick)
}

/// A long's levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels {
    pub entry_low: Decimal,
    pub entry_high: Decimal,
    pub stop: Decimal,
    pub target: Decimal,
}

/// Round raw levels to the instrument tick. The stop goes DOWN and the top of the entry zone UP:
/// rounding never moves toward a smaller risk. Entry low and target go to the nearest tick.
/// Every output is a multiple of the tick.
///
/// # Errors
/// `BadTick` for a non-positive tick or overflow.
pub fn round_levels(ctx: &StrategyContext<'_>, raw: &Levels) -> Result<Levels, Reject> {
    let t = tick_of(ctx)?;
    Ok(Levels {
        entry_low: snap(raw.entry_low, t, Mode::Nearest)?,
        entry_high: snap(raw.entry_high, t, Mode::Ceil)?,
        stop: snap(raw.stop, t, Mode::Floor)?,
        target: snap(raw.target, t, Mode::Nearest)?,
    })
}

/// True when `price` is a whole multiple of the instrument tick (carry-over from the slice 0
/// review: signal prices are checked against the tick, not trusted).
///
/// # Errors
/// `BadTick`.
pub fn on_tick(ctx: &StrategyContext<'_>, price: Decimal) -> Result<bool, Reject> {
    let t = tick_of(ctx)?;
    Ok(price.checked_rem(t) == Some(Decimal::ZERO))
}

/// Entry zone `[close, close + frac x ATR]` (unrounded; pass it through [`round_levels`]).
///
/// # Errors
/// `NotFinite`.
pub fn entry_zone(close: Decimal, atr: f64, frac: f64) -> Result<(Decimal, Decimal), Reject> {
    let width = to_decimal(atr)?
        .checked_mul(to_decimal(frac)?)
        .ok_or(Reject::NotFinite)?;
    Ok((close, close.saturating_add(width)))
}

/// Close of the next trading session from the calendar: the signal's `valid_until`.
///
/// # Errors
/// `ClosedBar(..)` wrapping the calendar failure.
pub fn valid_until(
    view: &DataView<'_>,
    ctx: &StrategyContext<'_>,
    kind: InstrumentKind,
) -> Result<DateTime<Tz>, Reject> {
    Ok(ctx.next_session_close(view, kind)?)
}

/// Stop guards: the stop must be below the entry zone and `entry_high - stop` must not exceed
/// `max_stop_atr x atr`. Returns the risk per share.
///
/// # Errors
/// `WrongSideStop`, `StopTooWide`, `NotFinite`.
pub fn check_stop(l: &Levels, atr: f64, max_stop_atr: f64) -> Result<Decimal, Reject> {
    if l.stop >= l.entry_low {
        return Err(Reject::WrongSideStop);
    }
    let risk = l.entry_high - l.stop;
    let cap = to_decimal(atr)?
        .checked_mul(to_decimal(max_stop_atr)?)
        .ok_or(Reject::NotFinite)?;
    if risk > cap {
        return Err(Reject::StopTooWide {
            distance: risk,
            cap,
        });
    }
    Ok(risk)
}

/// `(target - entry_low) / (entry_high - stop)` to two places (the `Signal` tolerance), rejected
/// when below `min_rr`.
///
/// # Errors
/// `WrongSideStop` for a non-positive risk, `RrTooLow`, `NotFinite`.
pub fn check_rr(l: &Levels, min_rr: f64) -> Result<Decimal, Reject> {
    let risk = l.entry_high - l.stop;
    if risk <= Decimal::ZERO {
        return Err(Reject::WrongSideStop);
    }
    let rr = ((l.target - l.entry_low) / risk).round_dp(2);
    let floor = to_decimal(min_rr)?;
    if rr < floor {
        return Err(Reject::RrTooLow { rr, min_rr: floor });
    }
    Ok(rr)
}

/// Pieces of a why-text in the fixed order of spec section 2: risk in rupees first, the setup
/// sentence(s), the regime label, the "proven wrong if" line, then a closing line (exit plan,
/// target, caution).
#[derive(Debug, Clone, Copy)]
pub struct Why<'a> {
    pub risk_rupees: Decimal,
    pub setup: &'a str,
    /// Plain-English regime label (`bazaar_regime::label`).
    pub regime_label: &'a str,
    pub proven_wrong_below: Decimal,
    pub closing: &'a str,
}

/// Build the why-text.
///
/// # Errors
/// `ForbiddenWord` when any part contains the word "sure", "safe" or "guaranteed".
pub fn why_text(w: &Why<'_>) -> Result<String, Reject> {
    let text = format!(
        "Risk first: you could lose about Rs {}. {} Market today: {}. Proven wrong if: price closes below {}. {}",
        w.risk_rupees.round_dp(2).normalize(),
        w.setup,
        w.regime_label,
        w.proven_wrong_below.normalize(),
        w.closing
    );
    let lower = text.to_lowercase();
    for word in FORBIDDEN {
        if lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|t| t == word)
        {
            return Err(Reject::ForbiddenWord(word));
        }
    }
    Ok(text.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::PortfolioView;
    use crate::exit::OpenPosition;
    use crate::testkit::{Row, World, bars, meta, regime, session_time};
    use bazaar_core::candle::Candle;
    use bazaar_regime::{Trend as T, VolState as V};

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn ok_world() -> World {
        World::new(T::Up, V::Normal, T::Up, V::Normal)
    }

    fn rows(n: usize) -> Vec<Row> {
        vec![(100, 101, 99, 100, 1000); n]
    }

    fn run(w: &World, b: &[Candle], min: usize) -> Result<Fit, Reject> {
        let view = DataView::ending_at(b, b.len() - 1).unwrap();
        entry_guards(&meta(min), &view, &w.ctx())
    }

    #[test]
    fn passes_when_everything_fits() {
        let w = ok_world();
        let b = bars(&rows(5), true);
        assert!(matches!(run(&w, &b, 5), Ok(Fit::Allowed(_))));
    }

    #[test]
    fn short_history_unadjusted_and_held_are_rejected() {
        let w = ok_world();
        assert_eq!(
            run(&ok_world().at(3), &bars(&rows(4), true), 5),
            Err(Reject::MinHistory { have: 4, need: 5 })
        );
        assert_eq!(run(&w, &bars(&rows(5), false), 5), Err(Reject::Unadjusted));
        let mut held = ok_world();
        held.portfolio = PortfolioView {
            positions: vec![OpenPosition {
                instrument_token: held.instrument.instrument_token,
                entry_price: d("100"),
                stop: d("90"),
                opened_at: session_time(0),
                bars_held: 1,
            }],
        };
        assert_eq!(run(&held, &bars(&rows(5), true), 5), Err(Reject::Held));
    }

    #[test]
    fn stale_regime_fails_the_closed_bar_check() {
        let mut w = ok_world();
        let b = bars(&rows(5), true);
        w.stock = regime(T::Up, V::Normal, Tri::Unknown, session_time(0));
        assert!(matches!(run(&w, &b, 5), Err(Reject::ClosedBar(_))));
    }

    #[test]
    fn market_gate_needs_up_or_range_and_no_breakdown() {
        let b = bars(&rows(5), true);
        let range = World::new(T::Range, V::Normal, T::Up, V::Normal);
        assert!(run(&range, &b, 5).is_ok());
        for t in [T::Down, T::Mixed, T::Unknown] {
            let w = World::new(t, V::Normal, T::Up, V::Normal);
            assert!(
                matches!(run(&w, &b, 5), Err(Reject::MarketGate(_))),
                "{t:?}"
            );
        }
        let mut w = ok_world();
        w.market.breakdown = Tri::True;
        assert!(matches!(run(&w, &b, 5), Err(Reject::MarketGate(_))));
        w.market.breakdown = Tri::Unknown;
        assert!(matches!(run(&w, &b, 5), Err(Reject::MarketGate(_))));
    }

    #[test]
    fn regime_fit_suppresses_and_penalises_per_declared_cells() {
        let b = bars(&rows(5), true);
        let down = World::new(T::Up, V::Normal, T::Down, V::Normal);
        assert!(matches!(run(&down, &b, 5), Err(Reject::RegimeFit(_))));
        let mixed = World::new(T::Up, V::Normal, T::Mixed, V::Normal);
        assert!(matches!(run(&mixed, &b, 5), Err(Reject::RegimeFit(_))));
        let wild = World::new(T::Up, V::Normal, T::Up, V::High);
        assert!(matches!(run(&wild, &b, 5), Ok(Fit::Penalised(_))));
    }

    fn lv(el: &str, eh: &str, s: &str, t: &str) -> Levels {
        Levels {
            entry_low: d(el),
            entry_high: d(eh),
            stop: d(s),
            target: d(t),
        }
    }

    #[test]
    fn rounding_moves_stop_down_zone_top_up_and_others_to_nearest() {
        let w = ok_world(); // tick 0.05
        let r = round_levels(&w.ctx(), &lv("100.02", "100.51", "94.97", "116.03")).unwrap();
        assert_eq!(
            (r.entry_low, r.entry_high, r.stop, r.target),
            (d("100.00"), d("100.55"), d("94.95"), d("116.05"))
        );
        // Already on tick: unchanged.
        let on = lv("100.05", "100.50", "94.95", "116.00");
        assert_eq!(round_levels(&w.ctx(), &on).unwrap(), on);
        assert!(on_tick(&w.ctx(), d("94.95")).unwrap());
        assert!(!on_tick(&w.ctx(), d("94.97")).unwrap());
        // Risk after rounding is never smaller than the raw risk.
        let raw = lv("100.02", "100.51", "94.97", "116.03");
        assert!(r.entry_high - r.stop >= raw.entry_high - raw.stop);
    }

    #[test]
    fn bad_tick_is_an_error_not_a_panic() {
        let mut w = ok_world();
        w.instrument.tick_size = Decimal::ZERO;
        assert_eq!(
            round_levels(&w.ctx(), &lv("1", "1", "1", "1")),
            Err(Reject::BadTick)
        );
        assert_eq!(on_tick(&w.ctx(), d("1")), Err(Reject::BadTick));
    }

    #[test]
    fn entry_zone_is_close_to_close_plus_fraction_of_atr() {
        assert_eq!(
            entry_zone(d("100"), 8.0, 0.25).unwrap(),
            (d("100"), d("102"))
        );
        assert_eq!(entry_zone(d("100"), f64::NAN, 0.25), Err(Reject::NotFinite));
        assert_eq!(need("atr", None), Err(Reject::Indicator("atr")));
        assert_eq!(
            need("atr", Some(f64::INFINITY)),
            Err(Reject::Indicator("atr"))
        );
        assert_eq!(need("atr", Some(2.0)), Ok(2.0));
    }

    #[test]
    fn stop_guards_wrong_side_and_cap() {
        let l = lv("100", "102", "94", "116");
        assert_eq!(check_stop(&l, 4.0, 2.0), Ok(d("8")));
        assert!(matches!(
            check_stop(&l, 4.0, 1.5),
            Err(Reject::StopTooWide { .. })
        ));
        for stop in ["100", "101"] {
            assert_eq!(
                check_stop(&lv("100", "102", stop, "116"), 4.0, 2.0),
                Err(Reject::WrongSideStop)
            );
        }
    }

    #[test]
    fn reward_risk_is_computed_and_floored() {
        let l = lv("100", "102", "94", "116"); // (116-100)/8 = 2
        assert_eq!(check_rr(&l, 1.5), Ok(d("2")));
        assert_eq!(check_rr(&l, 2.0), Ok(d("2")));
        assert!(matches!(check_rr(&l, 2.1), Err(Reject::RrTooLow { .. })));
        assert_eq!(
            check_rr(&lv("100", "102", "102", "116"), 1.0),
            Err(Reject::WrongSideStop)
        );
    }

    #[test]
    fn valid_until_is_the_next_session_close_from_the_calendar() {
        let w = ok_world();
        let b = bars(&rows(3), true);
        let view = DataView::ending_at(&b, 1).unwrap();
        let got = valid_until(&view, &w.ctx(), InstrumentKind::CashNonFno).unwrap();
        assert_eq!(got, b[2].close_time());
    }

    #[test]
    fn why_text_leads_with_risk_and_names_regime_and_stop() {
        let t = why_text(&Why {
            risk_rupees: d("1000.00"),
            setup: "ABC closed above its 20-day high.",
            regime_label: "Uptrend, normal swings",
            proven_wrong_below: d("94.50"),
            closing: "We leave after 60 days.",
        })
        .unwrap();
        assert_eq!(
            t,
            "Risk first: you could lose about Rs 1000. ABC closed above its 20-day high. \
             Market today: Uptrend, normal swings. Proven wrong if: price closes below 94.5. \
             We leave after 60 days."
        );
    }

    #[test]
    fn why_text_refuses_certainty_words_but_not_substrings() {
        let mk = |setup: &'static str| Why {
            risk_rupees: d("1"),
            setup,
            regime_label: "x",
            proven_wrong_below: d("1"),
            closing: "",
        };
        for w in ["A sure thing.", "It is SAFE.", "Guaranteed gains."] {
            assert!(
                matches!(why_text(&mk(w)), Err(Reject::ForbiddenWord(_))),
                "{w}"
            );
        }
        assert!(why_text(&mk("Unsurely worded, a safeguard.")).is_ok());
    }

    #[test]
    fn to_decimal_is_the_only_boundary() {
        assert_eq!(to_decimal(0.25), Ok(d("0.25")));
        assert_eq!(to_decimal(f64::NEG_INFINITY), Err(Reject::NotFinite));
    }
}
