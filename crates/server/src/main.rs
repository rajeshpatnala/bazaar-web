//! `bazaar-server` binary: `serve` (loopback only) and `openapi`.

use std::{io::Write as _, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Context as _;
use bazaar_core::{clock::SystemClock, config::Config};
use bazaar_server::{
    app,
    security::{Security, ensure_loopback, loopback_addr},
    state::AppState,
};
use clap::{Parser, Subcommand};

/// Default port (matches `.env.example` and the Kite redirect URL).
const DEFAULT_PORT: u16 = 8787;
/// Fixed UI directory (SEC-22-1): never taken from a request, header or environment value.
const WEB_DIST: &str = "web/dist";

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
    /// Start the server on 127.0.0.1 and print the one-time launch URL.
    Serve,
}

/// Port from an optional operator-supplied value; the host is never configurable.
fn parse_port(raw: Option<&str>) -> anyhow::Result<u16> {
    raw.map_or(Ok(DEFAULT_PORT), |s| {
        s.trim()
            .parse::<u16>()
            .context("BAZAAR_PORT must be a port number")
    })
}

/// The bind address: loopback only, asserted before any socket is opened.
fn bind_addr(port: u16) -> anyhow::Result<SocketAddr> {
    Ok(ensure_loopback(loopback_addr(port))?)
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Best-effort browser open; failure is ignored (the URL is printed anyway).
fn open_browser(url: &str) {
    let tool = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(tool)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

async fn serve() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    init_tracing();
    let port = parse_port(std::env::var("BAZAAR_PORT").ok().as_deref())?;
    let config_dir = std::env::var_os("BAZAAR_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from("config"), PathBuf::from);
    let config = Config::load(&config_dir).context("loading config")?;
    let dev = cfg!(debug_assertions);
    let addr = bind_addr(port)?;
    let security = Security::new(port, dev)?;
    let state = AppState::new(
        security.clone(),
        Arc::new(SystemClock),
        config.risk.mode.execution_mode,
        PathBuf::from(WEB_DIST),
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!(%addr, mode = ?config.risk.mode.execution_mode, "listening");
    if let Some(url) = security.launch_url(port, dev) {
        // Printed once to stdout, never through tracing (it carries the token).
        writeln!(std::io::stdout().lock(), "Open Bazaar: {url}")?;
        open_browser(&url);
    }
    axum::serve(listener, app::build(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Openapi => {
            let json = bazaar_server::openapi::document().to_pretty_json()?;
            writeln!(std::io::stdout().lock(), "{json}")?;
            Ok(())
        }
        Command::Serve => serve().await,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn bind_is_loopback_only() {
        let a = bind_addr(8787).unwrap();
        assert_eq!(a.to_string(), "127.0.0.1:8787");
        assert!(a.ip().is_loopback());
    }

    #[test]
    fn port_defaults_and_parses() {
        assert_eq!(parse_port(None).unwrap(), 8787);
        assert_eq!(parse_port(Some(" 9000 ")).unwrap(), 9000);
        assert!(parse_port(Some("abc")).is_err());
        assert!(parse_port(Some("70000")).is_err());
    }

    #[test]
    fn cli_parses_both_subcommands() {
        assert!(Cli::try_parse_from(["bazaar-server", "openapi"]).is_ok());
        assert!(Cli::try_parse_from(["bazaar-server", "serve"]).is_ok());
        assert!(Cli::try_parse_from(["bazaar-server", "serve", "--host", "0.0.0.0"]).is_err());
    }

    #[test]
    fn web_dist_is_fixed() {
        assert_eq!(WEB_DIST, "web/dist");
    }
}
