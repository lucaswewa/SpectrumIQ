//! Checks instrument configuration files, and prints each one with its defaults filled in.
//!
//! ```text
//! cargo run -p siq-config --example validate_config -- config/sim.toml
//! cargo run -p siq-config --example validate_config -- --schema
//! ```
//!
//! Without arguments, it checks the configurations shipped in `config/`, which is also what smoke
//! mode (`SIQ_SMOKE=1`) does. It exits with an error if a file is invalid.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--schema") {
        println!("{:#}", siq_config::json_schema());
        return ExitCode::SUCCESS;
    }
    let files: Vec<PathBuf> = if args.is_empty() {
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        ["sim.toml", "hardware.example.toml"]
            .map(|name| shipped.join(name))
            .into()
    } else {
        args.iter().map(PathBuf::from).collect()
    };
    let mut ok = true;
    for file in &files {
        match siq_config::load(file) {
            Ok(config) => {
                println!("# {}: valid\n", file.display());
                match toml::to_string_pretty(&config) {
                    Ok(text) => println!("{text}"),
                    Err(error) => println!("(can't be shown as TOML: {error})\n"),
                }
            }
            Err(error) => {
                eprintln!("{error}");
                ok = false;
            }
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
