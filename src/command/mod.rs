mod log;
mod status;

use std::process;
use std::str::FromStr;

use anyhow::anyhow;

use crate::{Context, Error, Result};

use self::log::LogCommand;
use self::status::StatusCommand;

#[derive(Debug)]
pub enum Command {
    Noop,
    Status(StatusCommand),
    Log(LogCommand),
    Quit,
}

impl Command {
    pub fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Noop => Ok(()),
            Self::Quit => {
                process::exit(0);
            }
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
            "quit" => {
                if args.is_empty() {
                    Ok(Self::Quit)
                } else {
                    Err(UnrecognisedArgumentsError(args.to_vec()).into())
                }
            }
            unknown => Err(anyhow!("unknown command '{unknown}'")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unrecognised arguments: {_0:?}")]
struct UnrecognisedArgumentsError(Vec<String>);
