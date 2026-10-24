//! `cargo xtask loc`: path classes from a fixture, and a measurement in a scratch repository.

use std::fs;
use std::path::Path;
use std::process::Command;

use xtask::loc::{self, Class, Tally};

// REQ-NFR-007
#[test]
fn paths_are_classified_as_the_fixture_says() {
    let fixture = include_str!("fixtures/loc_paths.txt");
    let mut checked = 0;
    for line in fixture.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (label, path) = line.split_once(' ').expect("each line is `<class> <path>`");
        let expected = Class::from_label(label).unwrap_or_else(|| panic!("unknown class {label}"));
        assert_eq!(loc::classify(path), expected, "{path}");
        checked += 1;
    }
    assert!(checked > 40, "the fixture lists every rule");
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=SpectrumIQ test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    fs::create_dir_all(path.parent().expect("a file in a folder")).expect("creates the folder");
    fs::write(path, text).expect("writes the file");
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "--all"]);
    git(dir, &["commit", "--quiet", "-m", message]);
}

// REQ-NFR-007
#[test]
fn a_phase_branch_is_measured_from_its_merge_base_with_imports_excluded() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let dir = scratch.path();
    git(dir, &["init", "--quiet", "-b", "main"]);
    write(dir, "README.md", "# Base\n");
    commit_all(dir, "base");
    git(dir, &["switch", "--quiet", "-c", "phase/P9.9-test"]);

    // Counted: the 3 lines of the function. Tests: 4. Example: 3. Config: 2.
    write(
        dir,
        "crates/siq-demo/src/lib.rs",
        "//! Demo.\n\npub fn one() -> u32 {\n    1 // one\n}\n",
    );
    write(
        dir,
        "crates/siq-demo/src/lib/tests.rs",
        "#[test]\nfn one() {\n    assert_eq!(super::one(), 1);\n}\n",
    );
    write(
        dir,
        "crates/siq-demo/examples/demo.rs",
        "fn main() {\n    println!(\"{}\", siq_demo::one());\n}\n",
    );
    write(
        dir,
        "crates/siq-demo/Cargo.toml",
        "[package]\nname = \"siq-demo\"\n",
    );
    commit_all(dir, "feat(P9.9): demo");

    // Copied: 5 lines, which don't count. The later edit counts: 1 changed and 3 new lines.
    let button = "<template>\n  <button>\n    <slot />\n  </button>\n</template>\n";
    write(dir, "web/src/ui/AppButton.vue", button);
    commit_all(dir, "chore(P9.9): import AppButton from microscope@0640f99");
    let styled = button.replace("<button>", "<button class=\"app-button\">")
        + "<style>\n.app-button { margin: 0; }\n</style>\n";
    write(dir, "web/src/ui/AppButton.vue", &styled);
    commit_all(dir, "feat(P9.9): style the button");

    // The base branch moves on. Its change doesn't count against the phase.
    git(dir, &["switch", "--quiet", "main"]);
    write(dir, "README.md", "# Base, edited later\n");
    commit_all(dir, "docs: edit the README");
    git(dir, &["switch", "--quiet", "phase/P9.9-test"]);

    // Uncommitted work counts: 3 lines appended to lib.rs, and 1 line of an untracked file.
    let lib = dir.join("crates/siq-demo/src/lib.rs");
    let mut text = fs::read_to_string(&lib).expect("reads lib.rs");
    text.push_str("\npub fn two() -> u32 {\n    2\n}\n");
    fs::write(&lib, text).expect("writes lib.rs");
    write(
        dir,
        "crates/siq-demo/src/extra.rs",
        "/* Extra. */\npub const EXTRA: u8 = 0;\n",
    );

    let report = loc::measure(dir, "main").expect("measures");
    assert_eq!(
        report.tally(Class::Counted),
        Tally {
            files: 3,
            lines: 11
        }
    );
    assert_eq!(report.tally(Class::Test), Tally { files: 1, lines: 4 });
    assert_eq!(report.tally(Class::Example), Tally { files: 1, lines: 3 });
    assert_eq!(report.tally(Class::Config), Tally { files: 1, lines: 2 });
    assert_eq!(report.tally(Class::Doc), Tally::default());
    assert_eq!(report.copied, Tally { files: 1, lines: 5 });
}
