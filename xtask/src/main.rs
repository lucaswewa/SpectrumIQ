//! `cargo xtask`: SpectrumIQ's development tasks. See xtask/README.md.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use xtask::metadata::Metadata;
use xtask::{ci, layers, loc, regen, soak};

#[derive(Parser)]
#[command(name = "cargo xtask", about = "SpectrumIQ's development tasks.")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Runs every check a phase must pass (conventions §1.3).
    Ci {
        /// Skips the web app's checks.
        #[arg(long)]
        skip_web: bool,
        /// Also runs the web app's end-to-end tests.
        #[arg(long)]
        e2e: bool,
    },
    /// Counts the lines added since a base revision (conventions §2).
    Loc {
        /// The revision the phase started from. Lines count from its merge base with HEAD.
        #[arg(long, default_value = "main")]
        base: String,
        /// The budget: the counted lines must stay below it.
        #[arg(long, default_value_t = 1000)]
        budget: usize,
        /// Also lists every file with added lines.
        #[arg(long)]
        files: bool,
    },
    /// Checks the crate layering (architecture §3.1).
    Layers,
    /// Regenerates siq-euresys-sys's bindings from its shim's header: needs bindgen-cli and LLVM.
    RegenBindings,
    /// Runs viewers and an operator against a service of its own, and reports (ADR-0069).
    Soak {
        /// How many viewers.
        #[arg(long, default_value_t = 8)]
        viewers: usize,
        /// For how long, in minutes.
        #[arg(long, default_value_t = 30.0)]
        minutes: f64,
        /// The smoke run: 2 viewers for 20 s.
        #[arg(long)]
        smoke: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse().task) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(task: Task) -> anyhow::Result<ExitCode> {
    let repo = repo_root();
    match task {
        Task::Ci { skip_web, e2e } => {
            ci::run(&repo, ci::Options { skip_web, e2e })?;
            println!("ci: every check passed");
            Ok(ExitCode::SUCCESS)
        }
        Task::Loc {
            base,
            budget,
            files,
        } => {
            let report = loc::measure(&repo, &base)?;
            let start = report.start.get(..10).unwrap_or(&report.start);
            println!(
                "Lines added since {base} (from {start}), with uncommitted and untracked files:"
            );
            println!("\n{report}\n");
            if files {
                for (path, (class, lines)) in &report.files {
                    println!("{lines:>7}  {:<9}  {path}", class.label());
                }
                println!();
            }
            let counted = report.counted();
            if counted < budget {
                println!("budget: {counted} counted lines, below {budget}: OK");
                Ok(ExitCode::SUCCESS)
            } else {
                println!("budget: {counted} counted lines, not below {budget}: split the phase");
                Ok(ExitCode::FAILURE)
            }
        }
        Task::Layers => {
            let metadata = Metadata::load(&repo)?;
            let violations = layers::check(&metadata);
            for violation in &violations {
                println!("{violation}");
            }
            if violations.is_empty() {
                let count = metadata.packages.len();
                let plural = if count == 1 { "" } else { "s" };
                println!("layers: OK ({count} crate{plural} checked)");
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::FAILURE)
            }
        }
        Task::RegenBindings => {
            regen::regen(&repo)?;
            println!("regen-bindings: wrote {}", regen::BINDINGS);
            Ok(ExitCode::SUCCESS)
        }
        Task::Soak {
            viewers,
            minutes,
            smoke,
        } => {
            let options = if smoke {
                soak::Options::SMOKE
            } else {
                let duration = Duration::from_secs_f64(minutes * 60.0);
                soak::Options { viewers, duration }
            };
            let report = soak::run(&repo, options)?;
            println!("{report}");
            Ok(if report.passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

/// The workspace root, which is xtask's parent folder.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is a folder of the workspace")
        .to_path_buf()
}
