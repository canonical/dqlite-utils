use std::cell::OnceCell;
use std::ffi::{CString, OsStr};
use std::fs::{self};
use std::io::{self, BufRead, ErrorKind, IsTerminal, StdinLock, Write};
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
    force_wal_mode: bool,
}

impl FinishCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".finish")
            .summary("validate snapshot and write to disk")
            .add_arg("dir", "the directory to save the snapshot into")
            .add_flag(
                "--force-wal-mode",
                "set journal_mode to WAL for all schemas",
            )
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let mut force_wal_mode = false;
        let mut dir = None;
        for arg in args {
            match arg.as_str() {
                "--force-wal-mode" => force_wal_mode = true,
                other => {
                    if dir.is_some() {
                        return Err(UnrecognizedArgumentsError(args.to_vec()).into());
                    }
                    dir = Some(PathBuf::from(other));
                }
            }
        }
        let dir = dir.ok_or(MissingArgumentError("dir"))?;
        Ok(Self {
            dir,
            force_wal_mode,
        })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self {
            dir,
            force_wal_mode,
        } = self;
        let shell = ctx.shell.snapshot_mut().ok_or_else(|| {
            anyhow!("internal error: .finish command not called in snapshot shell")
        })?;

        let conn = shell.connection_mut();

        let non_wal_schemas = Self::non_wal_schemas(conn)?;
        if !non_wal_schemas.is_empty() {
            let set_wal_mode = if force_wal_mode {
                true
            } else {
                let mut stdin = io::stdin().lock();
                stdin.is_terminal() && Self::prompt_wal_mode(&mut stdin, &non_wal_schemas)?
            };
            if !set_wal_mode {
                return Err(anyhow!("some schemas not in WAL mode: {}", non_wal_schemas.join(", ")));
            }
            for schema in non_wal_schemas {
                Self::apply_wal_mode(conn, &schema)?;
            }
            println!("WAL mode set");
        }

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

    fn non_wal_schemas(conn: &mut Connection) -> Result<Vec<String>> {
        let mut ret = Vec::with_capacity(10);
        let mut schemas = conn.attached_schemas()?;
        let mut schemas_iter = schemas.try_iter()?;
        while let Some(schema) = schemas_iter.next()? {
            let name = schema.name();
            if name == "raft" || name == "temp" {
                continue;
            }
            let wal_mode = conn.pragma_query_value(Some(name), "journal_mode", |row| {
                Ok(row.get_ref("journal_mode")?.as_str()? == "wal")
            })?;
            if !wal_mode {
                ret.push(name.to_owned());
            }
        }
        Ok(ret)
    }

    fn prompt_wal_mode(stdin: &mut StdinLock<'_>, non_wal_schemas: &[String]) -> Result<bool> {
        println!(
            "The following schemas must be changed to WAL mode: {}",
            non_wal_schemas.join("\n")
        );
        println!("This operation will modify the attached databases.");
        print!("Set WAL mode? [Y/n] ");
        io::stdout().flush()?;

        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            return Ok(false);
        }
        let input = input.trim().to_lowercase();
        match input.as_str() {
            "" | "y" | "ye" | "yes" => Ok(true),
            "n" | "no" => Ok(false),
            unrecognized => Err(anyhow!("unrecognized response: {unrecognized}")),
        }
    }

    fn apply_wal_mode(conn: &mut Connection, schema: &str) -> Result<()> {
        conn.pragma_update(Some(schema), "journal_mode", "WAL")?;
        let wal_mode = conn.pragma_query_value(Some(schema), "journal_mode", |row| {
            Ok(row.get_ref("journal_mode")?.as_str()? == "wal")
        })?;
        if !wal_mode {
            return Err(anyhow!(
                "cannot set journal mode of schema {schema} to 'wal'"
            ));
        }
        Ok(())
    }

    fn check_dbs(dbs: &[AttachedDb<'_>], conn: &Connection) -> Result<()> {
        let expected_page_size = OnceCell::new();
        for db in dbs {
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
        write_file(&mut self.main, out)?;
        Ok(())
    }

    fn write_wal(&mut self, out: &mut impl Write) -> Result<()> {
        let journal = match &mut self.journal {
            Some(journal) => journal,
            None => return Ok(()),
        };
        write_file(journal, out)?;
        Ok(())
    }
}

fn write_file(file: &mut ConnectionFile<'_>, out: &mut impl Write) -> Result<()> {
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
        out.write_all(buf)?;
        offset += buf.len();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::snapshot::SnapshotShell;
    use googletest::expect_that;
    use googletest::matchers::{container_eq, eq};
    use tempfile::NamedTempFile;

    #[googletest::test]
    fn test_non_wal_schemas() {
        let mut conn = SnapshotShell::open_connection().unwrap();
        let mut db_files = Vec::new();

        for (schema_name, journal_mode) in [("db1", "DELETE"), ("db2", "WAL")] {
            let db_file = NamedTempFile::new().unwrap();
            let path = db_file.path().to_path_buf();

            {
                let disk_conn = Connection::open(&path).unwrap();
                disk_conn
                    .pragma_update(None, "journal_mode", journal_mode)
                    .unwrap();
                disk_conn
                    .execute("CREATE TABLE data(id INTEGER PRIMARY KEY)", ())
                    .unwrap();
            }

            conn.execute_batch(&format!(
                "ATTACH DATABASE {:?} AS {schema_name};",
                path.display()
            ))
            .unwrap();
            db_files.push(db_file);
        }

        let non_wal = FinishCommand::non_wal_schemas(&mut conn).unwrap();
        expect_that!(non_wal, container_eq(vec!["db1".to_string()]));
    }

    #[googletest::test]
    fn test_apply_wal_mode() {
        let mut conn = SnapshotShell::open_connection().unwrap();
        let mut db_files = Vec::new();

        for (schema_name, journal_mode) in [("db1", "DELETE")] {
            let db_file = NamedTempFile::new().unwrap();
            let path = db_file.path().to_path_buf();

            {
                let disk_conn = Connection::open(&path).unwrap();
                disk_conn
                    .pragma_update(None, "journal_mode", journal_mode)
                    .unwrap();
                disk_conn
                    .execute("CREATE TABLE data(id INTEGER PRIMARY KEY)", ())
                    .unwrap();
            }

            conn.execute_batch(&format!(
                "ATTACH DATABASE {:?} AS {schema_name};",
                path.display()
            ))
            .unwrap();
            db_files.push(db_file);
        }

        FinishCommand::apply_wal_mode(&mut conn, "db1").unwrap();

        let mode_after: String = conn
            .pragma_query_value(Some("db1"), "journal_mode", |row| row.get("journal_mode"))
            .unwrap();
        expect_that!(mode_after, eq("wal"));
    }
}
