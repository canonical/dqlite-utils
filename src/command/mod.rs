mod log;

use std::process;
use std::str::FromStr;

use anyhow::anyhow;

use crate::{Context, Error, Result};

use self::log::Command as LogCommand;

#[derive(Debug)]
pub enum Command {
    Noop,
    Quit,
    Log(LogCommand),
}

impl Command {
    pub fn run(&self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Noop => Ok(()),
            Self::Quit => {
                process::exit(0);
            }
            Self::Log(cmd) => cmd.run(ctx),
        }
    }
}

impl FromStr for Command {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let words = shell_words::split(raw)?;
        let (command, args) = match words.split_first() {
            Some((command, args)) => (command, args),
            None => return Ok(Self::Noop),
        };
        match (command.as_str(), args) {
            ("quit", []) => Ok(Self::Quit),
            ("log", _) => Ok(Self::Log(LogCommand::try_from_args(args)?)),
            (unknown, []) => Err(anyhow!("unknown command '{unknown}'")),
            (_, tail) => Err(anyhow!("unrecognised arguments {tail:?}")),
        }
    }
}
