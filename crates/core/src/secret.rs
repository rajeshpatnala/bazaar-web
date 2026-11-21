//! `Secret` and `AccessToken`: values that must never reach a log, an error or a DTO.
//!
//! No `Display`, no `Serialize`, no `Clone` on `Secret`; `Debug` prints a fixed marker. The only
//! way to read the value is `expose`, which names the intent at the call site.

use std::fmt;

use chrono::DateTime;
use chrono_tz::Tz;

/// A secret string (API key, API secret, access token).
#[derive(PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a value. The caller must not keep another copy in a loggable place.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The raw value, for building an outbound header or a checksum only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when the value is empty or only whitespace.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// A broker access token with its expiry (IST).
#[derive(Debug, PartialEq, Eq)]
pub struct AccessToken {
    secret: Secret,
    expires_at: DateTime<Tz>,
}

impl AccessToken {
    #[must_use]
    pub fn new(secret: Secret, expires_at: DateTime<Tz>) -> Self {
        Self { secret, expires_at }
    }

    /// The token value (see [`Secret::expose`]).
    #[must_use]
    pub fn secret(&self) -> &Secret {
        &self.secret
    }

    /// When the broker stops accepting the token.
    #[must_use]
    pub fn expires_at(&self) -> DateTime<Tz> {
        self.expires_at
    }

    /// True while `now` is before the expiry.
    #[must_use]
    pub fn is_valid_at(&self, now: DateTime<Tz>) -> bool {
        now < self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    use super::*;

    #[test]
    fn debug_never_shows_the_value() {
        let s = Secret::new("abc123-super".to_owned());
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert!(!format!("{s:#?}").contains("abc123"));
        assert_eq!(s.expose(), "abc123-super");
    }

    #[test]
    fn access_token_debug_is_redacted_and_expiry_checked() {
        let exp = Kolkata.with_ymd_and_hms(2026, 11, 22, 6, 0, 0).unwrap();
        let t = AccessToken::new(Secret::new("tok-XYZ".to_owned()), exp);
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("tok-XYZ"), "{dbg}");
        assert!(t.is_valid_at(exp - chrono::Duration::seconds(1)));
        assert!(!t.is_valid_at(exp));
        assert_eq!(t.secret().expose(), "tok-XYZ");
    }

    #[test]
    fn blank_detection() {
        assert!(Secret::new("  ".to_owned()).is_blank());
        assert!(!Secret::new("x".to_owned()).is_blank());
    }
}
