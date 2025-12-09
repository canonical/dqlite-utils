pub(crate) mod help;
pub(crate) mod log;
pub(crate) mod quit;
pub(crate) mod snapshot;
pub(crate) mod status;

use std::str::FromStr;

use anyhow::Error;
use strum::EnumIter;

use crate::{Context, Result, Shell};

use self::help::{Help, HelpCommand};
use self::log::LogCommand;
use self::quit::QuitCommand;
use self::snapshot::{SnapshotCommand, SnapshotShellCommand, SnapshotShellCommandKind};
use self::status::StatusCommand;

pub enum Command {
    // NOTE: when adding new commands, remember to add them to the general `help` output.
    Noop,
    Help(HelpCommand),
    Root(RootCommand),
    Snapshot(SnapshotShellCommand),
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
            "help" | ".help" => return Ok(Self::Help(HelpCommand::try_from_args(args)?)),
            _ => {}
        }

        // NOTE: All commands share the same namespace, thereby allowing us to successfully
        // parse all commands ahead of time, without knowing their effect on the Context;
        // availability is checked later.
        match RootCommand::try_from_input(command, args) {
            Ok(cmd) => return Ok(Self::Root(cmd)),
            Err(err) if err.is::<UnknownCommand>() => {}
            Err(err) => return Err(err),
        }
        let cmd = SnapshotShellCommand::try_from_input(command, args)?;
        Ok(Self::Snapshot(cmd))
    }
}

impl Command {
    pub fn run(self, ctx: &mut Context) -> Result<()> {
        match (self, &ctx.shell) {
            (Self::Noop, _) => Ok(()),
            (Self::Help(cmd), _) => cmd.run(ctx),
            (Self::Root(cmd), Shell::Root(_)) => cmd.run(ctx),
            (Self::Root(cmd), _) => {
                return Err(Error::from(CommandUnavailable {
                    command_name: cmd.kind().name(),
                    shell_name: ctx.shell.name(),
                }));
            }
            (Self::Snapshot(cmd), Shell::Snapshot(_)) => cmd.run(ctx),
            (Self::Snapshot(cmd), _) => {
                return Err(Error::from(CommandUnavailable {
                    command_name: cmd.kind().name(),
                    shell_name: ctx.shell.name(),
                }));
            }
        }
    }

    #[allow(unused)]
    fn kind(&self) -> CommandKind {
        match self {
            Self::Noop => CommandKind::Noop,
            Self::Help(_) => CommandKind::Help,
            Self::Root(cmd) => CommandKind::Root(cmd.kind()),
            Self::Snapshot(cmd) => CommandKind::Snapshot(cmd.kind()),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CommandKind {
    Noop,
    Help,
    Root(RootCommandKind),
    Snapshot(SnapshotShellCommandKind),
}

impl CommandKind {
    pub(crate) fn help(&self) -> Help {
        match self {
            Self::Noop => panic!("cannot get help of noop command"),
            Self::Help => HelpCommand::help(),
            Self::Root(kind) => kind.help(),
            Self::Snapshot(kind) => kind.help(),
        }
    }

    #[cfg(test)]
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Noop => panic!("cannot get help of noop command"),
            Self::Help => "help",
            Self::Root(kind) => kind.name(),
            Self::Snapshot(kind) => kind.name(),
        }
    }
}

impl FromStr for CommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        if raw == "help" {
            return Ok(Self::Help);
        }
        RootCommandKind::from_str(raw)
            .map(Self::Root)
            .or_else(|_| SnapshotShellCommandKind::from_str(raw).map(Self::Snapshot))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{command_name} command unavailable in {shell_name} shell")]
struct CommandUnavailable {
    command_name: &'static str,
    shell_name: &'static str,
}

#[derive(Debug)]
pub enum RootCommand {
    Log(LogCommand),
    Quit(QuitCommand),
    Snapshot(SnapshotCommand),
    Status(StatusCommand),
}

impl RootCommand {
    fn try_from_input(command: &str, args: &[String]) -> Result<Self> {
        match command.parse()? {
            RootCommandKind::Log => Ok(Self::Log(LogCommand::try_from_args(args)?)),
            RootCommandKind::Quit => Ok(Self::Quit(QuitCommand::try_from_args(args)?)),
            RootCommandKind::Snapshot => Ok(Self::Snapshot(SnapshotCommand::try_from_args(args)?)),
            RootCommandKind::Status => Ok(Self::Status(StatusCommand::try_from_args(args)?)),
        }
    }

    pub fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Log(cmd) => cmd.run(ctx),
            Self::Quit(cmd) => cmd.run(ctx),
            Self::Snapshot(cmd) => cmd.run(ctx),
            Self::Status(cmd) => cmd.run(ctx),
        }
    }

    fn kind(&self) -> RootCommandKind {
        match self {
            Self::Log(_) => RootCommandKind::Log,
            Self::Quit(_) => RootCommandKind::Quit,
            Self::Snapshot(_) => RootCommandKind::Snapshot,
            Self::Status(_) => RootCommandKind::Status,
        }
    }
}

#[derive(Debug, Eq, PartialEq, EnumIter)]
pub(crate) enum RootCommandKind {
    Log,
    Quit,
    Snapshot,
    Status,
}

impl RootCommandKind {
    pub(crate) fn help(&self) -> Help {
        match self {
            Self::Log => LogCommand::help(),
            Self::Quit => QuitCommand::help(),
            Self::Snapshot => SnapshotCommand::help(),
            Self::Status => StatusCommand::help(),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Quit => "quit",
            Self::Snapshot => "snapshot",
            Self::Status => "status",
        }
    }
}

impl FromStr for RootCommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw {
            "log" => Ok(Self::Log),
            "quit" => Ok(Self::Quit),
            "status" => Ok(Self::Status),
            "snapshot" => Ok(Self::Snapshot),
            _ => Err(UnknownCommand.into()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("missing argument '{_0}'")]
struct MissingArgumentError(&'static str);

#[derive(Debug, thiserror::Error)]
#[error("unrecognized arguments: {_0:?}")]
struct UnrecognizedArgumentsError(Vec<String>);

#[derive(Debug, thiserror::Error)]
#[error("unknown command")]
struct UnknownCommand;

#[cfg(test)]
mod tests {
    use super::*;

    use googletest::expect_that;
    use googletest::matchers::{eq, lt, not};
    use strum::IntoEnumIterator;

    #[googletest::test]
    fn test_command_kinds_sorted_by_name() {
        fn test_kinds(kinds: impl Iterator<Item = CommandKind>) {
            let kinds: Vec<_> = kinds.collect();
            for window in kinds.windows(2) {
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
        test_kinds(RootCommandKind::iter().map(CommandKind::Root));
        test_kinds(SnapshotShellCommandKind::iter().map(CommandKind::Snapshot));
    }
}
