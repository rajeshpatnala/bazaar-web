//! Black-76 pricing and raw Greeks on the forward. Raw units: per unit, per year, per 1.00 vol.
//!
//! `d1 = (ln(F/K) + s^2 T/2)/(s sqrt T)`, `d2 = d1 - s sqrt T`, `D = e^{-rT}`.
//! Call `= D(F N(d1) - K N(d2))`, put `= D(K N(-d2) - F N(-d1))`.
//! Domain: `F > 0`, `K > 0`, `T > 0`, `sigma > 0`, `r` finite. Outside it (or any non-finite
//! input) the functions return `None` rather than NaN. Callers convert to display units.

use crate::{OptionKind, norm};

/// Model inputs (all supplied by the caller; nothing is read from market or config here).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inputs {
    /// Forward price `F` (future for the same expiry).
    pub forward: f64,
    /// Strike `K`.
    pub strike: f64,
    /// Time to expiry `T` in years.
    pub years: f64,
    /// Continuously compounded rate `r`.
    pub rate: f64,
    /// Volatility `sigma` (annualised, 0.20 = 20%).
    pub vol: f64,
}

/// Raw Greeks. `delta`/`gamma` are against the forward `F`; `theta` is per year of calendar time
/// (value change as time passes, `-dV/dT`); `rho` holds `F` fixed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Greeks {
    /// `dV/dF`.
    pub delta: f64,
    /// `d2V/dF2`.
    pub gamma: f64,
    /// `dV/dsigma` (per 1.00 of vol).
    pub vega: f64,
    /// `-dV/dT`.
    pub theta: f64,
    /// `dV/dr` with `F` fixed, `-T V`.
    pub rho: f64,
}

struct Terms {
    d1: f64,
    d2: f64,
    disc: f64,
    sqrt_t: f64,
}

fn setup(i: &Inputs) -> Option<Terms> {
    let all = [i.forward, i.strike, i.years, i.rate, i.vol];
    if !all.iter().all(|x| x.is_finite()) {
        return None;
    }
    if i.forward <= 0.0 || i.strike <= 0.0 || i.years <= 0.0 || i.vol <= 0.0 {
        return None;
    }
    let sqrt_t = i.years.sqrt();
    let sd = i.vol * sqrt_t;
    let d1 = ((i.forward / i.strike).ln() + 0.5 * sd * sd) / sd;
    let disc = (-i.rate * i.years).exp();
    let t = Terms {
        d1,
        d2: d1 - sd,
        disc,
        sqrt_t,
    };
    (t.d1.is_finite() && t.disc.is_finite()).then_some(t)
}

fn value(kind: OptionKind, i: &Inputs, t: &Terms) -> f64 {
    match kind {
        OptionKind::Call => t.disc * (i.forward * norm::cdf(t.d1) - i.strike * norm::cdf(t.d2)),
        OptionKind::Put => t.disc * (i.strike * norm::cdf(-t.d2) - i.forward * norm::cdf(-t.d1)),
    }
}

/// Black-76 option price (formula in the module doc). `None` outside the domain.
#[must_use]
pub fn price(kind: OptionKind, i: &Inputs) -> Option<f64> {
    let t = setup(i)?;
    Some(value(kind, i, &t).max(0.0)).filter(|v| v.is_finite())
}

/// Raw Greeks. Delta: call `D N(d1)`, put `-D N(-d1)`. Gamma `D n(d1)/(F s sqrt T)`.
/// Vega `D F n(d1) sqrt T`. Theta `r V - D F n(d1) s/(2 sqrt T)`. Rho `-T V`.
/// `None` outside the domain (see module doc).
#[must_use]
pub fn greeks(kind: OptionKind, i: &Inputs) -> Option<Greeks> {
    let t = setup(i)?;
    let v = value(kind, i, &t).max(0.0);
    let n1 = norm::pdf(t.d1);
    let delta = match kind {
        OptionKind::Call => t.disc * norm::cdf(t.d1),
        OptionKind::Put => -t.disc * norm::cdf(-t.d1),
    };
    let g = Greeks {
        delta,
        gamma: t.disc * n1 / (i.forward * i.vol * t.sqrt_t),
        vega: t.disc * i.forward * n1 * t.sqrt_t,
        theta: i.rate * v - t.disc * i.forward * n1 * i.vol / (2.0 * t.sqrt_t),
        rho: -i.years * v,
    };
    [g.delta, g.gamma, g.vega, g.theta, g.rho]
        .iter()
        .all(|x| x.is_finite())
        .then_some(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use OptionKind::{Call, Put};

    fn base() -> Inputs {
        Inputs {
            forward: 100.0,
            strike: 95.0,
            years: 0.5,
            rate: 0.05,
            vol: 0.25,
        }
    }

    #[test]
    fn parity_holds() {
        let b = base();
        let c = price(Call, &b).unwrap();
        let p = price(Put, &b).unwrap();
        assert!((c - p - (-b.rate * b.years).exp() * (b.forward - b.strike)).abs() < 1e-10);
    }

    #[test]
    fn atm_micro_case() {
        // F=K, r=0: d1 = s sqrt(T)/2, call = F(2N(d1)-1); put equals call.
        let b = Inputs {
            forward: 100.0,
            strike: 100.0,
            years: 1.0,
            rate: 0.0,
            vol: 0.2,
        };
        let expect = 100.0 * (2.0 * norm::cdf(0.1) - 1.0);
        assert!((price(Call, &b).unwrap() - expect).abs() < 1e-8);
        assert!((price(Put, &b).unwrap() - expect).abs() < 1e-8);
        // Table value N(0.1) = 0.5398278372770290 -> 7.9655674554058
        assert!((expect - 7.965_567_455_405_8).abs() < 1e-8);
    }

    #[test]
    fn discounting_micro_case() {
        let b = Inputs {
            forward: 100.0,
            strike: 100.0,
            years: 1.0,
            rate: 0.1,
            vol: 0.2,
        };
        let c = price(Call, &b).unwrap();
        assert!((c - (-0.1f64).exp() * 7.965_567_455_405_8).abs() < 1e-8);
    }

    #[test]
    fn greeks_match_finite_differences() {
        let b = base();
        for kind in [Call, Put] {
            let g = greeks(kind, &b).unwrap();
            let pr = |df: f64, dt: f64, dr: f64, dv: f64| {
                price(
                    kind,
                    &Inputs {
                        forward: b.forward + df,
                        years: b.years + dt,
                        rate: b.rate + dr,
                        vol: b.vol + dv,
                        ..b
                    },
                )
                .unwrap()
            };
            let h = 1e-3;
            let delta = (pr(h, 0.0, 0.0, 0.0) - pr(-h, 0.0, 0.0, 0.0)) / (2.0 * h);
            let gamma = (pr(h, 0.0, 0.0, 0.0) - 2.0 * pr(0.0, 0.0, 0.0, 0.0)
                + pr(-h, 0.0, 0.0, 0.0))
                / (h * h);
            let hs = 1e-5;
            let vega = (pr(0.0, 0.0, 0.0, hs) - pr(0.0, 0.0, 0.0, -hs)) / (2.0 * hs);
            let ht = 1e-6;
            let theta = -(pr(0.0, ht, 0.0, 0.0) - pr(0.0, -ht, 0.0, 0.0)) / (2.0 * ht);
            let rho = (pr(0.0, 0.0, hs, 0.0) - pr(0.0, 0.0, -hs, 0.0)) / (2.0 * hs);
            assert!((g.delta - delta).abs() < 1e-6, "delta {kind:?}");
            assert!((g.gamma - gamma).abs() < 1e-6, "gamma {kind:?}");
            assert!((g.vega - vega).abs() < 1e-6, "vega {kind:?}");
            assert!((g.theta - theta).abs() < 1e-6, "theta {kind:?}");
            assert!((g.rho - rho).abs() < 1e-6, "rho {kind:?}");
        }
    }

    #[test]
    fn call_put_delta_difference_is_discount() {
        let b = base();
        let c = greeks(Call, &b).unwrap();
        let p = greeks(Put, &b).unwrap();
        assert!((c.delta - p.delta - (-b.rate * b.years).exp()).abs() < 1e-12);
        assert!((c.gamma - p.gamma).abs() < 1e-15 && (c.vega - p.vega).abs() < 1e-12);
    }

    #[test]
    fn invalid_inputs_return_none() {
        let b = base();
        let bad = [
            Inputs { forward: 0.0, ..b },
            Inputs { strike: 0.0, ..b },
            Inputs { years: 0.0, ..b },
            Inputs { vol: 0.0, ..b },
            Inputs { forward: -1.0, ..b },
            Inputs {
                forward: f64::NAN,
                ..b
            },
            Inputs {
                rate: f64::INFINITY,
                ..b
            },
        ];
        for x in bad {
            assert!(price(Call, &x).is_none());
            assert!(greeks(Put, &x).is_none());
        }
    }

    #[test]
    fn extreme_inputs_never_nan() {
        let b = base();
        let cases = [
            Inputs {
                strike: 1e-3,
                years: 1e-6,
                vol: 0.01,
                ..b
            },
            Inputs {
                forward: 1.0,
                strike: 1e6,
                years: 1e-6,
                vol: 0.01,
                ..b
            },
            Inputs {
                strike: 100.0,
                years: 50.0,
                vol: 5.0,
                ..b
            },
        ];
        for x in cases {
            if let Some(v) = price(Call, &x) {
                assert!(v.is_finite() && v >= 0.0);
            }
            if let Some(g) = greeks(Put, &x) {
                assert!(g.delta.is_finite() && g.theta.is_finite());
            }
        }
    }

    #[test]
    fn hull_black76_example() {
        // Hull: futures option F=K=20, r=9%, sigma=25%, T=4/12 -> 1.1166 (call = put).
        let b = Inputs {
            forward: 20.0,
            strike: 20.0,
            years: 4.0 / 12.0,
            rate: 0.09,
            vol: 0.25,
        };
        assert!((price(Call, &b).unwrap() - 1.1166).abs() < 5e-5);
        assert!((price(Put, &b).unwrap() - 1.1166).abs() < 5e-5);
    }

    #[test]
    fn deep_otm_price_is_never_negative() {
        let b = Inputs {
            forward: 100.0,
            strike: 400.0,
            years: 0.01,
            rate: 0.05,
            vol: 0.1,
        };
        assert!(price(Call, &b).unwrap() >= 0.0);
        let p = Inputs { strike: 10.0, ..b };
        assert!(price(Put, &p).unwrap() >= 0.0);
    }

    #[test]
    fn greek_signs_and_limits() {
        let b = base();
        let d = (-b.rate * b.years).exp();
        for kind in [Call, Put] {
            let g = greeks(kind, &b).unwrap();
            assert!(g.gamma > 0.0 && g.vega > 0.0);
        }
        let itm_call = greeks(Call, &Inputs { strike: 10.0, ..b }).unwrap();
        assert!((itm_call.delta - d).abs() < 1e-9);
        let otm_call = greeks(
            Call,
            &Inputs {
                strike: 1000.0,
                ..b
            },
        )
        .unwrap();
        assert!(otm_call.delta.abs() < 1e-9);
        let itm_put = greeks(
            Put,
            &Inputs {
                strike: 1000.0,
                ..b
            },
        )
        .unwrap();
        assert!((itm_put.delta + d).abs() < 1e-9);
        let otm_put = greeks(Put, &Inputs { strike: 10.0, ..b }).unwrap();
        assert!(otm_put.delta.abs() < 1e-9);
        // T -> 0 at the money: delta -> D/2 for a call, gamma and vega stay finite.
        let tiny = Inputs {
            strike: 100.0,
            years: 1e-6,
            ..b
        };
        let g = greeks(Call, &tiny).unwrap();
        assert!((g.delta - 0.5).abs() < 1e-2 && g.gamma.is_finite() && g.vega >= 0.0);
        // T -> 0 in the money: delta -> D, vega -> 0.
        let g = greeks(
            Call,
            &Inputs {
                strike: 90.0,
                ..tiny
            },
        )
        .unwrap();
        assert!((g.delta - 1.0).abs() < 1e-6 && g.vega < 1e-6);
    }

    proptest::proptest! {
        #[test]
        fn parity_monotone_and_intrinsic(
            forward in 50.0f64..200.0,
            strike in 50.0f64..200.0,
            years in 0.01f64..2.0,
            rate in 0.0f64..0.12,
            vol in 0.05f64..1.5,
        ) {
            let i = Inputs { forward, strike, years, rate, vol };
            let disc = (-rate * years).exp();
            let c = price(Call, &i).unwrap();
            let p = price(Put, &i).unwrap();
            proptest::prop_assert!((c - p - disc * (forward - strike)).abs() < 1e-10);
            proptest::prop_assert!(c >= (disc * (forward - strike)).max(0.0) - 1e-10);
            proptest::prop_assert!(p >= (disc * (strike - forward)).max(0.0) - 1e-10);
            let up = Inputs { vol: vol + 0.01, ..i };
            proptest::prop_assert!(price(Call, &up).unwrap() >= c - 1e-12);
            proptest::prop_assert!(price(Put, &up).unwrap() >= p - 1e-12);
        }
    }
}
