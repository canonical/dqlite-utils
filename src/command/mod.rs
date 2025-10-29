use std::process;
use std::str::FromStr;

use anyhow::anyhow;

use crate::{Context, Error, Result};

#[derive(Debug)]
pub enum Command {
    Noop,
    Quit,
}

impl Command {
    pub fn run(&self, _ctx: &mut Context) -> Result<()> {
        match self {
            Self::Noop => Ok(()),
            Self::Quit => {
                process::exit(0);
            }
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
            (unknown, []) => Err(anyhow!("unknown command '{unknown}'")),
            (_, tail) => Err(anyhow!("unrecognised arguments {tail:?}")),
        }
    }
}
