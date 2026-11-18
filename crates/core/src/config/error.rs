//! Config errors. They carry the file name, a key path and a reason, never file contents,
//! so logging one cannot leak anything.

use std::fmt;

/// Why a config file could not be loaded or validated.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read.
    Io {
        file: &'static str,
        source: std::io::Error,
    },
    /// The file is not valid TOML for its schema (unknown key, wrong type, TOML float for money).
    Parse {
        file: &'static str,
        line: Option<usize>,
        message: String,
    },
    /// The file parsed but a value is out of range or inconsistent.
    Invalid {
        file: &'static str,
        key: String,
        reason: String,
    },
}

impl ConfigError {
    pub(crate) fn invalid(
        file: &'static str,
        key: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::Invalid {
            file,
            key: key.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { file, source } => write!(f, "{file}: cannot read file ({})", source.kind()),
            Self::Parse {
                file,
                line: Some(line),
                message,
            } => write!(f, "{file}: line {line}: {message}"),
            Self::Parse {
                file,
                line: None,
                message,
            } => write!(f, "{file}: {message}"),
            Self::Invalid { file, key, reason } => write!(f, "{file}: {key}: {reason}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
