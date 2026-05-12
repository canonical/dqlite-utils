mod config;
mod help;
mod log;
mod open;
pub(crate) mod quit;
mod snapshot;
mod sql;
mod status;

pub(crate) use self::help::Help;
pub(crate) use self::open::{DqliteDirContent, OpenShell};
pub(crate) use self::snapshot::SnapshotShell;

use std::str::FromStr;

use anyhow::Error;
use strum::EnumIter;

use crate::command::open::{OpenCommand, OpenShellCommand, OpenShellCommandKind};
use crate::prompt::Prompt;
use crate::{Context, Result, Shell, ShellKind};

use self::help::HelpCommand;
use self::log::LogCommand;
use self::quit::QuitCommand;
use self::snapshot::{SnapshotCommand, SnapshotShellCommand, SnapshotShellCommandKind};
use self::sql::SqlCommand;
use self::status::StatusCommand;
use self::config::ConfigCommand;

#[derive(Debug)]
pub enum Command {
    Noop,
    Help(HelpCommand),
    Root(RootCommand),
    Snapshot(SnapshotShellCommand),
    Open(OpenShellCommand),
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

            let ret = match command.parse()? {
                CommandKind::Noop => Self::Noop,
                CommandKind::Help => Self::Help(HelpCommand::try_from_args(args)?),
                CommandKind::Root(kind) => Self::Root(RootCommand::try_from_input(kind, args)?),
                CommandKind::Snapshot(kind) => {
                    Self::Snapshot(SnapshotShellCommand::try_from_input(kind, args)?)
                }
                CommandKind::Open(kind) => {
                    Self::Open(OpenShellCommand::try_from_input(kind, args)?)
                }
                CommandKind::Sql => unreachable!(), // No SQL command starts with a `.`
            };
            return Ok(ret);
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
            Self::Open(cmd) => CommandKind::Open(cmd.kind()),
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
            (Self::Open(cmd), Shell::Open(_)) => cmd.run(ctx),
            (cmd @ Self::Open(_), _) => Err(CommandUnavailable::new(&cmd, &ctx.shell).into()),
            (Self::Sql(cmd), Shell::Snapshot(_) | Shell::Open(_)) => cmd.run(ctx),
            (cmd @ Self::Sql(_), _) => Err(CommandUnavailable::new(&cmd, &ctx.shell).into()),
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
    Config(ConfigCommand),
    Log(LogCommand),
    Quit(QuitCommand),
    Snapshot(SnapshotCommand),
    Open(OpenCommand),
    Status(StatusCommand),
}

impl RootCommand {
    fn try_from_input(kind: RootCommandKind, args: &[String]) -> Result<Self> {
        match kind {
            RootCommandKind::Config => Ok(Self::Config(ConfigCommand::try_from_args(args)?)),
            RootCommandKind::Log => Ok(Self::Log(LogCommand::try_from_args(args)?)),
            RootCommandKind::Quit => Ok(Self::Quit(QuitCommand::try_from_args(args)?)),
            RootCommandKind::Snapshot => Ok(Self::Snapshot(SnapshotCommand::try_from_args(args)?)),
            RootCommandKind::Open => Ok(Self::Open(OpenCommand::try_from_args(args)?)),
            RootCommandKind::Status => Ok(Self::Status(StatusCommand::try_from_args(args)?)),
        }
    }

    fn kind(&self) -> RootCommandKind {
        match self {
            Self::Config(_) => RootCommandKind::Config,
            Self::Log(_) => RootCommandKind::Log,
            Self::Quit(_) => RootCommandKind::Quit,
            Self::Open(_) => RootCommandKind::Open,
            Self::Snapshot(_) => RootCommandKind::Snapshot,
            Self::Status(_) => RootCommandKind::Status,
        }
    }

    pub fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Config(cmd) => cmd.run(ctx),
            Self::Quit(cmd) => cmd.run(ctx),
            Self::Status(cmd) => cmd.run(ctx),
            Self::Log(cmd) => cmd.run(ctx),
            Self::Snapshot(cmd) => cmd.run(ctx),
            Self::Open(cmd) => cmd.run(ctx),
        }
    }
}

#[derive(Debug)]
pub(crate) enum CommandKind {
    Noop,
    Help,
    Root(RootCommandKind),
    Snapshot(SnapshotShellCommandKind),
    Open(OpenShellCommandKind),
    Sql,
}

impl FromStr for CommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw {
            "" => return Ok(Self::Noop),
            ".help" => return Ok(Self::Help),
            _ => {}
        }
        if !raw.starts_with('.') {
            return Ok(Self::Sql);
        }
        match RootCommandKind::from_str(raw) {
            Ok(cmd) => return Ok(Self::Root(cmd)),
            Err(err) if err.is::<UnknownCommand>() => {}
            Err(err) => return Err(err),
        }
        match SnapshotShellCommandKind::from_str(raw) {
            Ok(cmd) => return Ok(Self::Snapshot(cmd)),
            Err(err) if err.is::<UnknownCommand>() => {}
            Err(err) => return Err(err),
        }
        match OpenShellCommandKind::from_str(raw) {
            Ok(cmd) => return Ok(Self::Open(cmd)),
            Err(err) if err.is::<UnknownCommand>() => {}
            Err(err) => return Err(err),
        }
        Err(UnknownCommand.into())
    }
}

impl CommandKind {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Help => ".help",
            Self::Noop => "no-op",
            Self::Root(kind) => kind.name(),
            Self::Snapshot(kind) => kind.name(),
            Self::Open(kind) => kind.name(),
            Self::Sql => "<sql>",
        }
    }

    fn help(&self) -> Option<Help> {
        match self {
            Self::Help => Some(HelpCommand::help()),
            Self::Noop => None,
            Self::Root(kind) => Some(kind.help()),
            Self::Snapshot(kind) => Some(kind.help()),
            Self::Open(kind) => Some(kind.help()),
            Self::Sql => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown command")]
pub(crate) struct UnknownCommand;

#[derive(Debug, Eq, PartialEq, EnumIter)]
pub(crate) enum RootCommandKind {
    Config,
    Log,
    Quit,
    Snapshot,
    Open,
    Status,
}

impl RootCommandKind {
    pub(crate) fn help(&self) -> Help {
        match self {
            Self::Config => ConfigCommand::help(),
            Self::Log => LogCommand::help(),
            Self::Quit => QuitCommand::help(),
            Self::Status => StatusCommand::help(),
            Self::Snapshot => SnapshotCommand::help(),
            Self::Open => OpenCommand::help(),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Config => ".config",
            Self::Log => ".log",
            Self::Quit => ".quit",
            Self::Status => ".status",
            Self::Snapshot => ".snapshot",
            Self::Open => ".open",
        }
    }
}

impl FromStr for RootCommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw {
            ".config" => Ok(Self::Config),
            ".log" => Ok(Self::Log),
            ".quit" => Ok(Self::Quit),
            ".status" => Ok(Self::Status),
            ".snapshot" => Ok(Self::Snapshot),
            ".open" => Ok(Self::Open),
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
            .add_command(ConfigCommand::help())
            .add_command(LogCommand::help())
            .add_command(QuitCommand::help())
            .add_command(SnapshotCommand::help())
            .add_command(StatusCommand::help())
            .add_command(OpenCommand::help())
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
