//! The SpectrumIQ service: the instrument's Things and routes, served over HTTP until Ctrl-C or the
//! system asks it to stop.
//!
//! ```text
//! spectrumiq-server [run] -c config/sim.toml   serve the instrument (run is the default)
//!     [--port P] [--webapp-dir web/dist]      on another port; the web app from a folder
//! spectrumiq-server --print-openapi            the Things' OpenAPI document, for client types
//! spectrumiq-server --print-config-schema      the configuration's JSON Schema
//! spectrumiq-server --list-ports               the serial ports, for [stage] and [focus] port
//! spectrumiq-server user add <name> [--role R] -c config/sim.toml
//!                   user passwd|remove <name>, user role <name> <role>, user list
//!                                              the users of `[server] settings` (ADR-0065)
//! spectrumiq-server audit verify -c config/sim.toml
//!                                              the audit trail's chain checked (ADR-0067)
//! ```
//!
//! It listens where `[server]` says, 127.0.0.1:5000 by default, and serves the web app at `/`:
//! the build compiled in, or the folder `--webapp-dir` gives. Shutting down stops live view, then
//! motion, then the Things.
//!
//! While there is no admin, the user added is one, to manage the rest; a password is asked for
//! twice, without being shown, or read as two lines of standard input from a pipe.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use siq_auth::cli::UserCommand;
use siq_auth::{Role, USERS_FILE, UserStore};
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
    /// List this computer's serial ports, for the stage's and the focus drive's `port`, and exit.
    #[arg(long)]
    list_ports: bool,
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
    /// Manage the users, in `users.json` in `[server] settings`.
    User {
        #[command(subcommand)]
        command: UserArgs,
    },
    /// The audit trail of `[server] audit`.
    Audit {
        #[command(subcommand)]
        command: AuditArgs,
    },
}

#[derive(Subcommand)]
enum AuditArgs {
    /// Check that no entry was edited, removed or moved: exit code 1 where one was.
    Verify,
}

#[derive(Subcommand)]
enum UserArgs {
    /// Add a user: an admin if it is the first, a viewer unless --role says otherwise.
    Add {
        /// Its name: letters, digits, '.', '_' or '-'.
        name: String,
        /// viewer, operator or admin.
        #[arg(long)]
        role: Option<Role>,
    },
    /// Change a user's password.
    Passwd {
        /// Its name.
        name: String,
    },
    /// Change a user's role.
    Role {
        /// Its name.
        name: String,
        /// viewer, operator or admin.
        role: Role,
    },
    /// List the users.
    List,
    /// Remove a user. The last admin stays.
    Remove {
        /// Its name.
        name: String,
    },
}

impl From<UserArgs> for UserCommand {
    fn from(args: UserArgs) -> Self {
        match args {
            UserArgs::Add { name, role } => Self::Add { name, role },
            UserArgs::Passwd { name } => Self::Passwd { name },
            UserArgs::Role { name, role } => Self::Role { name, role },
            UserArgs::List => Self::List,
            UserArgs::Remove { name } => Self::Remove { name },
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.print_config_schema {
        println!("{:#}", siq_config::json_schema());
        return Ok(());
    }
    if cli.list_ports {
        for port in siq_serial::ports()? {
            println!("{:<8} {}", port.name, port.description);
        }
        return Ok(());
    }
    if cli.print_openapi {
        println!("{:#}", siq_server::openapi().await?);
        return Ok(());
    }
    let path = cli
        .config
        .context("the configuration is missing: give -c <config.toml>")?;
    let config = siq_config::load(&path)?;
    match cli.command {
        Some(Command::User { command }) => {
            let store = UserStore::open(config.server.settings.join(USERS_FILE))?;
            let mut ask = siq_auth::read_password;
            siq_auth::cli::run(&store, &command.into(), &mut ask, &mut std::io::stdout())?;
            return Ok(());
        }
        Some(Command::Audit {
            command: AuditArgs::Verify,
        }) => {
            let folder = &config.server.audit;
            let verified = siq_auth::audit::verify(folder)
                .with_context(|| format!("the audit trail in {}", folder.display()))?;
            println!(
                "{} entries in {} file(s), the chain intact; its last hash {}",
                verified.entries, verified.files, verified.head
            );
            return Ok(());
        }
        Some(Command::Run) | None => {}
    }
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
