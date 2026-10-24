use super::*;

fn counted(path: &str, lines: &[&str]) -> Vec<bool> {
    let mut counter = LineCounter::for_path(path);
    lines.iter().map(|line| counter.counts(line)).collect()
}

// REQ-NFR-007
#[test]
fn blank_lines_and_line_comments_dont_count() {
    let lines = [
        "//! Module docs.",
        "",
        "/// Item docs.",
        "pub fn one() -> u32 {",
        "    // A comment.",
        "    1 // A trailing comment doesn't hide code.",
        "}",
    ];
    assert_eq!(
        counted("a.rs", &lines),
        [false, false, false, true, false, true, true]
    );
}

// REQ-NFR-007
#[test]
fn block_comments_carry_over_to_later_lines() {
    let lines = [
        "/* Starts here,",
        "   goes on,",
        "   ends here. */ let after = 2;",
        "/** One line. */",
        "let z = 3;",
    ];
    assert_eq!(counted("a.rs", &lines), [false, false, true, false, true]);
}

// REQ-NFR-007
#[test]
fn a_hunk_inside_an_older_block_comment_doesnt_count() {
    let lines = [" * More docs.", " *", " */", "*self.count += 1;"];
    assert_eq!(counted("a.rs", &lines), [false, false, false, true]);
}

// REQ-NFR-007
#[test]
fn markup_and_hash_comments() {
    let vue = [
        "<!-- One line. -->",
        "<!--",
        "  Several lines.",
        "-->",
        "<template>",
    ];
    assert_eq!(counted("a.vue", &vue), [false, false, false, false, true]);
    let toml = ["# A comment.", "[package]", "", "name = \"x\" # trailing"];
    assert_eq!(counted("Cargo.toml", &toml), [false, true, false, true]);
    let markdown = ["# A heading counts in plain text.", "", "Text."];
    assert_eq!(counted("README.md", &markdown), [true, false, true]);
}

// REQ-NFR-007
#[test]
fn import_subjects() {
    assert!(is_import_subject(
        "chore(P3.1): import theme/ from microscope@0640f99"
    ));
    assert!(is_import_subject("chore(P12.3): import x"));
    for subject in [
        "feat(P3.1): import x",
        "chore(P3.1): imports x",
        "chore(P3): import x",
        "chore(Px.1): import x",
        "chore(P3.1.2): import x",
        "chore: import x",
    ] {
        assert!(!is_import_subject(subject), "{subject}");
    }
}

// REQ-NFR-007
#[test]
fn a_diff_counts_added_lines_by_file_and_class() {
    let diff = "\
diff --git a/crates/siq-a/src/lib.rs b/crates/siq-a/src/lib.rs
index 1111111..2222222 100644
--- a/crates/siq-a/src/lib.rs
+++ b/crates/siq-a/src/lib.rs
@@ -1,0 +2,3 @@
+pub fn a() {}
+// A comment.
+++counter;
@@ -9 +11 @@
-old();
+new();
diff --git a/docs/notes.md b/docs/notes.md
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/docs/notes.md
@@ -0,0 +1,2 @@
+# Notes
+
diff --git a/old.rs b/old.rs
deleted file mode 100644
index 4444444..0000000
--- a/old.rs
+++ /dev/null
@@ -1 +0,0 @@
--- a removed line that looks like a header
diff --git a/image.png b/image.png
new file mode 100644
index 0000000..5555555
Binary files /dev/null and b/image.png differ
";
    let mut report = Report::default();
    report.add_diff(diff);
    assert_eq!(report.tally(Class::Counted), Tally { files: 1, lines: 3 });
    assert_eq!(report.tally(Class::Doc), Tally { files: 1, lines: 1 });
    assert_eq!(report.files.len(), 2);
}

// REQ-NFR-007
#[test]
fn a_new_file_counts_every_line() {
    let mut report = Report::default();
    report.add_file(
        "crates/siq-a/src/new.rs",
        "/* Header. */\n\npub const A: u8 = 1;\npub const B: u8 = 2;\n",
    );
    report.add_file("crates/siq-a/src/new/tests.rs", "#[test]\nfn t() {}\n");
    assert_eq!(report.counted(), 2);
    assert_eq!(report.tally(Class::Test), Tally { files: 1, lines: 2 });
}
