//! The configuration format: the shipped files, defaults, every kind of problem with its
//! location, and the JSON Schema.

use std::path::Path;

use siq_config::*;

fn shipped(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config")
        .join(name);
    std::fs::read_to_string(path).expect("the shipped configuration exists")
}

// REQ-SIM-006
#[test]
fn the_simulated_instrument_is_valid() {
    let config = from_toml(&shipped("sim.toml"), "sim.toml", Path::new("config")).unwrap();
    assert!(matches!(config.camera, CameraConfig::Sim(_)));
    assert!(matches!(config.stage, StageConfig::Sim(_)));
    assert!(matches!(config.focus, FocusConfig::Sim(_)));
    assert_eq!(config.server.data_root, Path::new("config").join("../data"));
    insta::assert_json_snapshot!("sim_toml", config);
}

#[test]
fn the_hardware_example_is_valid_and_keeps_absolute_paths() {
    let text = shipped("hardware.example.toml");
    let config = from_toml(&text, "hardware.example.toml", Path::new("config")).unwrap();
    assert_eq!(config.server.data_root, Path::new("D:/SpectrumIQ/data"));
    let FocusConfig::Ascii(focus) = &config.focus else {
        panic!("an ASCII focus drive")
    };
    assert_eq!(
        focus.dialect,
        Path::new("config").join("dialects/focus-drive.toml")
    );
}

#[test]
fn load_reads_the_file_and_resolves_against_its_folder() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let config = load(&path).unwrap();
    assert_eq!(
        config.server.scratch,
        path.parent().unwrap().join("../scratch")
    );
    let missing = load(Path::new("no-such-file.toml")).unwrap_err();
    assert!(
        missing
            .to_string()
            .starts_with("no-such-file.toml: can't be read: ")
    );
}

const MINIMAL: &str = r#"
[camera]
kind = "sim"
[stage]
kind = "sim"
[focus]
kind = "sim"
[[objectives]]
id = 1
name = "10x"
magnification = 10
numerical_aperture = 0.3
[[channels]]
id = 1
name = "Brightfield"
exposure = 10
"#;

// REQ-NFR-002
#[test]
fn defaults_fill_what_a_minimal_configuration_leaves_out() {
    let config = from_toml(MINIMAL, "minimal.toml", Path::new("")).unwrap();
    assert_eq!(config.server, ServerConfig::default());
    assert_eq!(config.memory.pool_slots, 12);
    assert_eq!(config.memory.job_ram_gb, 14.0);
    assert_eq!(config.channels[0].colour, Colour([255, 255, 255]));
    insta::assert_json_snapshot!("minimal_toml", config);
}

/// Each case puts a prefix before the minimal configuration, and replaces one piece of it. Every
/// case must be rejected.
const CASES: &[(&str, &str, &str, &str)] = &[
    (
        "turret",
        "[peripherals.objective_turret]\npositions = 2\n\
         [[objectives]]\nid = 2\nname = \"20x\"\nmagnification = 20\nnumerical_aperture = 0.5\n\
         turret_position = 1\n\
         [[objectives]]\nid = 3\nname = \"40x\"\nmagnification = 40\nnumerical_aperture = 0.7\n\
         turret_position = 1\n\
         [[objectives]]\nid = 4\nname = \"60x\"\nmagnification = 60\nnumerical_aperture = 0.9\n\
         turret_position = 2",
        "",
        "",
    ),
    ("syntax", "[server]\nport = ", "", ""),
    ("typo", "[server]\nprot = 5001", "", ""),
    ("type", "[server]\nport = \"5000\"", "", ""),
    ("missing section", "", "[stage]\nkind = \"sim\"\n", ""),
    (
        "unknown kind",
        "",
        "kind = \"sim\"\n[stage]",
        "kind = \"basler\"\n[stage]",
    ),
    (
        "colour",
        "",
        "exposure = 10\n",
        "exposure = 10\ncolour = \"#33ff6\"\n",
    ),
    (
        "missing key",
        "",
        "[stage]\nkind = \"sim\"",
        "[stage]\nkind = \"grbl\"",
    ),
    (
        "stage rules",
        "",
        "[stage]\nkind = \"sim\"",
        "[stage]\nkind = \"grbl\"\nport = \"\"\npoll_hz = 500\n\
         motion = { approach = [1, 0, -1], overshoot = -5 }",
    ),
    (
        "rules",
        "[memory]\npool_slots = 1\n[peripherals.filter_wheel]\nfilters = [\"DAPI\", \"DAPI\"]\n\
         [[channels]]\nid = 1\nname = \"DAPI\"\nexposure = 0\nfilter = \"GFP\"\n\
         illumination = { source = \"470nm\", intensity = 1.5 }\n\
         [[objectives]]\nid = 2\nname = \"10x\"\nmagnification = 20\nnumerical_aperture = 1.9\n\
         turret_position = 1",
        "",
        "",
    ),
];

#[test]
fn every_problem_is_reported_with_its_location() {
    let mut report = String::new();
    for (name, prefix, from, to) in CASES {
        let mut text = format!("{prefix}\n{MINIMAL}");
        if !from.is_empty() {
            assert!(text.contains(from), "{name}: the piece to replace is there");
            text = text.replacen(from, to, 1);
        }
        let error = from_toml(&text, &format!("{name}.toml"), Path::new("")).unwrap_err();
        report.push_str(&format!("{error}\n"));
    }
    insta::assert_snapshot!(report);
}

#[test]
fn the_json_schema_describes_the_format() {
    let schema = json_schema();
    assert_eq!(schema["title"], "InstrumentConfig");
    insta::assert_json_snapshot!("schema", schema);
}
