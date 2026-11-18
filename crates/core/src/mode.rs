//! Execution mode. Read only from `config/risk.toml` `[mode]`; never from an env var, CLI flag
//! or HTTP request. The default is `paper`.
//!
//! Unlocking `Auto` is owned by `bazaar-execution` (`AutoUnlock`); this type is the shared name.

use serde::{Deserialize, Serialize};

/// How approved orders are executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Virtual rupee cash; every order still needs a user approval.
    #[default]
    Paper,
    /// Live orders, each approved by the user.
    SemiAuto,
    /// Live orders under an `AutoPolicy`; locked behind prerequisites.
    Auto,
}

impl ExecutionMode {
    /// The `snake_case` spelling used in `risk.toml` and the API.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::SemiAuto => "semi_auto",
            Self::Auto => "auto",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Risk {
        execution_mode: ExecutionMode,
    }

    #[test]
    fn default_is_paper() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::Paper);
    }

    #[test]
    fn parses_every_spelling_and_round_trips_with_as_str() {
        for mode in [
            ExecutionMode::Paper,
            ExecutionMode::SemiAuto,
            ExecutionMode::Auto,
        ] {
            let row: Risk =
                toml::from_str(&format!("execution_mode = \"{}\"", mode.as_str())).unwrap();
            assert_eq!(row.execution_mode, mode);
        }
    }

    #[test]
    fn rejects_unknown_and_wrong_case() {
        assert!(toml::from_str::<Risk>("execution_mode = \"live\"").is_err());
        assert!(toml::from_str::<Risk>("execution_mode = \"Paper\"").is_err());
        assert!(toml::from_str::<Risk>("execution_mode = true").is_err());
    }
}
