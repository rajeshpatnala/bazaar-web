//! The permanent disclaimer constant, for exports and notifications. It must stay byte-equal to
//! the UI constant in `web/src/shared/disclaimer.ts`; the Rust-vs-TS equality test is added by
//! F1 slice 9 (workspace checks). Never paraphrase.

/// Exact wording from the `bz-india-markets` compliance reference.
pub const DISCLAIMER: &str = "For education and personal analysis. Not investment advice. Past performance does not guarantee future results.";

/// Appends the disclaimer to an export or notification body on its own line.
#[must_use]
pub fn with_disclaimer(body: &str) -> String {
    format!("{body}\n{DISCLAIMER}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_text() {
        assert_eq!(
            DISCLAIMER,
            "For education and personal analysis. Not investment advice. Past performance does not guarantee future results."
        );
    }

    #[test]
    fn appended_after_body() {
        assert_eq!(with_disclaimer("a,b"), format!("a,b\n{DISCLAIMER}"));
    }
}
