//! What the shell's two capabilities grant, and to which pages (ADR-0040), read from the files
//! Tauri builds them from. REQ-NET-001, REQ-VIS-005.

use std::fs;

use serde_json::Value;
use tauri::utils::acl::RemoteUrlPattern;
use url::Url;

fn capability(name: &str) -> Value {
    let path = format!("{}/capabilities/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn permissions(capability: &Value) -> Vec<&str> {
    let listed = capability["permissions"].as_array().unwrap();
    listed
        .iter()
        .map(|permission| permission.as_str().unwrap())
        .collect()
}

#[test]
fn the_web_app_s_commands_are_granted_to_the_local_service_s_pages_alone() {
    let remote = capability("remote");
    let urls = remote["remote"]["urls"].as_array().unwrap();
    let patterns: Vec<RemoteUrlPattern> = urls
        .iter()
        .map(|url| url.as_str().unwrap().parse().unwrap())
        .collect();
    let granted = |url: &str| {
        patterns
            .iter()
            .any(|pattern| pattern.test(&Url::parse(url).unwrap()))
    };
    for local in [
        "http://127.0.0.1:5000/",
        "http://127.0.0.1:5000/#/live",
        "http://127.0.0.1:5078/assets/index.js",
    ] {
        assert!(granted(local), "{local}");
    }
    for elsewhere in [
        "http://localhost:5000/",
        "https://127.0.0.1:5000/",
        "http://10.0.0.5:5000/",
        "http://lab-pc:5000/",
        "https://example.org/",
    ] {
        assert!(!granted(elsewhere), "{elsewhere}");
    }
}

#[test]
fn the_web_app_gets_the_host_commands_and_the_connect_page_only_its_own() {
    let remote = capability("remote");
    let remote = permissions(&remote);
    for command in [
        "allow-app-info",
        "allow-open-link",
        "allow-save-file",
        "allow-service-state",
    ] {
        assert!(remote.contains(&command), "{command}");
    }
    // Neither the probe, nor Tauri's core set: the web app may not close or move the window.
    assert!(!remote.contains(&"allow-probe"));
    assert!(
        remote
            .iter()
            .all(|permission| !permission.ends_with(":default"))
    );

    let connect = capability("connect");
    assert!(
        connect.get("remote").is_none(),
        "the connect page is the bundled one"
    );
    assert!(permissions(&connect).contains(&"allow-probe"));
    assert!(!permissions(&connect).contains(&"allow-save-file"));
}
