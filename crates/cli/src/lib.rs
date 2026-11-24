#![cfg_attr(test, allow(clippy::unwrap_used))]

//! Headless composition root (clap): check-connections, imports, backtest.
//!
//! F2 adds the two offline subcommands `import-instruments` and `check-connections`.

pub mod commands;
pub mod compose;
pub mod costs;
pub mod specs;

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
    /// The audit anchor beside the database (ADR 0003).
    #[command(subcommand)]
    AuditAnchor(AuditAnchorCommand),
}

/// `audit-anchor` subcommands.
#[derive(Debug, Subcommand)]
pub enum AuditAnchorCommand {
    /// HUMAN-ONLY: verify the audit chain and write the anchor for its tail (new database,
    /// restored backup). Replacing an existing anchor needs `--accept-rollback`.
    ///
    /// Stop the server first: no lock is taken. A missing anchor adopts the current tail on
    /// trust, so check the printed tail seq and time against your own records. No audit row is
    /// written; the printed report is the only trail.
    Adopt {
        /// Replace an existing anchor file (it may be ahead of the log: rows would be lost).
        #[arg(long)]
        accept_rollback: bool,
    },
    /// Verify the chain and the anchor and print the anchored seq.
    ///
    /// It opens the store like the app does, so it may run a migration or write the
    /// crash-window catch-up: it is not strictly read-only.
    Status,
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
    fn adopt_help_says_stop_the_server_and_check_the_tail() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let anchor = cmd.find_subcommand("audit-anchor").unwrap();
        let adopt = anchor.find_subcommand("adopt").unwrap();
        let help = adopt.get_long_about().unwrap().to_string();
        assert!(help.contains("Stop the server first"), "{help}");
        assert!(help.contains("check the printed tail seq"), "{help}");
        let status = anchor.find_subcommand("status").unwrap();
        let help = status.get_long_about().unwrap().to_string();
        assert!(help.contains("not strictly read-only"), "{help}");
    }

    #[test]
    fn audit_anchor_adopt_defaults_to_no_rollback() {
        let c = Cli::try_parse_from(["bazaar-cli", "audit-anchor", "adopt"]).unwrap();
        assert!(matches!(
            c.command,
            Some(Command::AuditAnchor(AuditAnchorCommand::Adopt {
                accept_rollback: false
            }))
        ));
        let c = Cli::try_parse_from(["bazaar-cli", "audit-anchor", "adopt", "--accept-rollback"])
            .unwrap();
        assert!(matches!(
            c.command,
            Some(Command::AuditAnchor(AuditAnchorCommand::Adopt {
                accept_rollback: true
            }))
        ));
        assert!(Cli::try_parse_from(["bazaar-cli", "audit-anchor"]).is_err());
    }

    #[test]
    fn rejects_unknown_flags() {
        assert!(Cli::try_parse_from(["bazaar-cli", "--mode", "auto"]).is_err());
    }
}
