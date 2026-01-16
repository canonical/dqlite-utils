use std::ffi::{CString, OsString};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
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

        let attached_dbs = {
            let mut attached_dbs = Vec::with_capacity(10);
            let mut stmt = conn.prepare("PRAGMA database_list;")?;
            let mut rows = stmt.query(())?;
            while let Some(row) = rows.next()? {
                let name = row.get("name")?;
                if name == "main" {
                    // Main only contains metadata, this is encoded elsewhere.
                    continue;
                }
                let file = OsString::from_vec(row.get::<_, String>("file")?.into_bytes());
                attached_dbs.push(AttachedDb::new(name, file)?)
            }
            attached_dbs
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
                    .add_databases(attached_dbs.into_iter().map(|db| (db.name.clone(), db)))
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

struct AttachedDb {
    name: CString,
    main: AttachedDbFile,
    wal: Option<AttachedDbFile>,
}

impl AttachedDb {
    fn new(name: String, main_path: OsString) -> Result<Self> {
        let main_size = fs::metadata(&main_path)
            .with_context(|| anyhow!("cannot open {}", main_path.display()))?
            .size() as usize;
        let main = AttachedDbFile {
            path: main_path.clone(),
            size: main_size,
        };

        let wal_path = {
            let mut wal_path = main_path;
            wal_path.push("-wal");
            wal_path
        };
        let wal = match fs::metadata(&wal_path) {
            Ok(metadata) => {
                let size = metadata.size() as usize;
                Some(AttachedDbFile {
                    path: wal_path,
                    size,
                })
            }
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => {
                return Err(err).with_context(|| anyhow!("cannot open {}", wal_path.display()))?;
            }
        };
        let name = CString::new(name)?;
        Ok(Self { name, main, wal })
    }
}

impl DqliteDatabaseWriter for AttachedDb {
    fn main_size(&self) -> usize {
        self.main.size
    }

    fn wal_size(&self) -> usize {
        self.wal.as_ref().map(|wal| wal.size).unwrap_or_default()
    }

    fn write_main(&self, out: &mut impl Write) -> Result<()> {
        self.main.write_to(out)?;
        Ok(())
    }

    fn write_wal(&self, out: &mut impl Write) -> Result<()> {
        if let Some(wal) = &self.wal {
            wal.write_to(out)?;
        }
        Ok(())
    }
}

struct AttachedDbFile {
    path: OsString,
    size: usize,
}

impl AttachedDbFile {
    fn write_to(&self, out: &mut impl Write) -> Result<()> {
        let mut reader = BufReader::new(
            File::open(&self.path)
                .with_context(|| anyhow!("cannot open {}", self.path.display()))?,
        );
        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            out.write_all(buf)?;
            let bytes_written = buf.len(); // Ensures `buf` is dropped before `.consume()`.
            reader.consume(bytes_written);
        }
        Ok(())
    }
}
