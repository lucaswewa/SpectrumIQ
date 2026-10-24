//! Logging setup, in its own test binary: a process sets its global subscriber only once.

use siq_telemetry::{LogConfig, TelemetryError, init};

#[test]
fn logs_go_to_a_daily_file_and_logging_is_set_up_once() {
    let rust_log = std::env::var_os("RUST_LOG");
    let folder = tempfile::tempdir().unwrap();
    if rust_log.is_none() {
        let bad = LogConfig {
            filter: "info,=[".to_owned(),
            ..LogConfig::default()
        };
        assert!(matches!(init(&bad), Err(TelemetryError::Filter { .. })));
    }

    let config = LogConfig {
        console: false,
        directory: Some(folder.path().to_owned()),
        ..LogConfig::default()
    };
    let guard = init(&config).expect("the first set-up succeeds");
    tracing::warn!(slots = 12, "the frame pool is ready");
    assert!(matches!(init(&config), Err(TelemetryError::AlreadySet(_))));
    drop(guard); // flushes the file

    let files: Vec<_> = std::fs::read_dir(folder.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("spectrumiq.") && name.ends_with(".log"),
        "{name}"
    );
    let text = std::fs::read_to_string(&files[0]).unwrap();
    assert!(
        text.contains("the frame pool is ready") && text.contains("slots=12"),
        "{text}"
    );
    assert!(!text.contains('\u{1b}'), "no colour codes in files");
}
