//! Plain-English number formatting for gate messages: rupees with Indian grouping, percentages.

use rust_decimal::{Decimal, RoundingStrategy};

/// Rupees first (half rounds away from zero, so a loss is never understated), Indian digit grouping, paise only when present: `Rs 3,050`, `-Rs 1,00,000.50`.
#[must_use]
pub fn rupees(amount: Decimal) -> String {
    let rounded = amount
        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        .normalize();
    let negative = rounded.is_sign_negative() && !rounded.is_zero();
    let text = rounded.abs().to_string();
    let (int_part, frac_part) = match text.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (text.as_str(), None),
    };
    let mut grouped: Vec<char> = Vec::new();
    for (i, c) in int_part.chars().rev().enumerate() {
        if i == 3 || (i > 3 && i.saturating_sub(3).is_multiple_of(2)) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let int_str: String = grouped.into_iter().rev().collect();
    let sign = if negative { "-" } else { "" };
    match frac_part {
        // Two digits when paise exist (`0.5` prints as `.50`).
        Some(f) => format!("{sign}Rs {int_str}.{f:0<2}"),
        None => format!("{sign}Rs {int_str}"),
    }
}

/// A percentage without trailing zeros: `3%`, `0.5%`.
#[must_use]
pub fn pct(value: Decimal) -> String {
    format!("{}%", value.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn rupees_groups_the_indian_way() {
        let cases = [
            ("0", "Rs 0"),
            ("999", "Rs 999"),
            ("1000", "Rs 1,000"),
            ("3050", "Rs 3,050"),
            ("100000", "Rs 1,00,000"),
            ("12345678", "Rs 1,23,45,678"),
            ("-3050", "-Rs 3,050"),
            ("1000.5", "Rs 1,000.50"),
            ("940.00", "Rs 940"),
            ("0.004", "Rs 0"),
            ("-0.004", "Rs 0"),
            ("3000.005", "Rs 3,000.01"),
            ("-3000.005", "-Rs 3,000.01"),
            ("0.005", "Rs 0.01"),
            ("0.015", "Rs 0.02"),
            ("0.0049", "Rs 0"),
            ("10000000", "Rs 1,00,00,000"),
            ("123456789012", "Rs 1,23,45,67,89,012"),
            ("-100000000.5", "-Rs 10,00,00,000.50"),
        ];
        for (input, want) in cases {
            assert_eq!(rupees(d(input)), want, "input {input}");
        }
    }

    #[test]
    fn pct_trims_zeros() {
        assert_eq!(pct(d("3.00")), "3%");
        assert_eq!(pct(d("0.5")), "0.5%");
    }
}
