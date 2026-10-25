//! `siq-datagen`: synthetic datasets with exact ground truth (ADR-0047), written into a data root
//! as the service reads them.
//!
//! ```text
//! cargo run --release -p siq-datagen -- <generator> [--out <data root>] [--seed <n>] [--smoke] [its arguments]
//! ```
//!
//! The generators are `tilescan`, `zstack` and `beads`; `siq-datagen <generator> --help` lists
//! each one's arguments. `--out` is `data` by default, the simulated instrument's data root when
//! run from the repository. `--smoke` makes them small.

use std::path::PathBuf;
use std::time::Instant;

use clap::{Arg, ArgAction, Command, value_parser};
use siq_datagen::{GENERATORS, Output};

fn main() -> anyhow::Result<()> {
    let global = |arg: Arg| arg.global(true);
    let mut cli = Command::new("siq-datagen")
        .about("Synthetic datasets with exact ground truth")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_required(true)
        .arg(global(
            Arg::new("out")
                .long("out")
                .default_value("data")
                .value_parser(value_parser!(PathBuf))
                .help("The data root: the dataset goes in its datasets folder"),
        ))
        .arg(global(
            Arg::new("seed")
                .long("seed")
                .default_value("1")
                .value_parser(value_parser!(u64))
                .help("The seed of the world, the stage's errors and the noise"),
        ))
        .arg(global(
            Arg::new("smoke")
                .long("smoke")
                .action(ArgAction::SetTrue)
                .help("Small sizes, for CI"),
        ));
    for generator in GENERATORS {
        cli = cli.subcommand(generator.command());
    }
    let matches = cli.get_matches();
    let (name, args) = matches.subcommand().expect("a generator is required");
    let generator = GENERATORS
        .iter()
        .find(|generator| generator.command().get_name() == name)
        .expect("a generator listed");
    let out = Output {
        data_root: args.get_one::<PathBuf>("out").cloned().unwrap_or_default(),
        smoke: args.get_flag("smoke"),
        seed: args.get_one::<u64>("seed").copied().unwrap_or(1),
    };
    let started = Instant::now();
    let folder = generator.generate(args, &out)?;
    let took = started.elapsed().as_secs_f64();
    println!("{name}: {} in {took:.1} s", folder.display());
    Ok(())
}
