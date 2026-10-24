//! The quality gate (conventions §1.3): every check a phase passes, in one command.
//!
//! The checks run in order and stop at the first failure:
//!
//! 1. formatting, clippy with warnings denied, the tests, and the docs with warnings denied;
//! 2. the crate layering ([`crate::layers`]);
//! 3. every example, in smoke mode (`SIQ_SMOKE=1`), except those that need features;
//! 4. licences, with `cargo deny`, when it's installed;
//! 5. the web app's checks, once `web/package.json` exists.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::layers;
use crate::metadata::Metadata;
use crate::process;

/// Which optional checks run.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Skips the web app's checks.
    pub skip_web: bool,
    /// Also runs the web app's end-to-end tests.
    pub e2e: bool,
}

/// The web app's checks, by npm script name, in the order they run.
pub const WEB_SCRIPTS: [&str; 5] = ["format:check", "lint", "typecheck", "test:unit", "build"];

#[cfg(windows)]
const NPM: &str = "npm.cmd";
#[cfg(not(windows))]
const NPM: &str = "npm";

/// Runs every check on the workspace at `repo`.
pub fn run(repo: &Path, options: Options) -> Result<()> {
    step("format", cargo(&["fmt", "--all", "--", "--check"]), repo)?;
    step(
        "clippy",
        cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
        repo,
    )?;
    step("tests", cargo(&["test", "--workspace", "--locked"]), repo)?;
    let mut docs = cargo(&["doc", "--workspace", "--no-deps", "--locked"]);
    docs.env("RUSTDOCFLAGS", "-D warnings");
    step("docs", docs, repo)?;

    println!("==> layers");
    let metadata = Metadata::load(repo)?;
    let violations = layers::check(&metadata);
    if !violations.is_empty() {
        for violation in &violations {
            eprintln!("    {violation}");
        }
        bail!("layers failed: {} violation(s)", violations.len());
    }

    for (package, example) in metadata.examples() {
        let title = format!("example {}/{}", package.name, example.name);
        if !example.required_features.is_empty() {
            let features = example.required_features.join(", ");
            skip(&title, &format!("needs features {features}"));
            continue;
        }
        let mut command = cargo(&["run", "--locked", "--quiet", "-p", &package.name]);
        command
            .args(["--example", &example.name])
            .env("SIQ_SMOKE", "1");
        step(&title, command, repo)?;
    }

    let deny = Command::new("cargo").args(["deny", "--version"]).output();
    if deny.is_ok_and(|output| output.status.success()) {
        step("licences", cargo(&["deny", "check", "licenses"]), repo)?;
    } else {
        skip(
            "licences",
            "cargo-deny isn't installed: cargo install --locked cargo-deny",
        );
    }

    web(repo, options)
}

fn web(repo: &Path, options: Options) -> Result<()> {
    let dir = repo.join("web");
    let manifest = dir.join("package.json");
    if options.skip_web {
        skip("web", "--skip-web");
        return Ok(());
    }
    if !manifest.exists() {
        skip("web", "there's no web/package.json yet");
        return Ok(());
    }
    let scripts = scripts(&manifest)?;
    if !dir.join("node_modules").exists() {
        step("web: npm ci", npm(&["ci"]), &dir)?;
    }
    for script in WEB_SCRIPTS.into_iter().filter(|s| scripts.contains(*s)) {
        step(&format!("web: {script}"), npm(&["run", script]), &dir)?;
    }
    if scripts.contains("api:types") {
        let check = npm(&["run", "api:types", "--", "--check"]);
        step("web: api:types --check", check, &dir)?;
    }
    if options.e2e && scripts.contains("test:e2e") {
        step("web: test:e2e", npm(&["run", "test:e2e"]), &dir)?;
    }
    Ok(())
}

/// The names of the scripts in a `package.json`.
fn scripts(manifest: &Path) -> Result<BTreeSet<String>> {
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("could not read {}", manifest.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("could not parse {}", manifest.display()))?;
    Ok(json
        .get("scripts")
        .and_then(serde_json::Value::as_object)
        .map(|scripts| scripts.keys().cloned().collect())
        .unwrap_or_default())
}

fn cargo(args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command.args(args);
    command
}

fn npm(args: &[&str]) -> Command {
    let mut command = Command::new(NPM);
    command.args(args);
    command
}

/// Runs one check in `dir`, with its output shown as it runs.
fn step(title: &str, mut command: Command, dir: &Path) -> Result<()> {
    println!("==> {title}");
    let status = command
        .current_dir(dir)
        .status()
        .with_context(|| format!("could not run `{}`", process::describe(&command)))?;
    if !status.success() {
        bail!(
            "{title} failed: `{}` ({status})",
            process::describe(&command)
        );
    }
    Ok(())
}

fn skip(title: &str, why: &str) {
    println!("==> {title}: skipped, {why}");
}
