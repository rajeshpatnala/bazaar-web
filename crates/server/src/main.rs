//! `bazaar-server` binary: `openapi` subcommand only until F1 slice 7 wires config, tracing and
//! the loopback bind.

use std::io::Write as _;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bazaar-server", version, about = "Bazaar server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the `OpenAPI` document (needs no config, secrets, data dir or socket).
    Openapi,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Openapi => {
            let json = bazaar_server::openapi::document().to_pretty_json()?;
            writeln!(std::io::stdout().lock(), "{json}")?;
        }
    }
    Ok(())
}
