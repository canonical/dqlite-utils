mod add_server;
mod finish;
mod info;
mod set_index;
mod set_term;
mod set_timestamp;

use std::fmt::{self, Debug, Display};
use std::str::FromStr;
use std::{fs, io::ErrorKind, path::PathBuf};

use anyhow::{Context as _, anyhow};
use indoc::writedoc;
use strum::{EnumIter, IntoEnumIterator};
use time::UtcDateTime;
use time::format_description::well_known::Iso8601;

use crate::command::help::Help;
use crate::command::{UnknownCommand, UnrecognizedArgumentsError};
use crate::dqlite::{RaftConfiguration, RaftServer};
use crate::prompt::Prompt;
use crate::{Context, Error, Result, Shell};

use self::add_server::AddServerCommand;
use self::finish::FinishCommand;
use self::info::InfoCommand;
use self::set_index::SetIndexCommand;
use self::set_term::SetTermCommand;
use self::set_timestamp::SetTimestampCommand;

#[derive(Debug)]
pub(crate) struct SnapshotCommand {
    snapshot_path: PathBuf,
}

impl SnapshotCommand {
    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let snapshot_path = match args {
            [] => return Err(anyhow!("specify file path")),
            [path] => path,
            [_, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let snapshot_path = PathBuf::from(snapshot_path);
        Ok(Self { snapshot_path })
    }

    pub(crate) const SUMMARY: &'static str = "Enter snapshot-creation shell";

    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot")
            .summary(Self::SUMMARY)
            .add_arg("dir", "the directory to save the snapshot into")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self {
            snapshot_path: path,
        } = self;
        match fs::read_dir(&path) {
            Ok(mut dir_reader) => {
                if dir_reader.next().is_some() {
                    return Err(anyhow!("directory not empty"))
                        .with_context(|| anyhow!("cannot snapshot into {}", path.display()));
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| anyhow!("cannot snapshot into {}", path.display()));
            }
        }

        let dqlite = ctx.dqlite()?;
        let term = dqlite.term();
        let index = dqlite.first_index();
        let timestamp = UtcDateTime::now();
        let builder = ShellSnapshotContext {
            term,
            index,
            timestamp,
            configuration: None,
        };
        ctx.shell = Shell::Snapshot(SnapshotShell {
            path,
            snapshot: builder,
        });

        ctx.prompt = Prompt::new("snapshot");
        Ok(())
    }
}

pub struct SnapshotShell {
    path: PathBuf,
    snapshot: ShellSnapshotContext,
}

impl SnapshotShell {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot shell")
            .summary("incrementally create a snapshot")
            .skip_usage()
            .add_command(AddServerCommand::help())
            .add_command(FinishCommand::help())
            .add_command(InfoCommand::help())
            .add_command(SetIndexCommand::help())
            .add_command(SetTermCommand::help())
            .add_command(SetTimestampCommand::help())
            .build()
            .expect("internal error: help invalid")
    }
}

impl Debug for SnapshotShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { path, snapshot: _ } = self;
        f.debug_struct("SnapshotShell")
            .field("path", path)
            .finish_non_exhaustive()
    }
}

struct ShellSnapshotContext {
    term: u64,
    index: u64,
    timestamp: UtcDateTime,
    configuration: Option<ShellSnapshotRaftConfiguration>,
}

impl Display for ShellSnapshotContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
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
        if let Some(configuration) = configuration {
            writeln!(f, "configuration:")?;
            for server in &configuration.servers {
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
        } else {
            writeln!(f, "configuration: -")?;
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
struct ShellSnapshotRaftConfiguration {
    servers: Vec<RaftServer>,
}

impl From<ShellSnapshotRaftConfiguration> for RaftConfiguration {
    fn from(configuration: ShellSnapshotRaftConfiguration) -> Self {
        let ShellSnapshotRaftConfiguration { servers } = configuration;
        Self { servers }
    }
}

pub enum SnapshotShellCommand {
    AddServer(AddServerCommand),
    Finish(FinishCommand),
    Info(InfoCommand),
    SetIndex(SetIndexCommand),
    SetTerm(SetTermCommand),
    SetTimestamp(SetTimestampCommand),
}

impl SnapshotShellCommand {
    pub fn try_from_input(command: &str, args: &[String]) -> Result<Self> {
        match command {
            "add-server" => Ok(Self::AddServer(AddServerCommand::try_from_args(args)?)),
            "finish" => Ok(Self::Finish(FinishCommand::try_from_args(args)?)),
            "info" => Ok(Self::Info(InfoCommand::try_from_args(args)?)),
            "set-index" => Ok(Self::SetIndex(SetIndexCommand::try_from_args(args)?)),
            "set-term" => Ok(Self::SetTerm(SetTermCommand::try_from_args(args)?)),
            "set-timestamp" => Ok(Self::SetTimestamp(SetTimestampCommand::try_from_args(
                args,
            )?)),
            _ => Err(UnknownCommand.into()),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::AddServer(_) => "add-server",
            Self::Finish(_) => "finish",
            Self::Info(_) => "info",
            Self::SetIndex(_) => "set-index",
            Self::SetTerm(_) => "set-term",
            Self::SetTimestamp(_) => "set-timestamp",
        }
    }

    pub fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::AddServer(cmd) => cmd.run(ctx),
            Self::Finish(cmd) => cmd.run(ctx),
            Self::Info(cmd) => cmd.run(ctx),
            Self::SetIndex(cmd) => cmd.run(ctx),
            Self::SetTerm(cmd) => cmd.run(ctx),
            Self::SetTimestamp(cmd) => cmd.run(ctx),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum SnapshotShellCommandKind {
    AddServer,
    Finish,
    Info,
    SetIndex,
    SetTerm,
    SetTimestamp,
}

impl SnapshotShellCommandKind {
    pub(crate) fn help(&self) -> Help {
        match self {
            Self::AddServer => AddServerCommand::help(),
            Self::Finish => FinishCommand::help(),
            Self::Info => InfoCommand::help(),
            Self::SetIndex => SetIndexCommand::help(),
            Self::SetTerm => SetTermCommand::help(),
            Self::SetTimestamp => SetTimestampCommand::help(),
        }
    }
}

impl FromStr for SnapshotShellCommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw {
            "add-server" => Ok(Self::AddServer),
            "finish" => Ok(Self::Finish),
            "info" => Ok(Self::Info),
            "set-index" => Ok(Self::SetIndex),
            "set-term" => Ok(Self::SetTerm),
            "set-timestamp" => Ok(Self::SetTimestamp),
            _ => Err(UnknownCommand.into()),
        }
    }
}
