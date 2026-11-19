//! Session VWAP, gap %, classic and Camarilla pivots.
//!
//! Session boundaries are not computed here (no calendar, no clock, no hardcoded open time): the
//! caller tags each bar with an opaque, non-decreasing `session` id. The convention is the IST
//! trading date of the bar's `close_time`, accepted by `MarketCalendar::is_trading_day` (the
//! tests derive it that way). Muhurat and other special sessions would need their own id and are
//! out of scope here. A change of id starts a new session; an id lower than the current one is
//! out of order and the bar is skipped (`None`, state untouched).

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
/// session. If the first bar of a session is non-finite the whole session has no gap (the next
/// open is not the session open). The caller supplies a consistently adjusted series (a split
/// between two sessions on raw prices gives a fake gap) and contiguous sessions: ids are opaque,
/// so a missing session is spanned and the gap covers the whole jump. `warmup()` is 1 but the
/// first `Some` is on the first bar of the second session.
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
        if self.session.is_some_and(|s| x.session < s) {
            return None;
        }
        let ok = finite(&x.bar);
        if self.session != Some(x.session) {
            // New session: decided on its first bar even when that bar is skipped.
            self.gap = self
                .session
                .filter(|_| ok && self.last_close != 0.0)
                .map(|_| (x.bar.o - self.last_close) / self.last_close * 100.0)
                .filter(|g| g.is_finite());
            self.session = Some(x.session);
        }
        if !ok {
            return None;
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

/// Classic pivots for the next session. Inputs are the previous *completed* session's `high`,
/// `low`, `close` (never today's running values). `None` if an input is not finite, `high < low`,
/// or a level overflows.
#[must_use]
pub fn classic_pivots(high: f64, low: f64, close: f64) -> Option<ClassicPivots> {
    if high < low {
        return None;
    }
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

/// Camarilla pivots for the next session. Inputs are the previous *completed* session's `high`,
/// `low`, `close`. `None` if an input is not finite, `high < low`, or a level overflows.
#[must_use]
pub fn camarilla_pivots(high: f64, low: f64, close: f64) -> Option<CamarillaPivots> {
    if high < low {
        return None;
    }
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
    use bazaar_core::calendar::MarketCalendar;
    use bazaar_core::config::CalendarConfig;
    use bazaar_core::segment::Segment;
    use chrono::{Datelike, TimeZone};
    use chrono_tz::Asia::Kolkata;
    use proptest::prelude::*;
    use std::path::Path;

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
    fn gap_first_bar_non_finite_means_no_gap_for_the_session() {
        let out = batch(
            GapPct::new(),
            [
                mk(1, 10.0, 10.0),
                mk(2, f64::NAN, 11.0),
                mk(2, 12.0, 12.0),
                mk(3, 13.0, 13.0),
            ],
        );
        assert_eq!(out[1], None);
        assert_eq!(out[2], None);
        // session 3 gaps from the last accepted close (12): (13-12)/12*100
        near(out[3].unwrap(), 100.0 / 12.0);
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
        assert_eq!(classic_pivots(1.0, 2.0, 1.5), None);
        // flat range H=L=C=7: every level is 7
        let f = classic_pivots(7.0, 7.0, 7.0).unwrap();
        for v in [f.p, f.r1, f.r2, f.r3, f.s1, f.s2, f.s3] {
            near(v, 7.0);
        }
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
        near(c.s[1], 100.0 - 11.0 / 6.0);
        near(c.s[2], 97.25);
        near(c.s[3], 94.5);
        assert_eq!(camarilla_pivots(f64::INFINITY, 1.0, 1.0), None);
        assert_eq!(camarilla_pivots(1.0, 2.0, 1.5), None);
        let f = camarilla_pivots(7.0, 7.0, 7.0).unwrap();
        assert!(
            f.r.iter()
                .chain(f.s.iter())
                .all(|v| (v - 7.0).abs() < 1e-12)
        );
    }

    fn calendar() -> MarketCalendar {
        let cfg: CalendarConfig = bazaar_core::config::load(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/config"
        )))
        .unwrap();
        MarketCalendar::from_config(&cfg)
    }

    /// Session id convention: IST trading date of the bar close, `None` on a non-trading day.
    fn session_id(cal: &MarketCalendar, ts_ms: i64) -> Option<i64> {
        let d = Kolkata.timestamp_millis_opt(ts_ms).single()?.date_naive();
        cal.is_trading_day(d, Segment::Equity)
            .ok()?
            .then(|| i64::from(d.num_days_from_ce()))
    }

    #[test]
    fn calendar_derived_session_id_resets_vwap_and_gap_across_a_holiday() {
        // Fixture calendar: Wed 2026-03-11 and Fri 2026-03-13 trade; Thu 2026-03-12 is a holiday.
        let cal = calendar();
        let ts = |d, h, m| {
            Kolkata
                .with_ymd_and_hms(2026, 3, d, h, m, 0)
                .unwrap()
                .timestamp_millis()
        };
        assert_eq!(session_id(&cal, ts(12, 10, 30)), None);
        // (day, hour, minute, open, high, low, close, volume)
        let rows = [
            (11, 10, 1, 100.0, 102.0, 99.0, 101.0, 10.0),
            (11, 14, 0, 101.0, 103.0, 100.0, 102.0, 30.0),
            (13, 10, 1, 105.0, 106.0, 104.0, 105.0, 20.0),
            (13, 10, 2, 105.0, 107.0, 105.0, 106.0, 20.0),
        ];
        let bars: Vec<SessionBar> = rows
            .iter()
            .map(|&(d, h, m, o, hi, lo, c, v)| {
                let ts_ms = ts(d, h, m);
                SessionBar {
                    session: session_id(&cal, ts_ms).unwrap(),
                    bar: Ohlcv {
                        ts_ms,
                        o,
                        h: hi,
                        l: lo,
                        c,
                        v,
                    },
                }
            })
            .collect();
        assert_eq!(bars[0].session, bars[1].session);
        assert_eq!(bars[2].session, bars[3].session);
        assert!(bars[2].session > bars[1].session);
        // Day 1: 302/3 = 100.6667 then (3020/3 + 9150/3)/40 = 101.4167.
        // Day 2 resets: typical 105 -> 105; then (105*20 + 106*20)/40 = 105.5.
        let v = batch(SessionVwap::new(), bars.clone());
        near(v[0].unwrap(), 302.0 / 3.0);
        near(v[1].unwrap(), 12170.0 / 120.0);
        near(v[2].unwrap(), 105.0);
        near(v[3].unwrap(), 105.5);
        // Gap across the holiday: (105 - 102) / 102 * 100.
        let g = batch(GapPct::new(), bars);
        assert_eq!((g[0], g[1]), (None, None));
        near(g[2].unwrap(), 300.0 / 102.0);
        near(g[3].unwrap(), 300.0 / 102.0);
    }

    /// Naive per-session VWAP and gap, written from the definitions.
    fn naive(xs: &[SessionBar]) -> Vec<(Option<f64>, Option<f64>)> {
        (0..xs.len())
            .map(|i| {
                let s = xs[i].session;
                let first = (0..=i).find(|&j| xs[j].session == s).unwrap();
                let (pv, vol) = xs[first..=i].iter().fold((0.0, 0.0), |(pv, vol), b| {
                    (
                        pv + (b.bar.h + b.bar.l + b.bar.c) / 3.0 * b.bar.v,
                        vol + b.bar.v,
                    )
                });
                let vwap = (vol > 0.0).then(|| pv / vol);
                let gap = (first > 0).then(|| {
                    let pc = xs[first - 1].bar.c;
                    (xs[first].bar.o - pc) / pc * 100.0
                });
                (vwap, gap)
            })
            .collect()
    }

    proptest! {
        #[test]
        fn vwap_and_gap_match_naive(
            v in proptest::collection::vec((0i64..3, 10.0f64..200.0, 0.0f64..5.0, 0.0f64..5.0, 0u32..1000), 0..40),
        ) {
            let mut session = 0;
            let xs: Vec<SessionBar> = v.iter().enumerate().map(|(i, &(step, c, u, d, vol))| {
                session += step;
                SessionBar { session, bar: Ohlcv {
                    ts_ms: i64::try_from(i).unwrap() * 60_000,
                    o: c - d / 2.0, h: c + u, l: c - d, c, v: f64::from(vol),
                } }
            }).collect();
            testkit::check_contract(SessionVwap::new, &xs, false);
            testkit::check_contract(GapPct::new, &xs, false);
            let vw = batch(SessionVwap::new(), xs.clone());
            let gp = batch(GapPct::new(), xs.clone());
            for (i, (nv, ng)) in naive(&xs).into_iter().enumerate() {
                match (vw[i], nv) {
                    (Some(a), Some(b)) => near(a, b),
                    (None, None) => {}
                    _ => prop_assert!(false, "vwap mismatch at {}", i),
                }
                match (gp[i], ng) {
                    (Some(a), Some(b)) => near(a, b),
                    (None, None) => {}
                    _ => prop_assert!(false, "gap mismatch at {}", i),
                }
            }
        }
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
