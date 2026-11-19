//! Session VWAP, gap %, classic and Camarilla pivots.
//!
//! Session boundaries are not computed here (no calendar, no clock, no hardcoded open time): the
//! caller tags each bar with an opaque, non-decreasing `session` id derived from `MarketCalendar`
//! (for example the IST trading date). A change of id starts a new session; an id lower than the
//! current one is out of order and the bar is skipped (`None`, state untouched).

use crate::{Indicator, Ohlcv};

/// A bar tagged with its trading-session id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionBar {
    /// Opaque session id; equal within a session, increasing across sessions.
    pub session: i64,
    /// The closed bar.
    pub bar: Ohlcv,
}

/// True when every field of the bar is finite.
fn finite(b: &Ohlcv) -> bool {
    [b.o, b.h, b.l, b.c, b.v].iter().all(|v| v.is_finite())
}

/// Session VWAP: cumulative `sum(typical * volume) / sum(volume)` with typical `(H+L+C)/3`,
/// reset at each new session id. Warm-up 1 bar; `None` while the session's cumulative volume is
/// 0 (VWAP undefined), for a non-finite bar, an out-of-order session, or if the sums overflow.
#[derive(Debug, Clone, Default)]
pub struct SessionVwap {
    session: Option<i64>,
    pv: f64,
    vol: f64,
}

impl SessionVwap {
    /// New VWAP with no session yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            session: None,
            pv: 0.0,
            vol: 0.0,
        }
    }
}

impl Indicator for SessionVwap {
    type Input = SessionBar;
    type Output = f64;
    fn warmup(&self) -> usize {
        1
    }
    fn update(&mut self, x: SessionBar) -> Option<f64> {
        if !finite(&x.bar) || self.session.is_some_and(|s| x.session < s) {
            return None;
        }
        let (mut pv, mut vol) = (self.pv, self.vol);
        if self.session != Some(x.session) {
            (pv, vol) = (0.0, 0.0);
        }
        let b = x.bar;
        pv += (b.h + b.l + b.c) / 3.0 * b.v;
        vol += b.v;
        if !pv.is_finite() || !vol.is_finite() {
            return None;
        }
        self.session = Some(x.session);
        self.pv = pv;
        self.vol = vol;
        (vol > 0.0).then(|| pv / vol).filter(|v| v.is_finite())
    }
    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Gap %: `(first open of the session - last close of the previous session) / previous close * 100`.
/// Yielded on every bar of a session (it is known from the session's first bar on). `None` for
/// the first session seen, when the previous close is 0, for a non-finite bar or an out-of-order
/// session. Warm-up 1 bar.
#[derive(Debug, Clone, Default)]
pub struct GapPct {
    session: Option<i64>,
    last_close: f64,
    gap: Option<f64>,
}

impl GapPct {
    /// New gap indicator with no session yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            session: None,
            last_close: 0.0,
            gap: None,
        }
    }
}

impl Indicator for GapPct {
    type Input = SessionBar;
    type Output = f64;
    fn warmup(&self) -> usize {
        1
    }
    fn update(&mut self, x: SessionBar) -> Option<f64> {
        if !finite(&x.bar) || self.session.is_some_and(|s| x.session < s) {
            return None;
        }
        if self.session != Some(x.session) {
            self.gap = self.session.and_then(|_| {
                (self.last_close != 0.0)
                    .then(|| (x.bar.o - self.last_close) / self.last_close * 100.0)
                    .filter(|g| g.is_finite())
            });
            self.session = Some(x.session);
        }
        self.last_close = x.bar.c;
        self.gap
    }
    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Classic floor-trader pivot levels from the previous session's high, low and close.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassicPivots {
    /// Pivot `(H+L+C)/3`.
    pub p: f64,
    /// `2P - L`.
    pub r1: f64,
    /// `P + (H - L)`.
    pub r2: f64,
    /// `H + 2(P - L)`.
    pub r3: f64,
    /// `2P - H`.
    pub s1: f64,
    /// `P - (H - L)`.
    pub s2: f64,
    /// `L - 2(H - P)`.
    pub s3: f64,
}

/// Classic pivots for the next session from the previous session's `high`, `low`, `close`.
/// `None` if an input is not finite or a level overflows.
#[must_use]
pub fn classic_pivots(high: f64, low: f64, close: f64) -> Option<ClassicPivots> {
    let p = (high + low + close) / 3.0;
    let range = high - low;
    let out = ClassicPivots {
        p,
        r1: 2.0 * p - low,
        r2: p + range,
        r3: 2.0f64.mul_add(p - low, high),
        s1: 2.0 * p - high,
        s2: p - range,
        s3: 2.0f64.mul_add(-(high - p), low),
    };
    [out.p, out.r1, out.r2, out.r3, out.s1, out.s2, out.s3]
        .iter()
        .all(|v| v.is_finite())
        .then_some(out)
}

/// Camarilla pivot levels (`C +/- range * k` with k = 1.1/12, 1.1/6, 1.1/4, 1.1/2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CamarillaPivots {
    /// Resistance levels R1..R4.
    pub r: [f64; 4],
    /// Support levels S1..S4.
    pub s: [f64; 4],
}

/// Camarilla pivots for the next session from the previous `high`, `low`, `close`.
/// `None` if an input is not finite or a level overflows.
#[must_use]
pub fn camarilla_pivots(high: f64, low: f64, close: f64) -> Option<CamarillaPivots> {
    let range = high - low;
    let k = [1.1 / 12.0, 1.1 / 6.0, 1.1 / 4.0, 1.1 / 2.0];
    let out = CamarillaPivots {
        r: k.map(|f| range.mul_add(f, close)),
        s: k.map(|f| range.mul_add(-f, close)),
    };
    out.r
        .iter()
        .chain(out.s.iter())
        .all(|v| v.is_finite())
        .then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{batch, testkit};

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-8, "{a} vs {b}");
    }

    /// Fixture rows `session,open,high,low,close,volume`.
    fn micro() -> Vec<SessionBar> {
        include_str!("../../../fixtures/indicators/session_micro.csv")
            .lines()
            .skip(1)
            .enumerate()
            .map(|(i, line)| {
                let f: Vec<f64> = line.split(',').map(|s| s.parse().unwrap()).collect();
                SessionBar {
                    session: line.split(',').next().unwrap().parse().unwrap(),
                    bar: Ohlcv {
                        ts_ms: i64::try_from(i).unwrap() * 60_000,
                        o: f[1],
                        h: f[2],
                        l: f[3],
                        c: f[4],
                        v: f[5],
                    },
                }
            })
            .collect()
    }

    #[test]
    fn vwap_hand_case_resets_each_session() {
        // s1 bar0 typical (12+8+11)/3 = 31/3, vol 100 -> 31/3 = 10.3333.
        // bar1 typical 35/3, vol 300: (3100/3 + 10500/3)/400 = (13600/3)/400 = 34/3 = 11.3333.
        // s2 resets: bar2 typical 44/3 -> 14.6667; bar3 volume 0 leaves it at 14.6667.
        let out = batch(SessionVwap::new(), micro());
        near(out[0].unwrap(), 31.0 / 3.0);
        near(out[1].unwrap(), 34.0 / 3.0);
        near(out[2].unwrap(), 44.0 / 3.0);
        near(out[3].unwrap(), 44.0 / 3.0);
    }

    #[test]
    fn vwap_zero_volume_nonfinite_and_out_of_order() {
        let mut bars = micro();
        bars[0].bar.v = 0.0;
        assert_eq!(batch(SessionVwap::new(), bars)[0], None);
        let mut bad = micro();
        bad[1].bar.c = f64::NAN;
        assert_eq!(batch(SessionVwap::new(), bad)[1], None);
        // session id going backwards is skipped and does not disturb state
        let mut back = micro();
        back[3].session = 1;
        assert_eq!(batch(SessionVwap::new(), back)[3], None);
        let mut huge = micro();
        huge[0].bar.h = f64::MAX;
        huge[0].bar.v = f64::MAX;
        assert_eq!(batch(SessionVwap::new(), huge)[0], None);
    }

    fn mk(session: i64, o: f64, c: f64) -> SessionBar {
        SessionBar {
            session,
            bar: Ohlcv {
                ts_ms: 0,
                o,
                h: o.max(c),
                l: o.min(c),
                c,
                v: 1.0,
            },
        }
    }

    #[test]
    fn gap_hand_case() {
        // session 2 first open 14 vs session 1 last close 12: (14-12)/12*100 = 16.6667,
        // held for every bar of session 2; the first session has no gap.
        let out = batch(GapPct::new(), micro());
        assert_eq!(&out[..2], &[None, None]);
        near(out[2].unwrap(), 100.0 / 6.0);
        near(out[3].unwrap(), 100.0 / 6.0);
        // down gap: close 10 then open 9 -> -10
        let out = batch(GapPct::new(), [mk(1, 10.0, 10.0), mk(2, 9.0, 9.5)]);
        near(out[1].unwrap(), -10.0);
        // previous close 0 -> undefined
        let out = batch(GapPct::new(), [mk(1, 0.0, 0.0), mk(2, 9.0, 9.5)]);
        assert_eq!(out[1], None);
        // out of order skipped
        let out = batch(GapPct::new(), [mk(2, 1.0, 1.0), mk(1, 1.0, 1.0)]);
        assert_eq!(out[1], None);
        // non-finite skipped
        let out = batch(GapPct::new(), [mk(1, f64::NAN, 1.0)]);
        assert_eq!(out[0], None);
    }

    #[test]
    fn classic_pivots_hand_case() {
        // H 12, L 8, C 10: P = 10; R1 = 12, R2 = 14, R3 = 12 + 2*(10-8) = 16;
        // S1 = 8, S2 = 6, S3 = 8 - 2*(12-10) = 4.
        let p = classic_pivots(12.0, 8.0, 10.0).unwrap();
        for (got, want) in [
            (p.p, 10.0),
            (p.r1, 12.0),
            (p.r2, 14.0),
            (p.r3, 16.0),
            (p.s1, 8.0),
            (p.s2, 6.0),
            (p.s3, 4.0),
        ] {
            near(got, want);
        }
        assert_eq!(classic_pivots(f64::NAN, 1.0, 1.0), None);
        assert_eq!(classic_pivots(f64::MAX, -f64::MAX, 0.0), None);
    }

    #[test]
    fn camarilla_pivots_hand_case() {
        // range 10, C 100: R1 = 100 + 11/12, R2 = 100 + 11/6, R3 = 102.75, R4 = 105.5; supports mirror.
        let c = camarilla_pivots(110.0, 100.0, 100.0).unwrap();
        near(c.r[0], 100.0 + 11.0 / 12.0);
        near(c.r[1], 100.0 + 11.0 / 6.0);
        near(c.r[2], 102.75);
        near(c.r[3], 105.5);
        near(c.s[0], 100.0 - 11.0 / 12.0);
        near(c.s[3], 94.5);
        assert_eq!(camarilla_pivots(f64::INFINITY, 1.0, 1.0), None);
    }

    #[test]
    fn lookahead_session_indicators() {
        let xs: Vec<SessionBar> = (0..30)
            .map(|i| {
                let c = 100.0 + f64::from(i % 7);
                SessionBar {
                    session: i64::from(i / 6),
                    bar: Ohlcv {
                        ts_ms: i64::from(i) * 60_000,
                        o: c - 0.5,
                        h: c + 1.0,
                        l: c - 1.0,
                        c,
                        v: 10.0 + f64::from(i % 5),
                    },
                }
            })
            .collect();
        testkit::check_contract(SessionVwap::new, &xs, true);
        testkit::check_contract(GapPct::new, &xs, false);
    }
}
