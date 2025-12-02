mod log;
pub(crate) mod quit;
mod status;

use std::str::FromStr;

use anyhow::anyhow;

use crate::{Context, Error, Result};

use self::log::LogCommand;
use self::quit::QuitCommand;
use self::status::StatusCommand;

#[derive(Debug)]
pub enum Command {
    Noop,
    Quit(QuitCommand),
    Status(StatusCommand),
    Log(LogCommand),
}

impl Command {
    pub fn run(self, ctx: &mut Context) -> Result<Option<ReplEffect>> {
        match self {
            Self::Noop => Ok(None),
            Self::Quit(cmd) => cmd.run(ctx),
            Self::Status(cmd) => cmd.run(ctx),
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
        match command.as_str() {
            "status" => Ok(Self::Status(StatusCommand::try_from_args(args)?)),
            "log" => Ok(Self::Log(LogCommand::try_from_args(args)?)),
            "quit" => Ok(Self::Quit(QuitCommand::try_from_args(args)?)),
            unknown => Err(anyhow!("unknown command '{unknown}'")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unrecognized arguments: {_0:?}")]
struct UnrecognizedArgumentsError(Vec<String>);

pub enum ReplEffect {
    ChangePrompt(String),
    Quit,
}
