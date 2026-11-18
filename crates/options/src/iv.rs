//! Implied volatility for Black-76: bracketed Newton with a bisection fallback.
//!
//! Solves `price(sigma) = target` on `[min_vol, max_vol]`. Price is strictly increasing in
//! sigma, so a bracket exists iff `target` lies strictly between `price(min_vol)` and
//! `price(max_vol)`. Every loop is capped by `SolverParams::max_iter`; failure is a typed error,
//! never a made-up number.

use crate::OptionKind;
use crate::black76::{Inputs, greeks, price};

/// Solver settings (defaults are numerical conventions, not market facts).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolverParams {
    /// Stop when `|price(sigma) - target| <= price_tol` (relative to the target price).
    pub price_tol: f64,
    /// Stop when the bracket is narrower than this in vol.
    pub vol_tol: f64,
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
            max_iter: 200,
            min_vol: 1e-4,
            max_vol: 10.0,
            min_years: 1e-6,
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
    /// Non-finite or non-positive forward, strike, price, or invalid solver params.
    InvalidInput,
}

/// Implied vol of `target` for `kind`. `m.vol` is ignored. Domain: finite `F, K > 0`,
/// `T > min_years`, finite `target > 0`. Returns the sigma with `price(sigma) = target`
/// (to the tolerances).
///
/// # Errors
/// See [`IvError`].
pub fn implied_vol(
    kind: OptionKind,
    target: f64,
    m: &Inputs,
    p: &SolverParams,
) -> Result<f64, IvError> {
    let params_ok = p.price_tol > 0.0
        && p.vol_tol > 0.0
        && p.min_vol > 0.0
        && p.max_vol > p.min_vol
        && p.min_years > 0.0
        && p.max_vol.is_finite();
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
            return Ok(sigma);
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
            for strike in [80.0, 95.0, 100.0, 105.0, 125.0] {
                for k in 1..=30 {
                    let sigma = 0.05 * f64::from(k);
                    let m = mk(strike);
                    let px = price(kind, &Inputs { vol: sigma, ..m }).unwrap();
                    match implied_vol(kind, px, &m, &p) {
                        Ok(got) => assert!(
                            (got - sigma).abs() < 1e-8,
                            "{kind:?} K={strike} sigma={sigma} got={got}"
                        ),
                        Err(e) => {
                            // only acceptable where the price is numerically at intrinsic
                            let disc = (-m.rate * m.years).exp();
                            let intr = match kind {
                                Call => disc * (100.0 - strike).max(0.0),
                                Put => disc * (strike - 100.0).max(0.0),
                            };
                            assert!(px - intr < 1e-6, "{e:?} {kind:?} {strike} {sigma}");
                        }
                    }
                }
            }
        }
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
                Ok(_) | Err(IvError::BelowIntrinsic | IvError::NoConvergence)
            ));
            if let Ok(v) = r {
                assert!(v.is_finite() && v > 0.0);
            }
        }
    }
}
