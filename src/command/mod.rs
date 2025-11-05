pub(crate) mod quit;

mod status;

use std::str::FromStr;

use anyhow::anyhow;

use crate::{Context, Error, Result};

use self::quit::Command as QuitCommand;
use self::status::Command as StatusCommand;

#[derive(Debug)]
pub enum Command {
    Noop,
    Quit(QuitCommand),
    Status(StatusCommand),
}

impl Command {
    pub fn run(&self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Noop => Ok(()),
            Self::Quit(cmd) => cmd.run(ctx),
            Self::Status(cmd) => cmd.run(ctx),
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
        match command.as_str() {
            "quit" => Ok(Self::Quit(QuitCommand::try_from_args(args)?)),
            "status" => Ok(Self::Status(StatusCommand::try_from_args(args)?)),
            unknown => Err(anyhow!("unknown command '{unknown}'")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unrecognized arguments: {_0:?}")]
struct UnrecognizedArgumentsError(Vec<String>);
