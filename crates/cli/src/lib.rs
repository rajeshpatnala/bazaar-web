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
    /// The history checkpoint that protects the activity history (ADR 0003).
    #[command(name = "history-checkpoint", subcommand)]
    AuditAnchor(AuditAnchorCommand),
}

/// `history-checkpoint` subcommands.
#[derive(Debug, Subcommand)]
pub enum AuditAnchorCommand {
    /// HUMAN-ONLY: check the activity history and write the history checkpoint for its last
    /// entry (new database, restored backup). Any existing history checkpoint, even a matching
    /// one, is replaced only with `--accept-missing-entries`.
    ///
    /// Stop the server first: no lock is taken. With no history checkpoint the current last
    /// entry is accepted on trust, so check the printed entry number and time against your own
    /// records. No audit row is written; the printed report is the only trail.
    #[command(name = "accept-current")]
    Adopt {
        /// Replace any existing history checkpoint, even a matching one (it may show that
        /// history entries are missing).
        #[arg(long = "accept-missing-entries")]
        accept_rollback: bool,
    },
    /// Check the activity history against the history checkpoint and print the checkpointed
    /// entry number.
    ///
    /// It opens the store like the app does, so it may run a migration or write the
    /// crash-window catch-up: it is not strictly read-only.
    #[command(name = "verify")]
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
    fn help_uses_the_new_names_and_keeps_the_warnings() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let group = cmd.find_subcommand("history-checkpoint").unwrap();
        let accept = group.find_subcommand("accept-current").unwrap();
        let help = accept.get_long_about().unwrap().to_string();
        for want in [
            "HUMAN-ONLY",
            "Stop the server first",
            "No audit row is written",
            "check the printed entry number and time",
            "even a matching",
            "activity history",
        ] {
            assert!(help.contains(want), "{want}: {help}");
        }
        let flag = accept
            .get_arguments()
            .find(|a| a.get_long() == Some("accept-missing-entries"))
            .unwrap();
        let flag = flag.get_help().unwrap().to_string();
        assert!(flag.contains("even a matching one"), "{flag}");
        let verify = group.find_subcommand("verify").unwrap();
        let help = verify.get_long_about().unwrap().to_string();
        assert!(help.contains("not strictly read-only"), "{help}");
        assert!(help.contains("activity history"), "{help}");
        for mut c in [cmd.clone(), group.clone(), accept.clone(), verify.clone()] {
            let text = c.render_long_help().to_string();
            assert!(!text.contains("anchor"), "{text}");
        }
    }

    #[test]
    fn accept_current_defaults_to_keeping_an_existing_checkpoint() {
        let parse = |extra: &[&str]| {
            let args = ["bazaar-cli", "history-checkpoint", "accept-current"];
            Cli::try_parse_from(args.iter().chain(extra))
                .unwrap()
                .command
        };
        assert!(matches!(
            parse(&[]),
            Some(Command::AuditAnchor(AuditAnchorCommand::Adopt {
                accept_rollback: false
            }))
        ));
        assert!(matches!(
            parse(&["--accept-missing-entries"]),
            Some(Command::AuditAnchor(AuditAnchorCommand::Adopt {
                accept_rollback: true
            }))
        ));
        let c = Cli::try_parse_from(["bazaar-cli", "history-checkpoint", "verify"]).unwrap();
        assert!(matches!(
            c.command,
            Some(Command::AuditAnchor(AuditAnchorCommand::Status))
        ));
        assert!(Cli::try_parse_from(["bazaar-cli", "history-checkpoint"]).is_err());
    }

    /// No aliases: every old command and flag fails to parse, alone or mixed with new names.
    #[test]
    fn old_commands_and_flags_no_longer_parse() {
        for old in [
            &["audit-anchor", "status"][..],
            &["audit-anchor", "adopt"],
            &["audit-anchor", "adopt", "--accept-rollback"],
            &["audit-anchor", "verify"],
            &["audit-anchor", "accept-current"],
            &["history-checkpoint", "status"],
            &["history-checkpoint", "adopt"],
            &["history-checkpoint", "accept-current", "--accept-rollback"],
        ] {
            let args = std::iter::once(&"bazaar-cli").chain(old);
            assert!(Cli::try_parse_from(args).is_err(), "{old:?}");
        }
    }

    #[test]
    fn rejects_unknown_flags() {
        assert!(Cli::try_parse_from(["bazaar-cli", "--mode", "auto"]).is_err());
    }
}
