#![cfg_attr(test, allow(clippy::unwrap_used))]

//! Headless composition root (clap): check-connections, imports, backtest.
//!
//! F1 ships the argument shape only; subcommands arrive with the features that own them.

use std::path::PathBuf;

use clap::Parser;

/// Command line of `bazaar-cli`.
#[derive(Debug, Parser)]
#[command(
    name = "bazaar-cli",
    version,
    about = "Bazaar headless tools (paper trading by default)"
)]
pub struct Cli {
    /// Directory holding the config/*.toml files.
    #[arg(long, global = true, default_value = "config")]
    pub config_dir: PathBuf,
    /// Directory for runtime data (SQLite, Parquet); defaults to `data`.
    #[arg(long, global = true, default_value = "data")]
    pub data_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_repo_relative() {
        let cli = Cli::try_parse_from(["bazaar-cli"]).unwrap();
        assert_eq!(cli.config_dir, PathBuf::from("config"));
        assert_eq!(cli.data_dir, PathBuf::from("data"));
    }

    #[test]
    fn accepts_fixture_dirs() {
        let cli = Cli::try_parse_from([
            "bazaar-cli",
            "--config-dir",
            "fixtures/config",
            "--data-dir",
            "/tmp/x",
        ])
        .unwrap();
        assert_eq!(cli.config_dir, PathBuf::from("fixtures/config"));
        assert_eq!(cli.data_dir, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn rejects_unknown_flags() {
        assert!(Cli::try_parse_from(["bazaar-cli", "--mode", "auto"]).is_err());
    }
}
