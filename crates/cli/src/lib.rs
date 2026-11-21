#![cfg_attr(test, allow(clippy::unwrap_used))]

//! Headless composition root (clap): check-connections, imports, backtest.
//!
//! F2 adds the two offline subcommands `import-instruments` and `check-connections`.

pub mod commands;
pub mod compose;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    /// What to run; with none, only the shared arguments are parsed.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check the SQLite store and the freshness of the instrument master.
    CheckConnections,
    /// Import a Kite-format instruments CSV file into the store (no network).
    ImportInstruments {
        /// Path of the CSV file.
        #[arg(long)]
        file: PathBuf,
    },
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
    fn parses_subcommands() {
        let c = Cli::try_parse_from(["bazaar-cli", "check-connections"]).unwrap();
        assert!(matches!(c.command, Some(Command::CheckConnections)));
        let c =
            Cli::try_parse_from(["bazaar-cli", "import-instruments", "--file", "a.csv"]).unwrap();
        assert!(matches!(c.command, Some(Command::ImportInstruments { .. })));
        assert!(Cli::try_parse_from(["bazaar-cli", "import-instruments"]).is_err());
    }

    #[test]
    fn rejects_unknown_flags() {
        assert!(Cli::try_parse_from(["bazaar-cli", "--mode", "auto"]).is_err());
    }
}
