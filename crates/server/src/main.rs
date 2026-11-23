//! `bazaar-server` binary: `serve` (loopback only) and `openapi`.

use std::{io::Write as _, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Context as _;
use bazaar_cli::commands::DB_FILE;
use bazaar_core::{calendar::MarketCalendar, clock::SystemClock, config::Config};
use bazaar_server::{
    app, runtime,
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
    Serve {
        /// Loopback port (overrides `BAZAAR_PORT`; default 8787). The host is never configurable.
        #[arg(long)]
        port: Option<u16>,
    },
}

/// Port: `--port`, else `BAZAAR_PORT`, else the default. Port 0 is rejected (the Host/Origin
/// allowlist and launch URL are built from the port, so an OS-picked port would never match).
fn resolve_port(flag: Option<u16>, env: Option<&str>) -> anyhow::Result<u16> {
    let port = match (flag, env) {
        (Some(p), _) => p,
        (None, Some(s)) => s
            .trim()
            .parse::<u16>()
            .context("BAZAAR_PORT must be a port number")?,
        (None, None) => DEFAULT_PORT,
    };
    anyhow::ensure!(port != 0, "port 0 is not allowed");
    Ok(port)
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
    let child = std::process::Command::new(tool)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        // Reap the opener so it never lingers as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            tracing::info!("shutdown signal received");
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

async fn serve(port_flag: Option<u16>) -> anyhow::Result<()> {
    // Dev fallback file: absent is fine, malformed is an error.
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(e) if e.not_found() => {}
        Err(e) => return Err(anyhow::Error::new(e).context("reading dotenv fallback")),
    }
    init_tracing();
    let port = resolve_port(port_flag, std::env::var("BAZAAR_PORT").ok().as_deref())?;
    let config_dir = std::env::var_os("BAZAAR_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from("config"), PathBuf::from);
    let config = Config::load(&config_dir).context("loading config")?;
    let dev = cfg!(debug_assertions);
    let addr = bind_addr(port)?;
    let security = Security::new(port, dev)?;
    let clock: Arc<SystemClock> = Arc::new(SystemClock);
    let data_dir = std::env::var_os("BAZAAR_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from("data"), PathBuf::from);
    // SEC-138-1: a directory we create is owner-only (0700); a pre-existing one is never
    // chmod-ed, only warned about. The store restricts the database files to 0600.
    let existed = data_dir.exists();
    std::fs::create_dir_all(&data_dir).context("creating the data directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if existed {
            let mode = std::fs::metadata(&data_dir)
                .context("reading the data directory permissions")?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                tracing::warn!(
                    mode = format!("{:o}", mode & 0o777),
                    "data directory is group/world accessible"
                );
            }
        } else {
            std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
                .context("restricting the data directory")?;
        }
    }
    #[cfg(not(unix))]
    let _ = existed;
    let data_dir = std::fs::canonicalize(&data_dir).context("resolving the data directory")?;
    tracing::info!(data_dir = %data_dir.display(), "using data directory");
    let risk_toml =
        std::fs::read_to_string(config_dir.join("risk.toml")).context("reading risk.toml")?;
    // RISK-138-4: the pipeline and the app state must agree on the execution mode.
    let pipeline_mode =
        bazaar_core::config::parse_str::<bazaar_core::config::RiskConfig>(&risk_toml)
            .context("parsing risk.toml")?
            .mode
            .execution_mode;
    anyhow::ensure!(
        pipeline_mode == config.risk.mode.execution_mode,
        "execution mode differs between the pipeline and the loaded config"
    );
    let calendar = MarketCalendar::from_config(&config.calendar);
    // Store open, build, reload once, spawn. Any error exits non-zero BEFORE the port is bound
    // (RISK-133-3): nothing is served on a failed start.
    let rt = runtime::start(&data_dir.join(DB_FILE), &risk_toml, clock.clone(), calendar)
        .await
        .context("starting the order pipeline")?;
    tracing::info!(report = ?rt.report, "pipeline reloaded");
    let state = AppState::new(
        security.clone(),
        clock,
        config.risk.mode.execution_mode,
        PathBuf::from(WEB_DIST),
    )
    .with_pipeline(rt.handle.clone(), rt.kill_switch.clone());
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
        Command::Serve { port } => serve(port).await,
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
    fn port_resolution_order_and_zero() {
        assert_eq!(resolve_port(None, None).unwrap(), 8787);
        assert_eq!(resolve_port(None, Some(" 9000 ")).unwrap(), 9000);
        assert_eq!(resolve_port(Some(9100), Some("9000")).unwrap(), 9100);
        assert!(resolve_port(None, Some("abc")).is_err());
        assert!(resolve_port(None, Some("70000")).is_err());
        assert!(resolve_port(None, Some("0")).is_err());
        assert!(resolve_port(Some(0), None).is_err());
    }

    #[test]
    fn cli_parses_both_subcommands() {
        assert!(Cli::try_parse_from(["bazaar-server", "openapi"]).is_ok());
        assert!(Cli::try_parse_from(["bazaar-server", "serve"]).is_ok());
        assert!(Cli::try_parse_from(["bazaar-server", "serve", "--port", "9000"]).is_ok());
        assert!(Cli::try_parse_from(["bazaar-server", "serve", "--host", "0.0.0.0"]).is_err());
    }
}
