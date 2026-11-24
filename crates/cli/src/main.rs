//! `bazaar-cli` binary: parses the arguments and runs the offline subcommands.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: stdout/stderr are its interface

use std::process::ExitCode;

use bazaar_cli::commands::{
    CommandError, audit_anchor_adopt, audit_anchor_status, check_connections, import_instruments,
};
use bazaar_cli::{AuditAnchorCommand, Cli, Command};
use bazaar_core::clock::SystemClock;
use bazaar_data::instruments::ParseLimits;
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
                    let r = o.report;
                    let old = r.old_seq.map_or("none".to_owned(), |s| s.to_string());
                    let fault = r.old_fault.map_or("none".to_owned(), |f| format!("{f:?}"));
                    let at = o.tail_ts.as_deref().unwrap_or("empty log");
                    println!(
                        "anchor adopted: old seq {old} (fault: {fault}), new seq {} (last row at {at})",
                        r.new_seq
                    );
                    println!("check the new seq and time against your own records");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    match e {
                        CommandError::Store("conflict") => {
                            eprintln!("adopt refused: an anchor file exists");
                            eprintln!(
                                "replacing it can hide cut rows. Check it, then pass --accept-rollback"
                            );
                        }
                        CommandError::Store("corrupt") => {
                            eprintln!(
                                "adopt refused: the audit chain does not verify; nothing written"
                            );
                        }
                        CommandError::NoDatabase => {
                            eprintln!("adopt failed: no database in the data directory");
                        }
                        _ => eprintln!("adopt failed: {e}"),
                    }
                    ExitCode::FAILURE
                }
            }
        }
        Some(Command::AuditAnchor(AuditAnchorCommand::Status)) => {
            match audit_anchor_status(&cli.data_dir).await {
                Ok(r) => {
                    println!("anchor ok: seq {}, lag {}", r.seq, r.lag);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("anchor check failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
