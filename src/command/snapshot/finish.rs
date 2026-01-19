use std::ffi::CString;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context as _, anyhow};
use fallible_iterator::FallibleIterator;
use indoc::indoc;
use rusqlite::{Connection, Error as RusqliteError};
use time::UtcDateTime;
use time::format_description::well_known::Iso8601;

use crate::command::help::Help;
use crate::command::{MissingArgumentError, UnrecognizedArgumentsError};
use crate::dqlite::{DqliteDatabaseWriter, DqliteDir, RaftConfiguration, RaftServer};
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

        let RaftMetadata {
            term,
            index,
            timestamp,
        } = RaftMetadata::read_from(&conn)?;
        let timestamp = SystemTime::from(timestamp);

        let configuration = {
            let mut stmt = conn.prepare(indoc! {"
                SELECT id, address, role
                FROM raft_servers;
            "})?;
            let servers: Vec<_> = stmt
                .query(())?
                .map(|row| {
                    Ok(RaftServer {
                        id: row.get("id")?,
                        address: row.get("address")?,
                        role: row.get("role")?,
                    })
                })
                .collect()?;
            if servers.is_empty() {
                return Err(anyhow!("at least one server required"));
            }
            RaftConfiguration { servers }
        };

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

pub(crate) struct RaftMetadata {
    pub(crate) term: u64,
    pub(crate) index: u64,
    pub(crate) timestamp: UtcDateTime,
}

impl RaftMetadata {
    pub(crate) fn read_from(conn: &Connection) -> Result<Self> {
        let (term, index, timestamp) = conn.query_one(
            indoc! {"
                SELECT raft_term, raft_index, timestamp
                FROM raft_data
            "},
            (),
            |row| {
                let term: u64 = row.get("raft_term")?;
                let index: u64 = row.get("raft_index")?;
                let timestamp =
                    UtcDateTime::parse(row.get_ref("timestamp")?.as_str()?, &Iso8601::DEFAULT)
                        .map_err(|err| RusqliteError::UserFunctionError(err.into()))?;
                Ok((term, index, timestamp))
            },
        )?;
        Ok(Self {
            term,
            index,
            timestamp,
        })
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
