//! In development builds only, the simulated instrument as the shell's child process:
//! `spectrumiq-server -c config/sim.toml`, from this repository's `target/` folder. In production,
//! the shell expects the Windows service (P15.1) and manages nothing.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;
use url::Url;

/// The repository this development build was built in.
const REPOSITORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../..");

/// A child service's state, as the web app's `ServiceState` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Not started, or stopped.
    Stopped,
    /// Started, and not answering yet.
    Starting,
    /// Answering its health route.
    Running,
    /// It exited by itself.
    Failed,
}

/// The simulated instrument the shell started, if any.
#[derive(Default)]
pub struct DevServer(Mutex<Option<Child>>);

impl DevServer {
    fn child(&self) -> MutexGuard<'_, Option<Child>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether it runs, and answers (`healthy`, from the caller's probe).
    pub fn state(&self, healthy: bool) -> State {
        let mut child = self.child();
        match child.as_mut().map(Child::try_wait) {
            None => State::Stopped,
            Some(Ok(None)) if healthy => State::Running,
            Some(Ok(None)) => State::Starting,
            Some(_) => State::Failed,
        }
    }

    /// Starts the server on `server`'s port, unless one runs already: the release build if there
    /// is one, which keeps up with the simulated camera, else the debug build.
    pub fn start(&self, server: &Url) -> Result<(), String> {
        let mut child = self.child();
        if let Some(running) = child.as_mut()
            && matches!(running.try_wait(), Ok(None))
        {
            return Ok(());
        }
        let repository = PathBuf::from(REPOSITORY);
        let built = ["release", "debug"].map(|profile| {
            repository
                .join("target")
                .join(profile)
                .join("spectrumiq-server.exe")
        });
        let program = built.iter().find(|path| path.is_file()).ok_or(
            "spectrumiq-server isn't built: run `cargo build --release -p spectrumiq-server` first",
        )?;
        let port = server.port_or_known_default().unwrap_or(5000).to_string();
        let started = Command::new(program)
            .args(["-c", "config/sim.toml", "--port", &port])
            .current_dir(&repository)
            .spawn()
            .map_err(|error| format!("{} didn't start: {error}", program.display()))?;
        *child = Some(started);
        Ok(())
    }

    /// Ends it, if it runs. It is killed, not asked to stop: this is for development only.
    pub fn stop(&self) {
        let mut child = self.child();
        if let Some(mut running) = child.take() {
            let _ = running.kill();
            let _ = running.wait();
        }
    }
}
