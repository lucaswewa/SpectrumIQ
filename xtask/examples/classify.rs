//! Classifies paths by the code-budget rules, as `cargo xtask loc` does.
//!
//! ```text
//! cargo run -p xtask --example classify -- crates/siq-types/src/lib.rs web/src/views/dev/X.vue
//! ```
//!
//! Without arguments it classifies a sample of paths, checks each against its expected class, and
//! exits with an error if one is wrong. Smoke mode (`SIQ_SMOKE=1`) does the same.

use std::process::ExitCode;

use xtask::loc::{Class, classify};

const SAMPLE: &[(&str, Class)] = &[
    ("crates/siq-types/src/lib.rs", Class::Counted),
    ("crates/siq-types/src/units/tests.rs", Class::Test),
    ("crates/siq-types/examples/affine_demo.rs", Class::Example),
    ("crates/siq-euresys-sys/src/bindings.rs", Class::Generated),
    ("docs/plan/README.md", Class::Doc),
    ("crates/siq-types/Cargo.toml", Class::Config),
    ("web/vite.config.ts", Class::Other),
];

fn main() -> ExitCode {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if !paths.is_empty() {
        for path in &paths {
            println!("{:<10} {path}", classify(path).label());
        }
        return ExitCode::SUCCESS;
    }
    let mut wrong = 0;
    for &(path, expected) in SAMPLE {
        let class = classify(path);
        if class == expected {
            println!("{:<10} {path}", class.label());
        } else {
            println!(
                "{:<10} {path}  <- expected {}",
                class.label(),
                expected.label()
            );
            wrong += 1;
        }
    }
    if wrong == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
