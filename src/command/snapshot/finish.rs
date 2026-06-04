use std::cell::OnceCell;
use std::ffi::{CString, OsStr};
use std::fs::{self};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::SystemTime;

use anyhow::{Context as _, anyhow};
use libsqlite3_sys as sqlite3;
use regex::Regex;
use rusqlite::{Connection, TransactionBehavior};

use crate::command::help::Help;
use crate::command::snapshot::{RaftMetadata, RaftServers};
use crate::command::{MissingArgumentError, UnrecognizedArgumentsError};
use crate::dqlite::{DqliteDatabaseWriter, DqliteDir, RaftConfiguration};
use crate::rusqlite_ext::files::{ConnectionFile, ConnectionFilesExt};
use crate::utils::AttachedSchemasConnectionExt;
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
        let shell = ctx.shell.snapshot_mut().ok_or_else(|| {
            anyhow!("internal error: .finish command not called in snapshot shell")
        })?;

        let conn = shell.connection_mut();
        let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let configuration = {
            let RaftServers { servers } = RaftServers::read_from(&txn)?;
            if servers.is_empty() {
                return Err(anyhow!("at least one server required"));
            }
            RaftConfiguration { servers }
        };

        let RaftMetadata {
            term,
            index,
            timestamp,
        } = RaftMetadata::read_from(&txn)?;
        let timestamp = SystemTime::from(timestamp);

        let attached_dbs = {
            let mut attached_dbs = Vec::with_capacity(10);
            let mut schemas = txn.attached_schemas()?;
            let mut schemas_iter = schemas.try_iter()?;
            while let Some(schema) = schemas_iter.next()? {
                let name = schema.name();
                if name == "raft" || name == "temp" {
                    // `raft` only contains metadata, this is encoded elsewhere. `temp` is ignored as it cannot be used as a schema name.
                    continue;
                }
                attached_dbs.push(AttachedDb::new(&txn, name)?)
            }
            attached_dbs
        };

        Self::check_dbs(&attached_dbs, &txn)?;

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
                    .add_databases(attached_dbs.into_iter().map(|db| {
                        let name = CString::new(db.name.as_bytes())
                            .expect("cannot make CString from db name");
                        (name, db)
                    }))
            })
            .create();
        if let Err(err) = res {
            if !dir_preexists {
                fs::remove_dir_all(dir).ok();
            }
            return Err(err);
        }
        txn.rollback()?;

        ctx.shell = Shell::default();

        Ok(())
    }

    fn check_dbs(dbs: &[AttachedDb<'_>], conn: &Connection) -> Result<()> {
        let expected_page_size = OnceCell::new();
        for db in dbs {
            db.check(conn)?;

            let db_page_size = db.page_size(conn)?;
            let expected_page_size = expected_page_size.get_or_init(|| db_page_size);
            if db_page_size != *expected_page_size {
                return Err(anyhow!(
                    "page size mismatch: found both {db_page_size} and {expected_page_size}"
                ));
            }
        }
        Ok(())
    }
}

struct AttachedDb<'conn> {
    name: String,
    main: ConnectionFile<'conn>,
    journal: Option<ConnectionFile<'conn>>,
}

impl<'conn> AttachedDb<'conn> {
    fn new(conn: &'conn Connection, name: &str) -> Result<Self> {
        static VALID_IDENTIFIER: LazyLock<Regex> =
            LazyLock::new(|| Regex::new("^[a-zA-Z0-9]+$").unwrap());
        if !VALID_IDENTIFIER.is_match(name) {
            return Err(anyhow!("cannot use '{name}' as a schema name"));
        }

        let name_os_str = OsStr::new(name);
        let main = conn.main_file(Some(name_os_str))?;
        let journal = conn.journal_file(Some(name_os_str))?;

        let name = name.to_owned();
        Ok(Self {
            name,
            main,
            journal,
        })
    }

    fn check(&self, conn: &Connection) -> Result<()> {
        let name = &self.name;
        if self.journal.is_some() {
            let wal_mode = conn.pragma_query_value(Some(name), "journal_mode", |row| {
                Ok(row.get_ref("journal_mode")?.as_str()? == "wal")
            })?;
            if !wal_mode {
                return Err(anyhow!("schema {name} has non-wal journal present"));
            }
        }
        Ok(())
    }

    fn page_size(&self, conn: &Connection) -> Result<u32> {
        let page_size =
            conn.pragma_query_value(Some(&self.name), "page_size", |row| row.get("page_size"))?;
        Ok(page_size)
    }
}

impl DqliteDatabaseWriter for AttachedDb<'_> {
    fn main_size(&self) -> Result<u64> {
        self.main.len().context("cannot get length of main file")
    }

    fn wal_size(&self) -> Result<u64> {
        let journal = match &self.journal {
            Some(journal) => journal,
            None => return Ok(0),
        };
        journal.len().context("cannot get length of journal file")
    }

    fn write_main(&mut self, out: &mut impl Write) -> Result<()> {
        write_file(
            &mut self.main,
            out,
            Some(|header| {
                // Force WAL mode without mutating input data.
                // See https://sqlite.org/fileformat.html
                const FILE_FORMAT_WRITE_VERSION_OFFSET: usize = 18;
                const FILE_FORMAT_READ_VERSION_OFFSET: usize = 19;
                header[FILE_FORMAT_WRITE_VERSION_OFFSET] = 2;
                header[FILE_FORMAT_READ_VERSION_OFFSET] = 2;
            }),
        )?;
        Ok(())
    }

    fn write_wal(&mut self, out: &mut impl Write) -> Result<()> {
        let journal = match &mut self.journal {
            Some(journal) => journal,
            None => return Ok(()),
        };
        write_file(journal, out, None)?;
        Ok(())
    }
}

fn write_file(
    file: &mut ConnectionFile<'_>,
    out: &mut impl Write,
    patch_header: Option<fn(&mut [u8; 100])>,
) -> Result<()> {
    let len = file.len()? as usize;
    let mut offset = 0;
    let mut buf = [0; 4096];
    while offset < len {
        let to_read = std::cmp::min(buf.len(), len - offset);
        let buf = &mut buf[..to_read];
        match file.read_at(buf, offset as u64) {
            Ok(()) => {}
            Err(err) if err.into_rc() == sqlite3::SQLITE_IOERR_SHORT_READ => {}
            Err(err) => {
                return Err(err.into());
            }
        }
        if offset == 0
            && let Some(patch_header) = &patch_header
        {
            let header = buf.get_mut(..100)
                .context("internal error: header too small")?
                .try_into()
                .expect("internal error: size mismatch");
            patch_header(header);
        }
        out.write_all(buf)?;
        offset += buf.len();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use rusqlite::Connection;

    use crate::dqlite::DqliteDatabaseWriter;

    use super::*;

    #[test]
    fn test_header_wal_mode_patching() {
        let written_db_bytes = {
            // Create database in delete mode.
            let src_dir = tempfile::tempdir().unwrap();
            let db_path = src_dir.path().join("test.sqlite");
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
            conn.execute("CREATE TABLE test(id INTEGER PRIMARY KEY, value TEXT)", ())
                .unwrap();
            conn.execute("INSERT INTO test(value) VALUES ('hello')", ())
                .unwrap();

            // Write database to buffer, the written database should be in WAL mode.
            let mut attached_db = AttachedDb::new(&conn, "main").unwrap();
            let mut written_db_bytes = Vec::new();
            attached_db.write_main(&mut written_db_bytes).unwrap();
            written_db_bytes
        };

        // Check journal mode.
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("output.sqlite");
        {
            let mut out_file = File::create(&out_path).unwrap();
            out_file.write_all(&written_db_bytes).unwrap();
        }
        let out_conn = Connection::open(&out_path).unwrap();
        let wal_mode = out_conn
            .pragma_query_value(None, "journal_mode", |row| Ok(row.get_ref(0)?.as_str()? == "wal"))
            .unwrap();
        assert!(wal_mode);
    }
}
