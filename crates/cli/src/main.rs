//! `bazaar-cli` binary: parses the arguments and runs the offline subcommands.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: stdout/stderr are its interface

use std::process::ExitCode;

use bazaar_cli::commands::{
    AdoptOutcome, AnchorReport, CommandError, audit_anchor_adopt, audit_anchor_status,
    check_connections, import_instruments,
};
use bazaar_cli::{AuditAnchorCommand, Cli, Command};
use bazaar_core::clock::SystemClock;
use bazaar_data::instruments::ParseLimits;
use bazaar_store::AnchorFault;
use clap::Parser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None => ExitCode::SUCCESS,
        Some(Command::CheckConnections) => {
            let report = check_connections(&cli.config_dir, &cli.data_dir, &SystemClock).await;
            for line in &report.lines {
                println!("{line}");
            }
            if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some(Command::ImportInstruments { file }) => {
            match import_instruments(&cli.data_dir, &file, &SystemClock, &ParseLimits::default())
                .await
            {
                Ok(s) => {
                    println!(
                        "imported {} rows ({} accepted, {} rejected)",
                        s.written, s.accepted, s.rejected
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("import failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(Command::AuditAnchor(AuditAnchorCommand::Adopt { accept_rollback })) => {
            match audit_anchor_adopt(&cli.data_dir, accept_rollback).await {
                Ok(o) => {
                    for line in accept_success_lines(&o) {
                        println!("{line}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", accept_failure_text(e));
                    ExitCode::FAILURE
                }
            }
        }
        Some(Command::AuditAnchor(AuditAnchorCommand::Status)) => {
            match audit_anchor_status(&cli.data_dir).await {
                Ok(r) => {
                    println!("{}", verify_success_text(r));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", verify_failure_text(e));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Claims only what a refusal guarantees: opening the store may still migrate the database.
const NOT_CHANGED: &str = "The activity history and the history checkpoint were not changed";

/// Next step when the database cannot be found or used.
const CHECK_AND_RERUN: &str =
    "Check --data-dir, permissions, and that the server is stopped, then run again.";

/// The old checkpoint in plain words. The store reports no fault both for a checkpoint that
/// matched and for a missing one; only the former has an old entry number.
fn old_checkpoint_text(old_seq: Option<u64>, fault: Option<AnchorFault>) -> &'static str {
    match fault {
        None if old_seq.is_some() => "the old checkpoint matched",
        None | Some(AnchorFault::Missing) => "there was no checkpoint",
        Some(AnchorFault::Oversized | AnchorFault::Unparsable) => {
            "the checkpoint file was not a valid checkpoint"
        }
        Some(AnchorFault::Ahead) => {
            "the checkpoint was past the end of the history (entries may be missing)"
        }
        Some(AnchorFault::HashMismatch) => {
            "the history did not match the checkpoint (changed or replaced)"
        }
        Some(AnchorFault::Io) => "the checkpoint file could not be read",
    }
}

fn accept_success_lines(o: &AdoptOutcome) -> [String; 5] {
    let r = o.report;
    let old = r.old_seq.map_or("none".to_owned(), |s| s.to_string());
    let at = o.tail_ts.as_deref().unwrap_or("none (empty history)");
    [
        format!("Old entry number: {old}"),
        format!(
            "Old checkpoint: {}",
            old_checkpoint_text(r.old_seq, r.old_fault)
        ),
        format!("New entry number: {}", r.new_seq),
        format!("Last entry time: {at}"),
        "Check the new entry number and time against your own records.".to_owned(),
    ]
}

/// Only a refusal raised before any write may say "Not changed": an unavailable or other store
/// error can arrive after the checkpoint was written (late write, or the read for the report).
fn accept_failure_text(e: CommandError) -> String {
    match e {
        CommandError::Store("conflict") => format!(
            "{NOT_CHANGED}: a history checkpoint already exists. Replacing it can hide missing \
             history entries. Compare the entry number and time with your own records, then run \
             again with --accept-missing-entries."
        ),
        CommandError::Store("corrupt") => format!(
            "{NOT_CHANGED}: the activity history does not pass its check. Do not trade on this \
             data; restore a backup, then run verify."
        ),
        CommandError::EmptyHistory => format!(
            "{NOT_CHANGED}: the activity history is empty but a history checkpoint exists, which \
             accept-current cannot replace (entries may be missing). Do not trade on this data; \
             restore a backup, then run verify."
        ),
        CommandError::NoDatabase => {
            format!("{NOT_CHANGED}: no database in the data directory. {CHECK_AND_RERUN}")
        }
        CommandError::Store(label) | CommandError::AfterAdopt(label) => format!(
            "accept-current did not finish: store error ({label}). Run verify to see what is \
             stored. {CHECK_AND_RERUN}"
        ),
        other => format!("accept-current did not finish: {other}. {CHECK_AND_RERUN}"),
    }
}

fn verify_success_text(r: AnchorReport) -> String {
    if r.lag {
        format!(
            "History check passed, but the history checkpoint is out of date (entry number {}). \
             Check free disk space and data folder permissions, then run verify again.",
            r.seq
        )
    } else {
        format!(
            "History check passed: history checkpoint at entry number {}.",
            r.seq
        )
    }
}

fn verify_failure_text(e: CommandError) -> String {
    match e {
        CommandError::Store("corrupt") => format!(
            "History check failed: the activity history and the history checkpoint do not agree. \
             {NOT_CHANGED}. Compare with your own records or restore a backup before you consider \
             accept-current."
        ),
        CommandError::NoDatabase => {
            format!("History check not run: no database in the data directory. {CHECK_AND_RERUN}")
        }
        CommandError::Store(label) => {
            format!("History check did not finish: store error ({label}). {CHECK_AND_RERUN}")
        }
        other => format!("History check did not finish: {other}. {CHECK_AND_RERUN}"),
    }
}

#[cfg(test)]
mod tests {
    use bazaar_store::AdoptReport;

    use super::*;

    #[test]
    fn every_checkpoint_fault_has_plain_words() {
        let invalid = "the checkpoint file was not a valid checkpoint";
        for (seq, fault, want) in [
            (Some(3), None, "the old checkpoint matched"),
            (None, None, "there was no checkpoint"),
            (None, Some(AnchorFault::Missing), "there was no checkpoint"),
            (None, Some(AnchorFault::Oversized), invalid),
            (None, Some(AnchorFault::Unparsable), invalid),
            (
                Some(9),
                Some(AnchorFault::Ahead),
                "the checkpoint was past the end of the history (entries may be missing)",
            ),
            (
                Some(2),
                Some(AnchorFault::HashMismatch),
                "the history did not match the checkpoint (changed or replaced)",
            ),
            (
                None,
                Some(AnchorFault::Io),
                "the checkpoint file could not be read",
            ),
        ] {
            assert_eq!(old_checkpoint_text(seq, fault), want, "{seq:?} {fault:?}");
        }
    }

    #[test]
    fn success_lists_old_and_new_then_asks_for_a_check() {
        let o = AdoptOutcome {
            report: AdoptReport {
                old_seq: Some(9),
                old_fault: Some(AnchorFault::Ahead),
                new_seq: 2,
            },
            tail_ts: Some("2026-11-25T09:15:00+05:30".to_owned()),
        };
        assert_eq!(
            accept_success_lines(&o),
            [
                "Old entry number: 9",
                "Old checkpoint: the checkpoint was past the end of the history (entries may be \
                 missing)",
                "New entry number: 2",
                "Last entry time: 2026-11-25T09:15:00+05:30",
                "Check the new entry number and time against your own records.",
            ]
        );
        let empty = AdoptOutcome {
            report: AdoptReport {
                old_seq: None,
                old_fault: None,
                new_seq: 0,
            },
            tail_ts: None,
        };
        let lines = accept_success_lines(&empty);
        assert_eq!(lines[0], "Old entry number: none");
        assert_eq!(lines[1], "Old checkpoint: there was no checkpoint");
        assert_eq!(lines[3], "Last entry time: none (empty history)");
    }

    #[test]
    fn accept_refusals_say_not_changed_and_the_next_step() {
        assert_eq!(
            accept_failure_text(CommandError::Store("conflict")),
            "The activity history and the history checkpoint were not changed: a history \
             checkpoint already exists. Replacing it can hide missing history entries. Compare the \
             entry number and time with your own records, then run again with \
             --accept-missing-entries."
        );
        assert_eq!(
            accept_failure_text(CommandError::Store("corrupt")),
            "The activity history and the history checkpoint were not changed: the activity \
             history does not pass its check. Do not trade on this data; restore a backup, then \
             run verify."
        );
        assert_eq!(
            accept_failure_text(CommandError::EmptyHistory),
            "The activity history and the history checkpoint were not changed: the activity \
             history is empty but a history checkpoint exists, which accept-current cannot \
             replace (entries may be missing). Do not trade on this data; restore a backup, then \
             run verify."
        );
        let none = accept_failure_text(CommandError::NoDatabase);
        assert!(
            none.starts_with(&format!(
                "{NOT_CHANGED}: no database in the data directory."
            )),
            "{none}"
        );
        assert!(none.ends_with(CHECK_AND_RERUN), "{none}");
    }

    /// The write may have landed, so these never claim that nothing changed.
    #[test]
    fn accept_store_errors_name_the_category_without_claiming_no_change() {
        use CommandError::{AfterAdopt, Store};
        let errors = ["unavailable", "schema", "too_large", "other"]
            .map(|l| (Store(l), l))
            .into_iter()
            .chain(["corrupt", "conflict"].map(|l| (AfterAdopt(l), l)));
        for (e, label) in errors {
            let t = accept_failure_text(e);
            assert!(t.contains(label), "{t}");
            assert!(!t.contains("not changed"), "{t}");
            assert!(t.contains("Run verify to see what is stored."), "{t}");
            assert!(t.ends_with(CHECK_AND_RERUN), "{t}");
        }
    }

    #[test]
    fn verify_failures_keep_their_category_and_say_the_next_step() {
        assert_eq!(
            verify_failure_text(CommandError::Store("corrupt")),
            "History check failed: the activity history and the history checkpoint do not agree. \
             The activity history and the history checkpoint were not changed. Compare with your \
             own records or restore a backup before you consider accept-current."
        );
        let none = verify_failure_text(CommandError::NoDatabase);
        assert!(none.contains("no database"), "{none}");
        assert!(none.ends_with(CHECK_AND_RERUN), "{none}");
        for label in ["unavailable", "schema", "other"] {
            let t = verify_failure_text(CommandError::Store(label));
            assert!(t.contains(label), "{t}");
            assert!(!t.contains("accept-current"), "{t}");
            assert!(t.ends_with(CHECK_AND_RERUN), "{t}");
        }
    }

    #[test]
    fn verify_success_reports_the_entry_number_and_a_lag_with_its_next_step() {
        assert_eq!(
            verify_success_text(AnchorReport { seq: 3, lag: false }),
            "History check passed: history checkpoint at entry number 3."
        );
        let t = verify_success_text(AnchorReport { seq: 3, lag: true });
        assert!(t.contains("out of date (entry number 3)"), "{t}");
        assert!(t.contains("then run verify again"), "{t}");
    }
}
