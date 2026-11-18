//! `bazaar-cli` binary: parses the arguments and runs the offline subcommands.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: stdout/stderr are its interface

use std::process::ExitCode;

use bazaar_cli::commands::{check_connections, import_instruments};
use bazaar_cli::{Cli, Command};
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
    }
}
