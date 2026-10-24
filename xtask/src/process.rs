//! Running other programs.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Runs `program` with `args` in `dir`, and returns what it printed.
///
/// Fails with what it printed to standard error if it exits with an error.
pub(crate) fn output<I, S>(dir: &Path, program: &str, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command.args(args).current_dir(dir);
    let output = command
        .output()
        .with_context(|| format!("could not run `{program}`"))?;
    if !output.status.success() {
        bail!(
            "`{}` failed ({}): {}",
            describe(&command),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The command line, for messages.
pub(crate) fn describe(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(OsStr::to_string_lossy)
        .collect::<Vec<_>>()
        .join(" ")
}
