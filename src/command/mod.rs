mod help;
mod log;
pub(crate) mod quit;
mod snapshot;
mod sql;
mod status;

pub(crate) use self::help::Help;
pub(crate) use self::snapshot::SnapshotShell;

use std::str::FromStr;

use anyhow::Error;
use strum::EnumIter;

use crate::prompt::Prompt;
use crate::{Context, Result, Shell, ShellKind};

use self::help::HelpCommand;
use self::log::LogCommand;
use self::quit::QuitCommand;
use self::snapshot::{SnapshotCommand, SnapshotShellCommand, SnapshotShellCommandKind};
use self::sql::SqlCommand;
use self::status::StatusCommand;

#[derive(Debug)]
pub enum Command {
    Noop,
    Help(HelpCommand),
    Root(RootCommand),
    Snapshot(SnapshotShellCommand),
    Sql(SqlCommand),
}

impl FromStr for Command {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        // NOTE: All commands share the same namespace, thereby allowing us to successfully
        // parse all commands ahead of time, without knowing their effect on the Context;
        // availability is checked later.
        if raw.is_empty() {
            return Ok(Self::Noop);
        }
        if raw.starts_with('.') {
            let words = shell_words::split(raw)?;
            let (command, args) = match words.split_first() {
                Some((command, args)) => (command, args),
                None => return Ok(Self::Noop),
            };

            if command == ".help" {
                return Ok(Self::Help(HelpCommand::try_from_args(args)?));
            }
            match RootCommand::try_from_input(command, args) {
                Ok(cmd) => return Ok(Self::Root(cmd)),
                Err(err) if err.is::<UnknownCommand>() => {}
                Err(err) => return Err(err),
            }
            match SnapshotShellCommand::try_from_input(command, args) {
                Ok(cmd) => return Ok(Self::Snapshot(cmd)),
                Err(err) if err.is::<UnknownCommand>() => {}
                Err(err) => return Err(err),
            }
            return Err(UnknownCommand.into());
        }
        Ok(Self::Sql(SqlCommand::try_from_raw(raw)?))
    }
}

impl Command {
    fn kind(&self) -> CommandKind {
        match self {
            Self::Help(_) => CommandKind::Help,
            Self::Noop => CommandKind::Noop,
            Self::Root(cmd) => CommandKind::Root(cmd.kind()),
            Self::Snapshot(cmd) => CommandKind::Snapshot(cmd.kind()),
            Self::Sql(_) => CommandKind::Sql,
        }
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        match (self, &ctx.shell) {
            (Self::Noop, _) => Ok(()),
            (Self::Help(cmd), _) => cmd.run(ctx),
            (Self::Root(cmd), Shell::Root(_)) => cmd.run(ctx),
            (cmd @ Self::Root(_), _) => Err(CommandUnavailable::new(&cmd, &ctx.shell).into()),
            (Self::Snapshot(cmd), Shell::Snapshot(_)) => cmd.run(ctx),
            (cmd @ Self::Snapshot(_), _) => Err(CommandUnavailable::new(&cmd, &ctx.shell).into()),
            (Self::Sql(cmd), _) => cmd.run(ctx),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{} command unavailable in {} shell", command_kind.name(), shell_kind.name())]
struct CommandUnavailable {
    command_kind: CommandKind,
    shell_kind: ShellKind,
}

impl CommandUnavailable {
    fn new(command: &Command, shell: &Shell) -> Self {
        Self {
            command_kind: command.kind(),
            shell_kind: shell.kind(),
        }
    }
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

    fn kind(&self) -> RootCommandKind {
        match self {
            Self::Log(_) => RootCommandKind::Log,
            Self::Quit(_) => RootCommandKind::Quit,
            Self::Snapshot(_) => RootCommandKind::Snapshot,
            Self::Status(_) => RootCommandKind::Status,
        }
    }

    pub fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Quit(cmd) => cmd.run(ctx),
            Self::Status(cmd) => cmd.run(ctx),
            Self::Log(cmd) => cmd.run(ctx),
            Self::Snapshot(cmd) => cmd.run(ctx),
        }
    }
}

#[derive(Debug)]
pub(crate) enum CommandKind {
    Noop,
    Help,
    Root(RootCommandKind),
    Snapshot(SnapshotShellCommandKind),
    Sql,
}

impl FromStr for CommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        Ok(Self::Root(RootCommandKind::from_str(raw)?))
    }
}

impl CommandKind {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Help => ".help",
            Self::Noop => "no-op",
            Self::Root(kind) => kind.name(),
            Self::Snapshot(kind) => kind.name(),
            Self::Sql => "<sql>",
        }
    }

    fn help(&self) -> Help {
        match self {
            Self::Help => HelpCommand::help(),
            Self::Noop => panic!("cannot get help of no-op"),
            Self::Root(kind) => kind.help(),
            Self::Snapshot(kind) => kind.help(),
            Self::Sql => SqlCommand::help(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown command")]
pub(crate) struct UnknownCommand;

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
            Self::Status => StatusCommand::help(),
            Self::Snapshot => SnapshotCommand::help(),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Log => ".log",
            Self::Quit => ".quit",
            Self::Status => ".status",
            Self::Snapshot => ".snapshot",
        }
    }
}

impl FromStr for RootCommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw {
            ".log" => Ok(Self::Log),
            ".quit" => Ok(Self::Quit),
            ".status" => Ok(Self::Status),
            ".snapshot" => Ok(Self::Snapshot),
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

#[derive(Debug, Default)]
pub struct RootShell {
    prompt: Prompt,
}

impl RootShell {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("dqlite-utils")
            .summary("an observability tool for inspecting the on-disk state of a dqlite node")
            .skip_usage()
            .add_command(HelpCommand::help())
            .add_command(LogCommand::help())
            .add_command(QuitCommand::help())
            .add_command(SnapshotCommand::help())
            .add_command(StatusCommand::help())
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn prompt(&self) -> &Prompt {
        &self.prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    use googletest::expect_that;
    use googletest::matchers::contains_substring;
    use strum::IntoEnumIterator;

    #[googletest::test]
    fn test_all_commands_listed_in_help() {
        let help_output = {
            let mut help_output = Cursor::new(Vec::new());
            RootShell::help().write_to(&mut help_output).unwrap();
            String::try_from(help_output.into_inner()).unwrap()
        };
        for command_kind in RootCommandKind::iter() {
            expect_that!(help_output, contains_substring(command_kind.name()));
        }
    }
}
