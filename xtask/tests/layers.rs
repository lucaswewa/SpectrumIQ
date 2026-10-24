//! `cargo xtask layers`: fixture metadata, and the real workspace.

use std::path::Path;

use xtask::layers::{self, Layer, Violation};
use xtask::metadata::Metadata;

fn fixture(json: &str) -> Metadata {
    serde_json::from_str(json).expect("the fixture is valid metadata")
}

// REQ-NFR-007
#[test]
fn broken_rules_are_reported() {
    let violations = layers::check(&fixture(include_str!("fixtures/metadata_violations.json")));
    assert_eq!(
        violations,
        [
            Violation::Dependency {
                from: "siq-config".into(),
                from_layer: Layer::Foundation,
                to: "siq-buffer".into(),
                to_layer: Layer::Core,
            },
            Violation::Dependency {
                from: "siq-dff".into(),
                from_layer: Layer::Processing,
                to: "siq-sim".into(),
                to_layer: Layer::Devices,
            },
            Violation::TetaWot("siq-preview".into()),
            Violation::Unlisted("siq-unplanned".into()),
        ]
    );
}

// REQ-NFR-007
#[test]
fn a_layered_workspace_passes_with_dev_dependencies_exempt() {
    let metadata = fixture(include_str!("fixtures/metadata_layered.json"));
    assert_eq!(layers::check(&metadata), []);
    let examples: Vec<_> = metadata
        .examples()
        .map(|(p, t)| (&p.name, &t.name))
        .collect();
    assert_eq!(examples, [(&"xtask".to_owned(), &"classify".to_owned())]);
}

// REQ-NFR-007
#[test]
fn this_workspace_follows_the_layers() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root");
    let metadata = Metadata::load(repo).expect("cargo metadata runs");
    assert_eq!(layers::check(&metadata), []);
}
