//! The SpectrumIQ service: the instrument's Things and routes, served over HTTP until Ctrl-C or the
//! system asks it to stop.
//!
//! ```text
//! spectrumiq-server [run] -c config/sim.toml   serve the instrument (run is the default)
//!     [--port P] [--webapp-dir web/dist]      on another port; the web app from a folder
//! spectrumiq-server --print-openapi            the Things' OpenAPI document, for client types
//! spectrumiq-server --print-config-schema      the configuration's JSON Schema
//! ```
//!
//! It listens where `[server]` says, 127.0.0.1:5000 by default, and serves the web app at `/`:
//! the build compiled in, or the folder `--webapp-dir` gives. Shutting down stops live view, then
//! motion, then the Things.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use siq_server::webapp::WebApp;
use siq_server::{Service, shutdown_signal};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(version, about = "The SpectrumIQ service")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// The instrument configuration.
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    /// Print the Things' OpenAPI document, and exit.
    #[arg(long)]
    print_openapi: bool,
    /// Print the configuration's JSON Schema, and exit.
    #[arg(long)]
    print_config_schema: bool,
    /// Listen on this port instead of the configuration's.
    #[arg(long, global = true)]
    port: Option<u16>,
    /// Serve the web app from this folder, a build such as web/dist, instead of the one compiled in.
    #[arg(long, global = true)]
    webapp_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the instrument: the default.
    Run,
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.print_config_schema {
        println!("{:#}", siq_config::json_schema());
        return Ok(());
    }
    if cli.print_openapi {
        println!("{:#}", siq_server::openapi().await?);
        return Ok(());
    }
    let Command::Run = cli.command.unwrap_or(Command::Run);
    let path = cli
        .config
        .context("the configuration is missing: give -c <config.toml>")?;
    let config = siq_config::load(&path)?;
    let _logs = siq_telemetry::init(&config.log)?;
    let address = (config.server.bind, cli.port.unwrap_or(config.server.port));
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("listening on {address:?}"))?;
    let webapp = cli.webapp_dir.map_or(WebApp::Embedded, WebApp::Dir);
    let service = Service::build(config).await?.with_webapp(webapp);
    service.serve(listener, shutdown_signal()).await
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
