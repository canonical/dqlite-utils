mod dqlite;
mod info;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, command};

use crate::{dqlite::DqliteState, info::InfoCommand};

fn main() -> Result<()> {
    let args = DqliteUtilsArgs::parse();
    let dqlite = DqliteState::from(&args.folder)?;

    args.command.run(&dqlite)
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct DqliteUtilsArgs {
    #[arg(short, long, default_value = ".")]
    folder: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Info(InfoCommand),
}

impl Command {
    fn run(&self, dqlite: &DqliteState) -> Result<()> {
        match self {
            Command::Info(info) => info.run(dqlite),
        }
    }
}
