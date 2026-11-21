//! Pure, seeded fill simulation. Same ticks + same seed + same book gives the same fills.
//! No clock reads (callers pass `now`), no I/O, no `f64`. Every number comes from
//! [`SimConfig`] (built from `[execution]`) or the instrument spec; none is a literal rule value.
//!
//! Rules: a market order fills on the first tick strictly after acceptance, slipped against the
//! trader and aligned to the tick size; a limit order fills only when the tick traded strictly
//! through the limit, at the limit; no fill on stale data, outside the normal session, or when a
//! buy sits at the upper circuit or a sell at the lower circuit; each fill is capped by
//! `max_participation` of the tick volume (seeded 50-100% of the cap, at least 1).
//! A tick for a different instant than `now` allows is stale: age beyond `stale_after_secs`, or
//! a tick from the future, is refused (fail closed).

use std::collections::HashMap;

use bazaar_core::calendar::{MarketCalendar, Session};
use bazaar_core::config::ExecutionSection;
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_execution::types::{IntentId, OrderKind, Product, Side};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

/// Marker bit for ids of simulated square-off orders (never collides with a real intent id).
pub const SYNTH_BASE: u128 = 1 << 127;

/// Instrument facts the simulator and executor need; from the instrument master, never literals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentSpec {
    pub segment: Segment,
    pub kind: InstrumentKind,
    pub lot_size: u32,
    pub tick_size: Decimal,
    /// Exchange freeze quantity; `None` when the instrument has none.
    pub freeze_qty: Option<u32>,
}

/// Simulator settings, all from `[execution]`.
#[derive(Debug, Clone)]
pub struct SimConfig {
    pub slippage_equity_bps: u32,
    pub slippage_equity_derivatives_bps: u32,
    pub slippage_currency_bps: u32,
    pub slippage_commodity_bps: u32,
    pub max_participation: Decimal,
    pub stale_after_secs: u32,
    pub seed: u64,
}

impl SimConfig {
    /// Settings from the validated `[execution]` section.
    #[must_use]
    pub fn from_section(e: &ExecutionSection, seed: u64) -> Self {
        Self {
            slippage_equity_bps: e.paper_slippage_bps.equity,
            slippage_equity_derivatives_bps: e.paper_slippage_bps.equity_derivatives,
            slippage_currency_bps: e.paper_slippage_bps.currency,
            slippage_commodity_bps: e.paper_slippage_bps.commodity,
            max_participation: e.paper_max_participation,
            stale_after_secs: e.paper_stale_after_secs,
            seed,
        }
    }

    const fn slippage(&self, seg: Segment) -> u32 {
        match seg {
            Segment::Equity => self.slippage_equity_bps,
            Segment::EquityDerivatives => self.slippage_equity_derivatives_bps,
            Segment::Currency => self.slippage_currency_bps,
            Segment::Commodity => self.slippage_commodity_bps,
        }
    }
}

/// One market tick. Whether the session is open is decided by the simulator from the calendar
/// and each order's instrument; the caller cannot assert it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    pub symbol: String,
    pub price: Decimal,
    pub volume: u64,
    pub at: DateTime<Tz>,
    pub lower_circuit: Option<Decimal>,
    pub upper_circuit: Option<Decimal>,
}

/// True only in the normal session. A calendar error (date outside the covered years) is closed.
#[must_use]
pub fn session_open(
    cal: &MarketCalendar,
    at: DateTime<Tz>,
    seg: Segment,
    kind: InstrumentKind,
) -> bool {
    matches!(
        cal.session_at(at.with_timezone(&Utc), seg, kind),
        Ok(Session::Normal)
    )
}

/// An accepted order waiting for ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingOrder {
    pub id: IntentId,
    pub symbol: String,
    pub side: Side,
    pub qty: u32,
    pub filled: u32,
    pub kind: OrderKind,
    pub product: Product,
    pub spec: InstrumentSpec,
    pub accepted_at: DateTime<Tz>,
    /// Set when a fill was refused because the data was stale.
    pub stale_flagged: bool,
    /// Simulated broker square-off, not a user order.
    pub square_off: bool,
}

/// Why a fill happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillReason {
    Market,
    Limit,
    SquareOff,
}

/// One simulated fill (possibly partial).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimFill {
    pub id: IntentId,
    pub symbol: String,
    pub side: Side,
    pub qty: u32,
    pub price: Decimal,
    pub product: Product,
    pub at: DateTime<Tz>,
    /// True when this fill completes the order.
    pub complete: bool,
    pub reason: FillReason,
}

/// Working orders, the net position per (symbol, product) and the last price seen per symbol.
#[derive(Debug, Default)]
pub struct WorkingBook {
    pub orders: Vec<WorkingOrder>,
    positions: HashMap<(String, &'static str), i64>,
    pub last_price: HashMap<String, Decimal>,
    /// Spec of every symbol an order was accepted for (square-off orders reuse it).
    pub specs: HashMap<String, InstrumentSpec>,
    next_synth: u128,
}

impl WorkingBook {
    /// Signed net quantity (long positive) for a symbol and product.
    #[must_use]
    pub fn position(&self, symbol: &str, product: Product) -> i64 {
        self.positions
            .get(&(symbol.to_owned(), product.as_str()))
            .copied()
            .unwrap_or(0)
    }

    /// Unfilled quantity of working sells for a symbol (reserved against the position).
    #[must_use]
    pub fn working_sell_qty(&self, symbol: &str) -> u64 {
        self.orders
            .iter()
            .filter(|o| o.symbol == symbol && o.side == Side::Sell)
            .map(|o| u64::from(o.qty - o.filled))
            .sum()
    }

    /// Non-flat positions of one product, sorted by symbol (deterministic).
    #[must_use]
    pub fn open_positions(&self, product: Product) -> Vec<(String, i64)> {
        let mut v: Vec<(String, i64)> = self
            .positions
            .iter()
            .filter(|((_, p), q)| *p == product.as_str() && **q != 0)
            .map(|((s, _), q)| (s.clone(), *q))
            .collect();
        v.sort();
        v
    }

    /// Next id for a simulated order.
    pub fn synth_id(&mut self) -> IntentId {
        self.next_synth += 1;
        IntentId(SYNTH_BASE | self.next_synth)
    }

    /// Book a fill that the account accepted: move the position, retire a complete order.
    pub fn record(&mut self, f: &SimFill) {
        let signed = i64::from(f.qty);
        let delta = if f.side == Side::Buy { signed } else { -signed };
        *self
            .positions
            .entry((f.symbol.clone(), f.product.as_str()))
            .or_insert(0) += delta;
        if f.complete {
            self.orders.retain(|o| o.id != f.id);
        }
    }

    /// Remove an order without filling it.
    pub fn drop_order(&mut self, id: IntentId) {
        self.orders.retain(|o| o.id != id);
    }
}

/// Deterministic generator (splitmix64); the only randomness in the simulator.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The pure simulator.
#[derive(Debug, Clone)]
pub struct FillSimulator {
    cfg: SimConfig,
    rng: Rng,
}

const BPS_DENOM: i64 = 10_000;

impl FillSimulator {
    #[must_use]
    pub fn new(cfg: SimConfig) -> Self {
        let rng = Rng(cfg.seed);
        Self { cfg, rng }
    }

    fn is_stale(&self, tick: &Tick, now: DateTime<Tz>) -> bool {
        let age = now.signed_duration_since(tick.at).num_seconds();
        age < 0 || age > i64::from(self.cfg.stale_after_secs)
    }

    /// Slipped price against the trader, tick-aligned against the trader, kept inside the circuits.
    fn slipped(&self, o: &WorkingOrder, tick: &Tick) -> Option<Decimal> {
        let f = Decimal::from(self.cfg.slippage(o.spec.segment)) / Decimal::from(BPS_DENOM);
        let buy = o.side == Side::Buy;
        let raw = if buy {
            tick.price.checked_mul(Decimal::ONE + f)?
        } else {
            tick.price.checked_mul(Decimal::ONE - f)?
        };
        let steps = raw.checked_div(o.spec.tick_size)?;
        let steps = if buy { steps.ceil() } else { steps.floor() };
        let mut p = steps.checked_mul(o.spec.tick_size)?;
        // A price beyond a circuit is pulled back to the nearest valid tick inside it (the
        // circuit is the worst price the trader can get; it is never better than the slipped one).
        if let Some(u) = tick.upper_circuit.filter(|u| p > *u) {
            p = u.checked_div(o.spec.tick_size)?.floor() * o.spec.tick_size;
        }
        if let Some(l) = tick.lower_circuit.filter(|l| p < *l) {
            p = l.checked_div(o.spec.tick_size)?.ceil() * o.spec.tick_size;
        }
        (p > Decimal::ZERO).then_some(p)
    }

    fn take(&mut self, remaining: u32, tick: &Tick) -> u32 {
        let cap = Decimal::from(tick.volume)
            .checked_mul(self.cfg.max_participation)
            .map_or(Decimal::ZERO, |c| c.trunc());
        let cap = cap.to_u32().unwrap_or(u32::MAX);
        if cap == 0 {
            return 0;
        }
        let pct = 50 + u32::try_from(self.rng.next() % 51).unwrap_or(0);
        let part = (u64::from(cap) * u64::from(pct) / 100).max(1);
        u32::try_from(part).unwrap_or(u32::MAX).min(remaining)
    }

    /// Fills for one tick. `now` is the injected clock reading.
    pub fn on_tick(
        &mut self,
        tick: &Tick,
        now: DateTime<Tz>,
        cal: &MarketCalendar,
        book: &mut WorkingBook,
    ) -> Vec<SimFill> {
        let mut out = Vec::new();
        book.last_price.insert(tick.symbol.clone(), tick.price);
        let stale = self.is_stale(tick, now);
        let mut orders = std::mem::take(&mut book.orders);
        for o in &mut orders {
            if o.symbol != tick.symbol || tick.at <= o.accepted_at {
                continue;
            }
            if stale {
                o.stale_flagged = true;
                continue;
            }
            if !session_open(cal, tick.at, o.spec.segment, o.spec.kind)
                || Self::at_circuit(o.side, tick)
            {
                continue;
            }
            let (price, reason) = match o.kind {
                OrderKind::Market if o.square_off => (self.slipped(o, tick), FillReason::SquareOff),
                OrderKind::Market => (self.slipped(o, tick), FillReason::Market),
                OrderKind::Limit(l) => {
                    let through = if o.side == Side::Buy {
                        tick.price < l
                    } else {
                        tick.price > l
                    };
                    (through.then_some(l), FillReason::Limit)
                }
            };
            let Some(price) = price else { continue };
            let qty = self.take(o.qty - o.filled, tick);
            if qty == 0 {
                continue;
            }
            o.filled += qty;
            out.push(SimFill {
                id: o.id,
                symbol: o.symbol.clone(),
                side: o.side,
                qty,
                price,
                product: o.product,
                at: tick.at,
                complete: o.filled >= o.qty,
                reason,
            });
        }
        book.orders = orders;
        out
    }

    fn at_circuit(side: Side, t: &Tick) -> bool {
        match side {
            Side::Buy => t.upper_circuit.is_some_and(|u| t.price >= u),
            Side::Sell => t.lower_circuit.is_some_and(|l| t.price <= l),
        }
    }

    /// DAY validity: drop working orders accepted on an earlier IST date. Returns their ids.
    pub fn on_clock(&mut self, now: DateTime<Tz>, book: &mut WorkingBook) -> Vec<IntentId> {
        let today = now.date_naive();
        let gone: Vec<IntentId> = book
            .orders
            .iter()
            .filter(|o| o.accepted_at.date_naive() < today)
            .map(|o| o.id)
            .collect();
        book.orders.retain(|o| !gone.contains(&o.id));
        gone
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    pub(crate) fn cal() -> MarketCalendar {
        let cfg: bazaar_core::config::CalendarConfig =
            bazaar_core::config::load(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/config"
            )))
            .unwrap();
        MarketCalendar::from_config(&cfg)
    }

    pub(crate) fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    pub(crate) fn at(h: u32, m: u32, s: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 1, 5, h, m, s).unwrap()
    }

    pub(crate) fn cfg() -> SimConfig {
        SimConfig {
            slippage_equity_bps: 10,
            slippage_equity_derivatives_bps: 20,
            slippage_currency_bps: 2,
            slippage_commodity_bps: 5,
            max_participation: d("0.1"),
            stale_after_secs: 5,
            seed: 42,
        }
    }

    pub(crate) fn spec() -> InstrumentSpec {
        InstrumentSpec {
            segment: Segment::Equity,
            kind: InstrumentKind::CashNonFno,
            lot_size: 1,
            tick_size: d("0.05"),
            freeze_qty: None,
        }
    }

    pub(crate) fn order(id: u128, side: Side, qty: u32, kind: OrderKind) -> WorkingOrder {
        WorkingOrder {
            id: IntentId(id),
            symbol: "ABC".into(),
            side,
            qty,
            filled: 0,
            kind,
            product: Product::Cnc,
            spec: spec(),
            accepted_at: at(10, 0, 0),
            stale_flagged: false,
            square_off: false,
        }
    }

    pub(crate) fn tick(price: &str, volume: u64, s: u32) -> Tick {
        Tick {
            symbol: "ABC".into(),
            price: d(price),
            volume,
            at: at(10, 0, s),
            lower_circuit: Some(d("90")),
            upper_circuit: Some(d("110")),
        }
    }

    fn run(book: &mut WorkingBook, t: &Tick) -> Vec<SimFill> {
        FillSimulator::new(cfg()).on_tick(t, t.at, &cal(), book)
    }

    fn book_with(o: WorkingOrder) -> WorkingBook {
        WorkingBook {
            orders: vec![o],
            ..WorkingBook::default()
        }
    }

    #[test]
    fn market_fills_on_next_tick_not_the_acceptance_instant() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        assert_eq!(run(&mut b, &tick("100", 1000, 0)).len(), 0);
        let f = run(&mut b, &tick("100", 1000, 1));
        assert_eq!((f.len(), f[0].qty), (1, 5));
    }

    #[test]
    fn slippage_is_against_the_trader_and_tick_aligned() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        let f = run(&mut b, &tick("100.02", 1000, 1));
        // 100.02 * 1.001 = 100.12002 -> ceil to 0.05 = 100.15
        assert_eq!(f[0].price, d("100.15"));
        let mut b = book_with(order(2, Side::Sell, 5, OrderKind::Market));
        let f = run(&mut b, &tick("100.02", 1000, 1));
        // 100.02 * 0.999 = 99.91998 -> floor = 99.90
        assert_eq!(f[0].price, d("99.90"));
    }

    #[test]
    fn limit_needs_trade_through_and_fills_at_the_limit() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Limit(d("100"))));
        assert!(run(&mut b, &tick("100", 1000, 1)).is_empty(), "touch");
        let f = run(&mut b, &tick("99.95", 1000, 2));
        assert_eq!(f[0].price, d("100"));
        let mut b = book_with(order(2, Side::Sell, 5, OrderKind::Limit(d("100"))));
        assert_eq!(run(&mut b, &tick("100", 1000, 1)).len(), 0);
        assert_eq!(run(&mut b, &tick("100.05", 1000, 2))[0].price, d("100"));
    }

    #[test]
    fn no_fill_at_circuit_for_the_blocked_side_only() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        assert_eq!(run(&mut b, &tick("110", 1000, 1)).len(), 0);
        assert_eq!(b.orders.len(), 1, "stays working");
        let mut b = book_with(order(2, Side::Sell, 5, OrderKind::Market));
        assert_eq!(run(&mut b, &tick("90", 1000, 1)).len(), 0);
        let mut b = book_with(order(3, Side::Sell, 5, OrderKind::Market));
        assert_eq!(run(&mut b, &tick("110", 1000, 1)).len(), 1);
    }

    #[test]
    fn slippage_never_crosses_the_circuit() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        let f = run(&mut b, &tick("109.95", 1000, 1));
        assert_eq!(f[0].price, d("110"));
    }

    #[test]
    fn stale_or_future_data_does_not_fill_and_flags_the_order() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        let t = tick("100", 1000, 1);
        let late = t.at + chrono::Duration::seconds(6);
        assert_eq!(
            FillSimulator::new(cfg())
                .on_tick(&t, late, &cal(), &mut b)
                .len(),
            0
        );
        assert!(b.orders[0].stale_flagged);
        let early = t.at - chrono::Duration::seconds(1);
        assert_eq!(
            FillSimulator::new(cfg())
                .on_tick(&t, early, &cal(), &mut b)
                .len(),
            0
        );
        // exactly at the bound still fills
        let edge = t.at + chrono::Duration::seconds(5);
        assert_eq!(
            FillSimulator::new(cfg())
                .on_tick(&t, edge, &cal(), &mut b)
                .len(),
            1
        );
    }

    #[test]
    fn no_fill_outside_the_session() {
        let mut o = order(1, Side::Buy, 5, OrderKind::Market);
        o.accepted_at = at(8, 0, 0);
        let mut b = book_with(o);
        let mut t = tick("100", 1000, 1);
        t.at = at(9, 0, 1); // before the open
        assert_eq!(run(&mut b, &t).len(), 0);
        t.at = at(20, 0, 0); // after the close
        assert_eq!(run(&mut b, &t).len(), 0);
        t.at = at(10, 0, 1);
        assert_eq!(run(&mut b, &t).len(), 1, "same order fills in session");
    }

    #[test]
    fn date_outside_the_calendar_never_fills() {
        let far = |h| Kolkata.with_ymd_and_hms(2030, 1, 7, h, 0, 0).unwrap();
        let mut o = order(1, Side::Buy, 5, OrderKind::Market);
        o.accepted_at = far(10);
        let mut b = book_with(o);
        let mut t = tick("100", 1000, 0);
        t.at = far(11);
        assert_eq!(run(&mut b, &t).len(), 0);
    }

    #[test]
    fn participation_cap_is_exact_per_tick() {
        // volume 100 x 0.1 = 10: every fill is 5..=10 (the Decimal scale must not inflate it)
        let mut b = book_with(order(1, Side::Buy, 1000, OrderKind::Market));
        let mut sim = FillSimulator::new(cfg());
        for s in 1..30 {
            let t = tick("100", 100, s);
            for f in sim.on_tick(&t, t.at, &cal(), &mut b) {
                assert!((5..=10).contains(&f.qty), "qty {}", f.qty);
            }
        }
    }

    #[test]
    fn off_tick_circuit_is_pulled_back_inside_to_a_valid_tick() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        let mut t = tick("100", 1000, 1);
        t.upper_circuit = Some(d("100.12")); // not a 0.05 multiple; slipped 100.10 is inside
        assert_eq!(run(&mut b, &t)[0].price, d("100.10"));
        let mut b = book_with(order(2, Side::Buy, 5, OrderKind::Market));
        t.price = d("100.08"); // slipped to 100.18, beyond the circuit
        assert_eq!(run(&mut b, &t)[0].price, d("100.10"));
    }

    #[test]
    fn sim_config_from_the_real_config_maps_every_segment() {
        let r: bazaar_core::config::RiskConfig = bazaar_core::config::load(std::path::Path::new(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/config"),
        ))
        .unwrap();
        let c = SimConfig::from_section(&r.execution, 7);
        assert_eq!(
            (
                c.slippage_equity_bps,
                c.slippage_equity_derivatives_bps,
                c.slippage_currency_bps,
                c.slippage_commodity_bps
            ),
            (5, 5, 2, 5)
        );
        assert_eq!(
            (c.max_participation, c.stale_after_secs, c.seed),
            (d("0.1"), 5, 7)
        );
    }

    #[test]
    fn thin_volume_gives_partial_fills_summing_to_qty_within_cap() {
        let mut b = book_with(order(1, Side::Buy, 10, OrderKind::Market));
        let mut sim = FillSimulator::new(cfg());
        let mut total = 0;
        let mut parts = 0;
        for s in 1..60 {
            let t = tick("100", 40, s); // cap = 4
            for f in sim.on_tick(&t, t.at, &cal(), &mut b) {
                assert!(f.qty <= 4 && f.qty >= 1);
                total += f.qty;
                parts += 1;
                b.record(&f);
            }
        }
        assert_eq!(total, 10);
        assert!(parts >= 3);
        assert_eq!(b.orders.len(), 0);
    }

    #[test]
    fn zero_volume_never_fills() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        assert_eq!(run(&mut b, &tick("100", 0, 1)).len(), 0);
        assert!(
            run(&mut b, &tick("100", 9, 2)).is_empty(),
            "cap floors to 0"
        );
    }

    #[test]
    fn same_seed_same_fills_and_other_seed_may_differ() {
        let go = |seed| {
            let mut c = cfg();
            c.seed = seed;
            let mut b = book_with(order(1, Side::Buy, 100, OrderKind::Market));
            let mut sim = FillSimulator::new(c);
            (1..30)
                .flat_map(|s| {
                    let t = tick("100", 400, s);
                    sim.on_tick(&t, t.at, &cal(), &mut b)
                })
                .map(|f| f.qty)
                .collect::<Vec<_>>()
        };
        assert_eq!(go(7), go(7));
        assert_ne!(go(7), go(8));
    }

    #[test]
    fn day_orders_expire_on_a_later_date() {
        let mut b = book_with(order(1, Side::Buy, 5, OrderKind::Market));
        let mut sim = FillSimulator::new(cfg());
        assert_eq!(sim.on_clock(at(15, 0, 0), &mut b).len(), 0);
        let next = Kolkata.with_ymd_and_hms(2026, 1, 6, 9, 0, 0).unwrap();
        assert_eq!(sim.on_clock(next, &mut b), vec![IntentId(1)]);
        assert_eq!(b.orders.len(), 0);
    }

    #[test]
    fn book_tracks_positions_and_reserved_sells() {
        let mut b = book_with(order(1, Side::Sell, 5, OrderKind::Market));
        assert_eq!(b.working_sell_qty("ABC"), 5);
        let f = SimFill {
            id: IntentId(9),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 3,
            price: d("1"),
            product: Product::Cnc,
            at: at(10, 0, 1),
            complete: true,
            reason: FillReason::Market,
        };
        b.record(&f);
        assert_eq!(b.position("ABC", Product::Cnc), 3);
        assert_eq!(b.position("ABC", Product::Mis), 0);
        assert!(b.synth_id().0 & SYNTH_BASE != 0);
    }

    #[test]
    fn closed_calendar_is_no_session() {
        let cfg: bazaar_core::config::CalendarConfig =
            bazaar_core::config::load(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/config"
            )))
            .unwrap();
        let cal = MarketCalendar::from_config(&cfg);
        let (s, k) = (Segment::Equity, InstrumentKind::CashNonFno);
        assert!(session_open(&cal, at(11, 0, 0), s, k));
        assert!(!session_open(&cal, at(9, 0, 0), s, k));
        // 2030 is outside covered_years: fail closed.
        let far = Kolkata.with_ymd_and_hms(2030, 1, 7, 11, 0, 0).unwrap();
        assert!(!session_open(&cal, far, s, k));
    }
}
