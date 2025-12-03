use std::{fs, io::ErrorKind, path::PathBuf, time::SystemTime};

use anyhow::{Context as _, anyhow};

use crate::command::{UnknownCommand, UnrecognizedArgumentsError};
use crate::dqlite::DqliteSnapshotBuilder;
use crate::prompt::Prompt;
use crate::{Context, Result, Shell};

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
        let timestamp = SystemTime::now();

        let dqlite = ctx.dqlite()?;
        let term = dqlite.term();
        let index = dqlite.first_index();
        let builder = DqliteSnapshotBuilder::new(term, index, timestamp);

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

        ctx.shell = Shell::Snapshot(SnapshotShell { path, builder });
        ctx.prompt = Prompt::new("snapshot");
        Ok(())
    }
}

#[derive(Debug)]
pub struct SnapshotShell {
    path: PathBuf,
    builder: DqliteSnapshotBuilder<()>,
}

pub enum SnapshotShellCommand {
    Finish,
}

impl SnapshotShellCommand {
    pub fn try_from_input(command: &str, args: &[String]) -> Result<Self> {
        match command {
            "finish" => Ok(Self::Finish),
            _ => Err(UnknownCommand.into()),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Finish => "finish",
        }
    }

    pub fn run(self, _ctx: &mut Context) -> Result<()> {
        todo!("run snapshot command")
    }
}
