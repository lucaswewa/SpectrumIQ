//! `regen-bindings`: `siq-euresys-sys`'s bindings, regenerated from its shim's header with
//! bindgen's command line (ADR-0004). It needs `cargo install bindgen-cli`, and LLVM for
//! libclang; nothing else does, because the bindings are committed.

use std::path::Path;

use anyhow::{Context, Result};

use crate::process;

/// The header the bindings are made from.
pub const HEADER: &str = "crates/siq-euresys-sys/shim/siq_egrabber.h";
/// The bindings.
pub const BINDINGS: &str = "crates/siq-euresys-sys/src/bindings.rs";

/// What the bindings start with, before bindgen's own line.
const PREAMBLE: &str = "// The shim's bindings, from `shim/siq_egrabber.h`: regenerate them with \
                        `cargo xtask\n// regen-bindings`, never by hand.\n\n";

/// bindgen's options: the shim's declarations only, its enums as constants named as in C, and one
/// `unsafe extern` block, for Rust 2024.
pub const ARGUMENTS: [&str; 17] = [
    HEADER,
    "--allowlist-function",
    "siq_eg_.*",
    "--allowlist-type",
    "Siq.*",
    "--allowlist-var",
    "SIQ_EG_.*",
    "--no-prepend-enum-name",
    "--merge-extern-blocks",
    "--rust-target",
    "1.85",
    "--rust-edition",
    "2024",
    "--",
    "-x",
    "c",
    "-std=c11",
];

/// Regenerates the bindings of the repository at `repo`, formatted as the workspace is.
pub fn regen(repo: &Path) -> Result<()> {
    let generated = process::output(repo, "bindgen", ARGUMENTS).context(
        "bindgen is needed: `cargo install bindgen-cli`, and LLVM, with LIBCLANG_PATH set to \
         the folder of libclang.dll",
    )?;
    std::fs::write(repo.join(BINDINGS), format!("{PREAMBLE}{generated}"))
        .with_context(|| format!("writing {BINDINGS}"))?;
    process::output(repo, "rustfmt", ["--edition", "2024", BINDINGS])?;
    Ok(())
}
