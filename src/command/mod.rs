mod help;
mod log;
pub(crate) mod quit;
mod status;

use std::io::{self, Write};
use std::str::FromStr;

use anyhow::anyhow;
use strum::EnumIter;

use crate::{Context, Error, Result};

use self::help::HelpCommand;
use self::log::LogCommand;
use self::quit::QuitCommand;
use self::status::StatusCommand;

#[derive(Debug)]
pub enum Command {
    // NOTE: when adding new commands, remember to add them to the general `help` output.
    Quit(QuitCommand),
    Status(StatusCommand),
    Help(HelpCommand),
    Log(LogCommand),

    Noop,
}

impl Command {
    pub fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Noop => Ok(()),
            Self::Quit(cmd) => cmd.run(ctx),
            Self::Status(cmd) => cmd.run(ctx),
            Self::Help(cmd) => cmd.run(ctx),
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
        match command.parse()? {
            CommandKind::Status => Ok(Self::Status(StatusCommand::try_from_args(args)?)),
            CommandKind::Log => Ok(Self::Log(LogCommand::try_from_args(args)?)),
            CommandKind::Help => Ok(Self::Help(HelpCommand::try_from_args(args)?)),
            CommandKind::Quit => Ok(Self::Quit(QuitCommand::try_from_args(args)?)),
        }
    }
}

#[derive(Debug, Eq, PartialEq, EnumIter)]
pub(crate) enum CommandKind {
    Log,
    Quit,
    Status,
    Help,
}

impl CommandKind {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Quit => "quit",
            Self::Status => "status",
            Self::Help => "help",
        }
    }

    pub(crate) fn summary(&self) -> &'static str {
        match self {
            Self::Log => LogCommand::SUMMARY,
            Self::Quit => QuitCommand::SUMMARY,
            Self::Status => StatusCommand::SUMMARY,
            Self::Help => HelpCommand::SUMMARY,
        }
    }

    pub(crate) fn write_help(&self, writer: impl Write) -> io::Result<()> {
        let help = match self {
            Self::Log => LogCommand::help(),
            Self::Quit => QuitCommand::help(),
            Self::Status => StatusCommand::help(),
            Self::Help => HelpCommand::help(),
        };
        help.write_to(writer)?;
        Ok(())
    }
}

impl FromStr for CommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> std::prelude::v1::Result<Self, Self::Err> {
        match raw {
            "log" => Ok(Self::Log),
            "quit" => Ok(Self::Quit),
            "status" => Ok(Self::Status),
            "help" => Ok(Self::Help),
            unknown => Err(anyhow!("unknown command '{unknown}'")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unrecognized arguments: {_0:?}")]
struct UnrecognizedArgumentsError(Vec<String>);

#[cfg(test)]
mod tests {
    use super::*;

    use googletest::expect_that;
    use googletest::matchers::{eq, lt, not};
    use strum::IntoEnumIterator;

    #[googletest::test]
    fn test_command_kinds_sorted_by_name() {
        let command_kinds: Vec<_> = CommandKind::iter().collect();
        for window in command_kinds.windows(2) {
            let (entry_1, entry_2) = match window {
                [e_1, e_2] => (e_1, e_2),
                _ => unreachable!(),
            };
            // Help must come last.
            expect_that!(entry_1, not(eq(&CommandKind::Help)));

            if !matches!(entry_2, CommandKind::Help) {
                expect_that!(entry_1.name(), lt(entry_2.name()));
            }
        }
    }
}
