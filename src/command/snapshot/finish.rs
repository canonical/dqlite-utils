use std::ffi::CString;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context as _, anyhow};

use crate::command::help::Help;
use crate::command::snapshot::{RaftMetadata, RaftServers};
use crate::command::{MissingArgumentError, UnrecognizedArgumentsError};
use crate::dqlite::{DqliteDatabaseWriter, DqliteDir, RaftConfiguration};
use crate::{Context, Result, Shell};

#[derive(Debug)]
pub(crate) struct FinishCommand {
    dir: PathBuf,
}

impl FinishCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".finish")
            .summary("validate snapshot and write to disk")
            .add_arg("dir", "the directory to save the snapshot into")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let dir = match args {
            [] => return Err(MissingArgumentError("dir").into()),
            [dir] => dir,
            [_, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let dir = PathBuf::from(dir);
        Ok(Self { dir })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { dir } = self;
        let shell = ctx.shell.snapshot().ok_or_else(|| {
            anyhow!("internal error: .finish command not called in snapshot shell")
        })?;

        let conn = shell.connection();

        let configuration = {
            let RaftServers { servers } = RaftServers::read_from(conn)?;
            if servers.is_empty() {
                return Err(anyhow!("at least one server required"));
            }
            RaftConfiguration { servers }
        };

        let RaftMetadata {
            term,
            index,
            timestamp,
        } = RaftMetadata::read_from(conn)?;
        let timestamp = SystemTime::from(timestamp);

        // Heuristic to ensure clean directory. Clearly there's a TOCTOU issue here,
        // but if a user chooses to write a snapshot into an actively-changing
        // directory then on their head, be it.
        let dir_preexists = match fs::read_dir(&dir) {
            Ok(mut dir_reader) => {
                if dir_reader.next().is_some() {
                    return Err(anyhow!("directory not empty"))
                        .with_context(|| anyhow!("cannot write snapshot into {}", dir.display()));
                }
                true
            }
            Err(err) if err.kind() == ErrorKind::NotFound => false,
            Err(err) => {
                return Err(err)
                    .with_context(|| anyhow!("cannot write snapshot into {}", dir.display()));
            }
        };

        let res = DqliteDir::creator(&dir)
            .with_snapshot(move |s| {
                s.with_term(term)
                    .with_index(index)
                    .with_timestamp(timestamp)
                    .with_configuration(configuration)
                    .add_database(
                        CString::new("placeholder db".as_bytes().to_owned())
                            .expect("internal error: CString invalid"),
                        PlaceholderDb,
                    )
            })
            .create();
        if let Err(err) = res {
            if !dir_preexists {
                fs::remove_dir_all(dir).ok();
            }
            return Err(err);
        }

        ctx.shell = Shell::default();

        Ok(())
    }
}

struct PlaceholderDb;

impl DqliteDatabaseWriter for PlaceholderDb {
    fn main_size(&self) -> usize {
        0
    }

    fn wal_size(&self) -> usize {
        0
    }

    fn write_main(&self, _out: &mut impl Write) -> Result<()> {
        Ok(())
    }

    fn write_wal(&self, _out: &mut impl Write) -> Result<()> {
        Ok(())
    }
}
