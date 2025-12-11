mod abort;
mod info;

use std::fmt::{self, Display};
use std::str::FromStr;

use indoc::writedoc;
use strum::EnumIter;
use time::UtcDateTime;
use time::format_description::well_known::Iso8601;

use crate::command::help::Help;
use crate::command::{UnknownCommand, UnrecognizedArgumentsError};
use crate::dqlite::RaftServer;
use crate::prompt::Prompt;
use crate::{Context, Error, Result, Shell};

use self::abort::AbortCommand;
use self::info::InfoCommand;

#[derive(Debug)]
pub(crate) struct SnapshotCommand;

impl SnapshotCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot")
            .summary("Enter snapshot-creation shell")
            .add_arg("dir", "the directory to save the snapshot into")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self = self;
        ctx.shell = Shell::Snapshot(SnapshotShell::new());
        Ok(())
    }
}

#[derive(Debug)]
pub struct SnapshotShell {
    snapshot: ShellSnapshotContext,
    prompt: Prompt,
}

impl SnapshotShell {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot shell")
            .summary("incrementally create a snapshot")
            .skip_usage()
            .add_command(AbortCommand::help())
            .add_command(InfoCommand::help())
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn new() -> Self {
        let snapshot = ShellSnapshotContext::new();
        let prompt = Prompt::new("snapshot");
        Self { snapshot, prompt }
    }

    pub(crate) fn prompt(&self) -> &Prompt {
        &self.prompt
    }
}

#[derive(Debug)]
struct ShellSnapshotContext {
    #[allow(unused)]
    term: u64,
    #[allow(unused)]
    index: u64,
    #[allow(unused)]
    timestamp: UtcDateTime,
    #[allow(unused)]
    configuration: ShellSnapshotRaftConfiguration,
}

impl ShellSnapshotContext {
    fn new() -> Self {
        Self {
            term: 1,
            index: 1,
            timestamp: UtcDateTime::now(),
            configuration: ShellSnapshotRaftConfiguration::new(),
        }
    }
}

impl Display for ShellSnapshotContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ShellSnapshotContext {
            term,
            index,
            timestamp,
            configuration,
        } = self;
        let timestamp = timestamp
            .format(&Iso8601::DEFAULT)
            .map_err(|_| fmt::Error)?;
        writedoc!(
            f,
            "
                term: {term}
                index: {index}
                timestamp: {timestamp}
            "
        )?;

        let ShellSnapshotRaftConfiguration { servers } = configuration;
        if servers.is_empty() {
            writeln!(f, "configuration: -")?;
        } else {
            writeln!(f, "configuration:")?;
            for server in servers {
                let RaftServer { id, address, role } = server;
                writedoc!(
                    f,
                    "
                        - id: {id}
                          address: {address}
                          role: {role}
                    "
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct ShellSnapshotRaftConfiguration {
    #[allow(unused)]
    servers: Vec<RaftServer>,
}

impl ShellSnapshotRaftConfiguration {
    fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug)]
pub(crate) enum SnapshotShellCommand {
    Abort(AbortCommand),
    Info(InfoCommand),
}

impl SnapshotShellCommand {
    pub(crate) fn kind(&self) -> SnapshotShellCommandKind {
        use SnapshotShellCommandKind as Ssck;
        match self {
            Self::Abort(_) => Ssck::Abort,
            Self::Info(_) => Ssck::Info,
        }
    }

    pub(crate) fn try_from_input(command: &str, args: &[String]) -> Result<Self> {
        use SnapshotShellCommandKind as Ssck;
        match command.parse()? {
            Ssck::Abort => Ok(Self::Abort(AbortCommand::try_from_args(args)?)),
            Ssck::Info => Ok(Self::Info(InfoCommand::try_from_args(args)?)),
        }
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Abort(cmd) => cmd.run(ctx),
            Self::Info(cmd) => cmd.run(ctx),
        }
    }
}

#[derive(Debug, EnumIter)]
pub(crate) enum SnapshotShellCommandKind {
    Abort,
    Info,
}

impl SnapshotShellCommandKind {
    pub(crate) fn help(&self) -> Help {
        match self {
            Self::Abort => AbortCommand::help(),
            Self::Info => InfoCommand::help(),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Abort => ".abort",
            Self::Info => ".info",
        }
    }
}

impl FromStr for SnapshotShellCommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw {
            ".abort" => Ok(Self::Abort),
            ".info" => Ok(Self::Info),
            _ => Err(UnknownCommand.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use googletest::expect_that;
    use googletest::matchers::contains_substring;
    use strum::IntoEnumIterator;

    use super::*;

    #[googletest::test]
    fn test_all_commands_listed_in_help() {
        let help_output = {
            let mut help_output = Cursor::new(Vec::new());
            SnapshotShell::help().write_to(&mut help_output).unwrap();
            String::try_from(help_output.into_inner()).unwrap()
        };
        for command_kind in SnapshotShellCommandKind::iter() {
            expect_that!(help_output, contains_substring(command_kind.name()));
        }
    }
}
