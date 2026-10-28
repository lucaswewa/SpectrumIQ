//! Tauri's build step: the app's context, its Windows resources, and a permission for each of
//! its commands, which the capabilities grant (`allow-app-info`, …).

fn main() {
    let commands = tauri_build::AppManifest::new().commands(&[
        "app_info",
        "probe",
        "local_secret",
        "open_link",
        "save_file",
        "service_state",
        "service_start",
        "service_stop",
        "service_restart",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands))
        .expect("Tauri's build step runs");
}
