//! Standard normal PDF and CDF, written in-crate (no dependency).
//!
//! `N(x) = erfc(-x/sqrt 2) / 2`. `erf` uses the non-alternating series
//! `erf z = 2/sqrt(pi) * e^{-z^2} * sum_n 2^n z^{2n+1} / (1*3*...*(2n+1))` for `|z| < 2`
//! (no cancellation), and the Laplace continued fraction
//! `erfc z = e^{-z^2}/sqrt(pi) * 1/(z + (1/2)/(z + 1/(z + (3/2)/(z + ...))))` for `z >= 2`
//! (Abramowitz and Stegun 7.1.14 context). Accuracy is near machine precision.

const FRAC_1_SQRT_2PI: f64 = 0.398_942_280_401_432_7;
const FRAC_1_SQRT_PI: f64 = 0.564_189_583_547_756_3;

/// Standard normal density `exp(-x^2/2)/sqrt(2 pi)`. Domain: any finite `x`; NaN in, NaN out.
#[must_use]
pub fn pdf(x: f64) -> f64 {
    FRAC_1_SQRT_2PI * (-0.5 * x * x).exp()
}

/// `erfc(z)` for `z >= 0`.
fn erfc_nonneg(z: f64) -> f64 {
    if z < 2.0 {
        let mut term = z;
        let mut sum = z;
        let mut n = 0.0_f64;
        while term > sum * 1e-17 && n < 500.0 {
            n += 1.0;
            term *= 2.0 * z * z / (2.0 * n + 1.0);
            sum += term;
        }
        1.0 - 2.0 * FRAC_1_SQRT_PI * (-z * z).exp() * sum
    } else {
        let mut f = z;
        for k in (1..=120).rev() {
            f = z + (f64::from(k) / 2.0) / f;
        }
        FRAC_1_SQRT_PI * (-z * z).exp() / f
    }
}

/// Standard normal CDF `N(x)`. Domain: any `x`; NaN in, NaN out; `x = +-inf` gives 1 / 0.
#[must_use]
pub fn cdf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let z = x.abs() / std::f64::consts::SQRT_2;
    let tail = 0.5 * erfc_nonneg(z); // N(-|x|)
    if x >= 0.0 { 1.0 - tail } else { tail }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_values() {
        assert!((cdf(0.0) - 0.5).abs() < 1e-12);
        assert!((cdf(1.0) - 0.841_344_746_068_542_9).abs() < 1e-12);
        assert!((cdf(-1.96) - 0.024_997_895_148_220_4).abs() < 1e-12);
        assert!((cdf(3.0) - 0.998_650_101_968_369_9).abs() < 1e-12);
    }

    #[test]
    fn symmetric_monotone_and_edges() {
        let mut prev = 0.0;
        let mut x = -9.0;
        while x < 9.0 {
            let c = cdf(x);
            assert!(c >= prev && (0.0..=1.0).contains(&c));
            assert!((c + cdf(-x) - 1.0).abs() < 1e-14);
            prev = c;
            x += 0.0137;
        }
        assert_eq!(cdf(f64::INFINITY), 1.0);
        assert_eq!(cdf(f64::NEG_INFINITY), 0.0);
        assert!(cdf(f64::NAN).is_nan());
    }

    #[test]
    fn pdf_value_and_derivative_of_cdf() {
        assert!((pdf(0.0) - 0.398_942_280_401_432_7).abs() < 1e-15);
        let h = 1e-5;
        for x in [-2.0, -0.3, 0.7, 2.5] {
            let d = (cdf(x + h) - cdf(x - h)) / (2.0 * h);
            assert!((d - pdf(x)).abs() < 1e-9);
        }
    }
}
