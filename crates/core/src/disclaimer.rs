//! The permanent disclaimer constant, for exports and notifications. Byte-equal to the UI
//! constant in `web/src/shared/disclaimer.ts` (checked by the workspace tests); never paraphrase.

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
