//! The SpectrumIQ desktop shell (ADR-0040): a Tauri 2 window around the web app that the local
//! service serves.
//!
//! The window opens on a bundled connect page, which probes the service's health and then loads
//! it from `http://127.0.0.1:<port>/`. The web app reaches the shell's host commands (app info,
//! the save dialog, links, and the development server) through a capability for that origin
//! alone; image data never crosses the IPC (ADR-0002).

pub mod server;

#[cfg(debug_assertions)]
mod child;

use std::time::Duration;

use serde::Serialize;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Manager, RunEvent, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::server::Health;

/// How long the connect page's probe waits for an answer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// The shell's state: where the service is, and in development builds, the one it started.
struct Shell {
    /// The service's base URL, or why the one given can't be used.
    server: Result<Url, String>,
    #[cfg(debug_assertions)]
    child: child::DevServer,
}

impl Shell {
    fn server(&self) -> Result<&Url, String> {
        self.server.as_ref().map_err(Clone::clone)
    }
}

/// What the web app and the connect page learn about the shell.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    /// The shell's version.
    version: &'static str,
    /// The service's base URL.
    server: Option<String>,
    /// Whether the shell can start the service: in development builds only.
    manages_service: bool,
}

#[tauri::command]
fn app_info(shell: State<'_, Shell>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        server: shell.server.as_ref().ok().map(Url::to_string),
        manages_service: cfg!(debug_assertions),
    }
}

#[tauri::command]
async fn probe(shell: State<'_, Shell>) -> Result<Health, String> {
    let server = shell.server()?;
    server::probe(server, PROBE_TIMEOUT)
        .await
        .map_err(|error| error.to_string())
}

/// Opens a web or mail link in the system's browser or mail program.
#[tauri::command]
fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|error| error.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err(format!("{url} isn't a web or mail link"));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| error.to_string())
}

/// Saves the request's body, raw bytes rather than JSON, through a save dialog proposing the
/// name in its `x-file-name` header (URI-encoded). False if the dialog was cancelled.
#[tauri::command]
async fn save_file(app: AppHandle, request: Request<'_>) -> Result<bool, String> {
    let InvokeBody::Raw(data) = request.body() else {
        return Err("the file comes as raw bytes".into());
    };
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .map(|name| {
            percent_encoding::percent_decode_str(name)
                .decode_utf8_lossy()
                .into_owned()
        })
        .unwrap_or_default();
    let Some(path) = app.dialog().file().set_file_name(name).blocking_save_file() else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|error| error.to_string())?;
    std::fs::write(&path, data).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(true)
}

#[cfg(debug_assertions)]
#[tauri::command]
async fn service_state(shell: State<'_, Shell>) -> Result<child::State, String> {
    let healthy = probe(shell.clone()).await.is_ok();
    Ok(shell.child.state(healthy))
}

#[cfg(debug_assertions)]
#[tauri::command]
fn service_start(shell: State<'_, Shell>) -> Result<(), String> {
    shell.child.start(shell.server()?)
}

#[cfg(debug_assertions)]
#[tauri::command]
fn service_stop(shell: State<'_, Shell>) {
    shell.child.stop();
}

#[cfg(debug_assertions)]
#[tauri::command]
fn service_restart(shell: State<'_, Shell>) -> Result<(), String> {
    shell.child.stop();
    shell.child.start(shell.server()?)
}

/// Opens the shell's window, and runs until it closes.
pub fn run() {
    let server = server::configured(
        std::env::args().skip(1),
        std::env::var("SPECTRUMIQ_SERVER").ok(),
    );
    let shell = Shell {
        server: server.map_err(|error| error.to_string()),
        #[cfg(debug_assertions)]
        child: child::DevServer::default(),
    };
    tauri::Builder::default()
        // First, as the plugin asks: a second launch shows this window instead.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(shell)
        .invoke_handler(handlers())
        .build(tauri::generate_context!())
        .expect("the shell's window opens")
        .run(|_app, _event: RunEvent| {
            #[cfg(debug_assertions)]
            if let RunEvent::Exit = _event {
                _app.state::<Shell>().child.stop();
            }
        });
}

#[cfg(debug_assertions)]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        app_info,
        probe,
        open_link,
        save_file,
        service_state,
        service_start,
        service_stop,
        service_restart
    ]
}

#[cfg(not(debug_assertions))]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![app_info, probe, open_link, save_file]
}
