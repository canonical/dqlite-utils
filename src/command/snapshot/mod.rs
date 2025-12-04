mod add_server;
mod finish;
mod info;
mod set_index;
mod set_term;
mod set_timestamp;

use std::fmt::Debug;
use std::{fs, io::ErrorKind, path::PathBuf, time::SystemTime};

use anyhow::{Context as _, anyhow};

use crate::command::{UnknownCommand, UnrecognizedArgumentsError};
use crate::dqlite::{DqliteSnapshotBuilder, Empty};
use crate::prompt::Prompt;
use crate::utils::Boomerang;
use crate::{Context, Result, Shell};

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
        let timestamp = SystemTime::now();
        let builder = Boomerang::new(DqliteSnapshotBuilder::new(term, index, timestamp));
        ctx.shell = Shell::Snapshot(SnapshotShell { path, builder });

        ctx.prompt = Prompt::new("snapshot");
        Ok(())
    }
}

pub struct SnapshotShell {
    path: PathBuf,
    builder: Boomerang<DqliteSnapshotBuilder<Empty>>,
}

impl Debug for SnapshotShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { path, builder: _ } = self;
        f.debug_struct("SnapshotShell")
            .field("path", path)
            .finish_non_exhaustive()
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
