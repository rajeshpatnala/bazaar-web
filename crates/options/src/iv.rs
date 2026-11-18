//! Implied volatility for Black-76: bracketed Newton with a bisection fallback.
//!
//! Solves `price(sigma) = target` on `[min_vol, max_vol]`. Price is strictly increasing in
//! sigma, so a bracket exists iff `target` lies strictly between `price(min_vol)` and
//! `price(max_vol)`. Every loop is capped by `SolverParams::max_iter`; failure is a typed error,
//! never a made-up number.

use crate::OptionKind;
use crate::black76::{Inputs, greeks, price};
use crate::time::DEFAULT_EPSILON_YEARS;

/// Solver settings (defaults are numerical conventions, not market facts).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolverParams {
    /// Stop when `|price(sigma) - target| <= price_tol` (relative to the target price).
    pub price_tol: f64,
    /// Stop when the bracket is narrower than this in vol.
    pub vol_tol: f64,
    /// Largest acceptable vol uncertainty `noise / vega` at the answer; above it the result is
    /// `IllConditioned` (vega too small to identify sigma), never a number.
    pub vol_accuracy: f64,
    /// Assumed relative price noise of the pricer (times `max(F, K) * D`), default 1e-15.
    pub price_noise: f64,
    /// Iteration cap (Newton and bisection steps together).
    pub max_iter: u32,
    /// Lower end of the search bracket.
    pub min_vol: f64,
    /// Upper end of the search bracket (10.0 = 1000%).
    pub max_vol: f64,
    /// `T` at or below this is rejected as expired (matches the `year_fraction` floor).
    pub min_years: f64,
}

impl Default for SolverParams {
    fn default() -> Self {
        Self {
            price_tol: 1e-13,
            vol_tol: 1e-13,
            vol_accuracy: 1e-9,
            price_noise: 1e-15,
            max_iter: 200,
            min_vol: 1e-4,
            max_vol: 10.0,
            min_years: DEFAULT_EPSILON_YEARS,
        }
    }
}

/// Why no implied vol was returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IvError {
    /// Price at or below the discounted intrinsic value (or the value at `min_vol`).
    BelowIntrinsic,
    /// Price at or above the model's upper bound (`D F` call, `D K` put) or above the value at `max_vol`.
    AboveUpperBound,
    /// `T <= min_years`.
    ExpiredOrZeroT,
    /// Iteration cap reached without meeting a tolerance.
    NoConvergence,
    /// The price is matched but vega is too small to pin sigma down (typically deep ITM or
    /// extremely short-dated); any sigma on a plateau would fit.
    IllConditioned,
    /// Non-finite or non-positive forward, strike, price, or invalid solver params.
    InvalidInput,
}

/// Implied vol of `target` for `kind`. `m.vol` is ignored. Domain: finite `F, K > 0`,
/// `T > min_years`, finite `target > 0`. Returns the sigma with `price(sigma) = target`
/// to within `vol_accuracy`: the solver stops on price or bracket width, then verifies that
/// `noise / vega` is below `vol_accuracy`, otherwise it returns `IllConditioned`.
///
/// # Errors
/// See [`IvError`].
pub fn implied_vol(
    kind: OptionKind,
    target: f64,
    m: &Inputs,
    p: &SolverParams,
) -> Result<f64, IvError> {
    let pos = |x: f64| x.is_finite() && x > 0.0;
    let params_ok = pos(p.price_tol)
        && pos(p.vol_tol)
        && pos(p.vol_accuracy)
        && pos(p.price_noise)
        && pos(p.min_vol)
        && pos(p.max_vol)
        && p.max_vol > p.min_vol
        && pos(p.min_years);
    let finite = [m.forward, m.strike, m.years, m.rate, target]
        .iter()
        .all(|x| x.is_finite());
    if !params_ok || !finite || m.forward <= 0.0 || m.strike <= 0.0 || target <= 0.0 {
        return Err(IvError::InvalidInput);
    }
    if m.years <= p.min_years {
        return Err(IvError::ExpiredOrZeroT);
    }
    let disc = (-m.rate * m.years).exp();
    if !disc.is_finite() || disc <= 0.0 {
        return Err(IvError::InvalidInput);
    }
    let (intrinsic, upper) = match kind {
        OptionKind::Call => (disc * (m.forward - m.strike).max(0.0), disc * m.forward),
        OptionKind::Put => (disc * (m.strike - m.forward).max(0.0), disc * m.strike),
    };
    if target <= intrinsic {
        return Err(IvError::BelowIntrinsic);
    }
    if target >= upper {
        return Err(IvError::AboveUpperBound);
    }
    let at = |vol: f64| price(kind, &Inputs { vol, ..*m }).ok_or(IvError::InvalidInput);
    let (mut lo, mut hi) = (p.min_vol, p.max_vol);
    if target < at(lo)? {
        return Err(IvError::BelowIntrinsic);
    }
    if target > at(hi)? {
        return Err(IvError::AboveUpperBound);
    }
    let mut sigma = 0.2_f64.clamp(lo, hi);
    for _ in 0..p.max_iter {
        let diff = at(sigma)? - target;
        if diff.abs() <= p.price_tol * target || hi - lo <= p.vol_tol {
            // Stopping on price says nothing about sigma where vega ~ 0: check the uncertainty.
            let vega = greeks(kind, &Inputs { vol: sigma, ..*m }).map_or(0.0, |g| g.vega);
            let noise = p.price_noise * m.forward.max(m.strike) * disc + p.price_tol * target;
            return if vega > 0.0 && noise / vega <= p.vol_accuracy {
                Ok(sigma)
            } else {
                Err(IvError::IllConditioned)
            };
        }
        if diff > 0.0 {
            hi = sigma;
        } else {
            lo = sigma;
        }
        let vega = greeks(kind, &Inputs { vol: sigma, ..*m }).map_or(0.0, |g| g.vega);
        let newton = sigma - diff / vega;
        sigma = if vega > 1e-300 && newton.is_finite() && newton > lo && newton < hi {
            newton
        } else {
            f64::midpoint(lo, hi)
        };
    }
    Err(IvError::NoConvergence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use OptionKind::{Call, Put};

    fn mk(strike: f64) -> Inputs {
        Inputs {
            forward: 100.0,
            strike,
            years: 0.25,
            rate: 0.05,
            vol: 0.0,
        }
    }

    #[test]
    fn round_trips_over_a_grid() {
        let p = SolverParams::default();
        for kind in [Call, Put] {
            for strike in [60.0, 80.0, 95.0, 100.0, 105.0, 125.0] {
                for k in 1..=30 {
                    let sigma = 0.05 * f64::from(k) + 0.013;
                    let m = mk(strike);
                    let px = price(kind, &Inputs { vol: sigma, ..m }).unwrap();
                    match implied_vol(kind, px, &m, &p) {
                        Ok(got) => assert!(
                            (got - sigma).abs() < 1e-8,
                            "{kind:?} K={strike} sigma={sigma} got={got}"
                        ),
                        Err(e) => assert!(
                            matches!(
                                e,
                                IvError::BelowIntrinsic
                                    | IvError::AboveUpperBound
                                    | IvError::IllConditioned
                                    | IvError::NoConvergence
                            ),
                            "{e:?}"
                        ),
                    }
                }
            }
        }
    }

    #[test]
    fn deep_itm_is_error_or_accurate() {
        // K=60, T=0.02 (vega ~ 0): previously returned 0.2 for a true 0.5.
        let p = SolverParams::default();
        let m = Inputs {
            years: 0.02,
            ..mk(60.0)
        };
        for sigma in [0.08, 0.12, 0.5] {
            let px = price(Call, &Inputs { vol: sigma, ..m }).unwrap();
            match implied_vol(Call, px, &m, &p) {
                Ok(got) => assert!((got - sigma).abs() < 1e-8, "{sigma} -> {got}"),
                Err(e) => assert!(
                    matches!(e, IvError::IllConditioned | IvError::BelowIntrinsic),
                    "{e:?}"
                ),
            }
        }
        // and the ill-conditioned case is reported as such
        let px = price(Call, &Inputs { vol: 0.5, ..m }).unwrap();
        assert!(implied_vol(Call, px, &m, &p).is_err());
    }

    proptest::proptest! {
        #[test]
        fn off_grid_round_trip_never_silently_wrong(
            sigma in 0.03f64..2.5,
            strike in 55.0f64..170.0,
            years in 0.003f64..3.0,
            rate in 0.0f64..0.1,
            is_call in proptest::bool::ANY,
        ) {
            let kind = if is_call { Call } else { Put };
            let m = Inputs { forward: 100.0, strike, years, rate, vol: 0.0 };
            let px = price(kind, &Inputs { vol: sigma, ..m }).unwrap();
            if let Ok(got) = implied_vol(kind, px, &m, &SolverParams::default()) {
                proptest::prop_assert!((got - sigma).abs() < 1e-8, "sigma {} got {}", sigma, got);
            }
        }
    }

    #[test]
    fn non_finite_discount_is_invalid_input() {
        let m = Inputs {
            forward: 100.0,
            strike: 100.0,
            years: 1e3,
            rate: 1e6,
            vol: 0.0,
        };
        assert_eq!(
            implied_vol(Call, 5.0, &m, &SolverParams::default()),
            Err(IvError::InvalidInput)
        );
    }

    #[test]
    fn central_cases_recover() {
        let p = SolverParams::default();
        for kind in [Call, Put] {
            let m = mk(100.0);
            let px = price(kind, &Inputs { vol: 0.2, ..m }).unwrap();
            assert!((implied_vol(kind, px, &m, &p).unwrap() - 0.2).abs() < 1e-9);
        }
    }

    #[test]
    fn typed_errors() {
        let p = SolverParams::default();
        let m = mk(90.0);
        let disc = (-m.rate * m.years).exp();
        let r = |kind, px, mm: &Inputs, pp: &SolverParams| implied_vol(kind, px, mm, pp);
        assert_eq!(
            r(Call, disc * 10.0 - 0.5, &m, &p),
            Err(IvError::BelowIntrinsic)
        );
        assert_eq!(r(Call, disc * 10.0, &m, &p), Err(IvError::BelowIntrinsic));
        assert_eq!(
            r(Call, disc * 100.0 + 1.0, &m, &p),
            Err(IvError::AboveUpperBound)
        );
        assert_eq!(r(Put, disc * 90.0, &m, &p), Err(IvError::AboveUpperBound));
        let expired = Inputs { years: 1e-6, ..m };
        assert_eq!(r(Call, 11.0, &expired, &p), Err(IvError::ExpiredOrZeroT));
        for bad in [f64::NAN, 0.0, -1.0, f64::INFINITY] {
            assert_eq!(r(Call, bad, &m, &p), Err(IvError::InvalidInput));
        }
        let badm = Inputs { strike: 0.0, ..m };
        assert_eq!(r(Call, 1.0, &badm, &p), Err(IvError::InvalidInput));
        let nan_params = SolverParams {
            min_vol: f64::NAN,
            ..p
        };
        assert_eq!(r(Call, 11.0, &m, &nan_params), Err(IvError::InvalidInput));
    }

    #[test]
    fn iteration_cap_gives_no_convergence() {
        let p = SolverParams {
            max_iter: 1,
            price_tol: 1e-15,
            vol_tol: 1e-15,
            ..SolverParams::default()
        };
        let m = mk(100.0);
        let px = price(Call, &Inputs { vol: 0.37, ..m }).unwrap();
        assert_eq!(implied_vol(Call, px, &m, &p), Err(IvError::NoConvergence));
    }

    #[test]
    fn deep_otm_does_not_panic() {
        let p = SolverParams::default();
        let m = Inputs {
            years: 0.01,
            ..mk(300.0)
        };
        for px in [1e-300, 1e-12, 1e-6] {
            let r = implied_vol(Call, px, &m, &p);
            assert!(matches!(
                r,
                Ok(_)
                    | Err(IvError::BelowIntrinsic
                        | IvError::NoConvergence
                        | IvError::IllConditioned)
            ));
            if let Ok(v) = r {
                assert!(v.is_finite() && v > 0.0);
            }
        }
    }
}
