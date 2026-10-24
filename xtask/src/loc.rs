//! The code budget (conventions §2): the lines a phase adds, by class.
//!
//! A phase adds fewer than 1000 *counted* lines. Counted lines are the non-blank, non-comment
//! lines of production source. Tests, examples, documentation, configuration and generated files
//! are reported, but not counted. Neither are the files that an import commit
//! (`chore(P3.1): import … from …`) copied in unchanged: only later edits to them count.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

use anyhow::Result;

use crate::process;

/// What an added line counts as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// Production source, counted against the budget.
    Counted,
    /// Tests and benches.
    Test,
    /// Examples, and the web app's demo pages.
    Example,
    /// Generated files and lockfiles.
    Generated,
    /// Documentation.
    Doc,
    /// Manifests and configuration.
    Config,
    /// Anything else, such as tool configuration outside the source folders.
    Other,
}

impl Class {
    /// Every class, in the order reports list them.
    pub const ALL: [Class; 7] = [
        Class::Counted,
        Class::Test,
        Class::Example,
        Class::Generated,
        Class::Doc,
        Class::Config,
        Class::Other,
    ];

    /// The class's name in reports and fixtures.
    pub fn label(self) -> &'static str {
        match self {
            Class::Counted => "counted",
            Class::Test => "tests",
            Class::Example => "examples",
            Class::Generated => "generated",
            Class::Doc => "docs",
            Class::Config => "config",
            Class::Other => "other",
        }
    }

    /// The class with this label.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.label() == label)
    }
}

/// Classifies a path relative to the repository root, with `/` or `\` separators.
pub fn classify(path: &str) -> Class {
    let path = path.replace('\\', "/");
    let segments: Vec<&str> = path.split('/').collect();
    let (file, dirs) = segments.split_last().unwrap_or((&"", &[]));
    let in_dir = |name: &str| dirs.contains(&name);
    let extension = extension(file);

    let lockfile = matches!(
        *file,
        "Cargo.lock" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock"
    );
    if in_dir("generated") || lockfile || matches!(*file, "bindings.rs" | "openapi.json") {
        Class::Generated
    } else if in_dir("tests")
        || in_dir("benches")
        || in_dir("snapshots")
        || extension == "snap"
        || *file == "tests.rs"
        || file.ends_with(".spec.ts")
        || file.ends_with(".test.ts")
        || path.starts_with("web/e2e/")
    {
        Class::Test
    } else if in_dir("examples") || path.starts_with("web/src/views/dev/") {
        Class::Example
    } else if extension == "md" || segments[0] == "docs" {
        Class::Doc
    } else if is_production(&segments, file, extension) {
        Class::Counted
    } else if matches!(extension, "toml" | "json" | "yml" | "yaml")
        || file.starts_with(".git")
        || file.starts_with(".prettierrc")
        || *file == ".nvmrc"
    {
        Class::Config
    } else {
        Class::Other
    }
}

/// Whether a path is production source, which the budget counts.
fn is_production(segments: &[&str], file: &str, extension: &str) -> bool {
    let code = matches!(
        extension,
        "rs" | "wgsl"
            | "c"
            | "cc"
            | "cpp"
            | "cxx"
            | "h"
            | "hpp"
            | "ts"
            | "tsx"
            | "js"
            | "mjs"
            | "vue"
            | "css"
    );
    match segments {
        _ if extension == "wgsl" => true,
        ["crates", _, "build.rs"] => true,
        ["crates", _, "src" | "shim", ..] | ["xtask", "src", ..] => code,
        ["apps", rest @ ..] => code && (file == "build.rs" || rest.contains(&"src")),
        ["web", "src", ..] => matches!(extension, "ts" | "vue" | "css"),
        _ => false,
    }
}

fn extension(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.').map_or("", |(_, extension)| extension)
}

/// A file's comment syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Syntax {
    /// `//`, `/* */` and, in markup, `<!-- -->`.
    Slash,
    /// `#`.
    Hash,
    /// No comments: every non-blank line counts.
    Plain,
}

/// Decides which lines count, one line at a time. A block comment carries over to later lines.
#[derive(Debug)]
pub struct LineCounter {
    syntax: Syntax,
    /// The end of the block comment the counter is in.
    block_end: Option<&'static str>,
}

impl LineCounter {
    /// A counter for lines of the file at `path`, starting outside any comment.
    pub fn for_path(path: &str) -> Self {
        let syntax = match extension(path) {
            "rs" | "wgsl" | "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" | "ts" | "tsx" | "js"
            | "mjs" | "cjs" | "vue" | "css" | "html" => Syntax::Slash,
            "py" | "toml" | "yml" | "yaml" | "sh" | "ps1" => Syntax::Hash,
            _ => Syntax::Plain,
        };
        Self {
            syntax,
            block_end: None,
        }
    }

    /// Whether `line` counts: it isn't blank, and isn't only a comment.
    pub fn counts(&mut self, line: &str) -> bool {
        let line = line.trim();
        if let Some(end) = self.block_end {
            return match line.find(end) {
                Some(at) => {
                    self.block_end = None;
                    self.counts(&line[at + end.len()..])
                }
                None => false,
            };
        }
        if line.is_empty() {
            return false;
        }
        match self.syntax {
            Syntax::Plain => true,
            Syntax::Hash => !line.starts_with('#'),
            Syntax::Slash => {
                if line.starts_with("//") {
                    return false;
                }
                for (open, close) in [("/*", "*/"), ("<!--", "-->")] {
                    if let Some(rest) = line.strip_prefix(open) {
                        self.block_end = Some(close);
                        return self.counts(rest);
                    }
                }
                // The middle of a block comment that began before a diff hunk.
                !(line == "*" || line.starts_with("* ") || line.starts_with("*/"))
            }
        }
    }
}

/// A number of files and the lines they add.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    /// Files with at least one line in the class.
    pub files: usize,
    /// Lines in the class.
    pub lines: usize,
}

/// What `cargo xtask loc` measured.
#[derive(Debug, Default)]
pub struct Report {
    /// The commit the lines are counted from: the merge base of the base revision and `HEAD`.
    pub start: String,
    /// Every file with added lines: its class, and how many lines count in that class.
    pub files: BTreeMap<String, (Class, usize)>,
    /// What import commits copied in, which isn't counted.
    pub copied: Tally,
}

impl Report {
    /// The files and lines of one class.
    pub fn tally(&self, class: Class) -> Tally {
        self.files
            .values()
            .filter(|(of, lines)| *of == class && *lines > 0)
            .fold(Tally::default(), |tally, (_, lines)| Tally {
                files: tally.files + 1,
                lines: tally.lines + lines,
            })
    }

    /// The lines counted against the budget.
    pub fn counted(&self) -> usize {
        self.tally(Class::Counted).lines
    }

    /// Adds the lines that a unified diff (`git diff -U0`) adds.
    pub fn add_diff(&mut self, diff: &str) {
        let mut path: Option<String> = None;
        let mut in_hunk = false;
        let mut counter = LineCounter::for_path("");
        for line in diff.lines() {
            if line.starts_with("diff --git ") {
                (path, in_hunk) = (None, false);
            } else if line.starts_with("@@") {
                in_hunk = true;
                counter = LineCounter::for_path(path.as_deref().unwrap_or(""));
            } else if !in_hunk {
                if let Some(target) = line.strip_prefix("+++ ") {
                    path = target
                        .trim_matches('"')
                        .strip_prefix("b/")
                        .map(str::to_owned);
                }
            } else if let (Some(path), Some(added)) = (&path, line.strip_prefix('+')) {
                let counts = counter.counts(added);
                *self.entry(path) += usize::from(counts);
            }
        }
    }

    /// Adds every line of a new file.
    pub fn add_file(&mut self, path: &str, text: &str) {
        let mut counter = LineCounter::for_path(path);
        let lines = text.lines().filter(|line| counter.counts(line)).count();
        *self.entry(path) += lines;
    }

    fn entry(&mut self, path: &str) -> &mut usize {
        &mut self
            .files
            .entry(path.to_owned())
            .or_insert_with(|| (classify(path), 0))
            .1
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{:<10} {:>6} {:>7}", "class", "files", "lines")?;
        for class in Class::ALL {
            let tally = self.tally(class);
            writeln!(
                f,
                "{:<10} {:>6} {:>7}",
                class.label(),
                tally.files,
                tally.lines
            )?;
        }
        write!(
            f,
            "{:<10} {:>6} {:>7}  (copied by import commits, not counted)",
            "copied", self.copied.files, self.copied.lines
        )
    }
}

/// Whether a commit subject marks an import of copied code: `chore(P<m>.<n>): import …`.
pub fn is_import_subject(subject: &str) -> bool {
    let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    subject
        .strip_prefix("chore(P")
        .and_then(|rest| rest.split_once("): import "))
        .and_then(|(phase, _)| phase.split_once('.'))
        .is_some_and(|(milestone, phase)| number(milestone) && number(phase))
}

/// Measures the lines added since `base`: committed, staged, unstaged and untracked.
///
/// Lines are counted from the merge base of `base` and `HEAD`, so a base branch that moved on
/// after the phase branched off doesn't count against the phase.
pub fn measure(repo: &Path, base: &str) -> Result<Report> {
    let start = git(repo, &["merge-base", base, "HEAD"])?.trim().to_owned();
    let imports = imports(repo, &start)?;
    let copied: BTreeSet<&str> = imports
        .iter()
        .flat_map(|import| import.files.iter().map(String::as_str))
        .collect();
    let mut report = Report::default();

    let mut args = diff_args(&start);
    args.push(".".to_owned());
    args.extend(
        copied
            .iter()
            .map(|file| format!(":(exclude,literal){file}")),
    );
    report.add_diff(&git(repo, &args)?);

    for import in &imports {
        let mut args = diff_args(&import.commit);
        args.extend(import.files.iter().map(|file| format!(":(literal){file}")));
        report.add_diff(&git(repo, &args)?);
        report.copied.files += import.files.len();
        report.copied.lines += import.lines;
    }

    let untracked = git(repo, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for path in untracked.split('\0').filter(|path| !path.is_empty()) {
        // Binary or unreadable files have no lines to count.
        if let Ok(text) = std::fs::read_to_string(repo.join(path)) {
            report.add_file(path, &text);
        }
    }
    report.start = start;
    Ok(report)
}

/// An import commit, and the files it added.
struct Import {
    commit: String,
    files: Vec<String>,
    lines: usize,
}

fn imports(repo: &Path, start: &str) -> Result<Vec<Import>> {
    let log = git(
        repo,
        &["log", "--format=%H%x09%s", &format!("{start}..HEAD")],
    )?;
    let mut imports = Vec::new();
    for (commit, _) in log
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(_, subject)| is_import_subject(subject))
    {
        let stat = git(
            repo,
            &["show", "--numstat", "--format=", "--diff-filter=A", commit],
        )?;
        let mut import = Import {
            commit: commit.to_owned(),
            files: Vec::new(),
            lines: 0,
        };
        for row in stat.lines() {
            if let [added, _, path] = row.splitn(3, '\t').collect::<Vec<_>>()[..] {
                import.lines += added.parse::<usize>().unwrap_or(0);
                import.files.push(path.to_owned());
            }
        }
        imports.push(import);
    }
    Ok(imports)
}

/// `git diff` from `from` to the working tree, with fixed prefixes and no context lines.
fn diff_args(from: &str) -> Vec<String> {
    [
        "diff",
        "-U0",
        "--no-color",
        "--no-ext-diff",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        from,
        "--",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn git<S: AsRef<OsStr>>(repo: &Path, args: &[S]) -> Result<String> {
    let config = ["-c", "core.quotePath=false"].map(OsStr::new);
    process::output(
        repo,
        "git",
        config.into_iter().chain(args.iter().map(AsRef::as_ref)),
    )
}
