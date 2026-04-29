use std::borrow::Cow;
use std::ffi::OsStr;
use std::io::Write;
use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use libsqlite3_sys as sqlite3;
use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{Connection, Statement, Transaction, named_params};

use crate::dqlite::{
    DqliteDatabaseLoader, DqliteDir, DqliteLogEntryContent, DqliteSegment, DqliteSnapshotLoader,
};
use crate::rusqlite_ext::vfs::{
    FileType, IoCapabilities, LockLevel, OpenFlags, PragmaError, PragmaResult, SyncOptions, Vfs,
    VfsFile, VfsPath,
};
use crate::rusqlite_ext::{self, SqliteError};

const HEADER_SIZE: usize = 100;
const SCHEMA: &str = "
    CREATE TABLE raft_log(
        idx INTEGER NOT NULL PRIMARY KEY CHECK(idx > 0),
        term INTEGER NOT NULL CHECK(term >= 0)
    ) STRICT;

    CREATE TABLE raft_configuration(
        log_idx INTEGER NULL -- NULL means this is the snapshotted configuration
            REFERENCES raft_log(idx)
            ON DELETE CASCADE
            ON UPDATE RESTRICT,
        id INTEGER NOT NULL,
        address TEXT NOT NULL,
        role INTEGER NOT NULL,
        role_name TEXT AS (CASE
            WHEN role = 0 THEN 'Standby'
            WHEN role = 1 THEN 'Voter'
            WHEN role = 2 THEN 'Spare'
            ELSE 'Unknown'
        END),
        PRIMARY KEY(log_idx, id)
    );

    CREATE TABLE dqlite_transaction(
        log_idx INTEGER NOT NULL
            REFERENCES raft_log(idx)
            ON DELETE CASCADE
            ON UPDATE RESTRICT,
        database TEXT NOT NULL,
        page_number INTEGER NOT NULL,
        page_id INTEGER NOT NULL
            REFERENCES dqlite_page(id)
            ON DELETE CASCADE
            ON UPDATE RESTRICT,
        PRIMARY KEY(log_idx, database, page_number)
    ) STRICT;

    CREATE TABLE dqlite_database(
        database TEXT NOT NULL,
        page_number INTEGER NOT NULL,
        page_id INTEGER NOT NULL
            REFERENCES dqlite_page(id)
            ON DELETE CASCADE
            ON UPDATE RESTRICT,
        PRIMARY KEY(database, page_number)
    ) STRICT;

    CREATE INDEX dqlite_database__page_id_idx ON dqlite_database(page_id);

    CREATE TABLE dqlite_page(
        id INTEGER NOT NULL PRIMARY KEY,
        data BLOB NOT NULL
    ) STRICT;
";

#[derive(Debug)]
pub struct DqliteVfs {
    connection: Arc<Mutex<Connection>>,
    databases: Vec<String>,
    page_size: usize,
    first_index: u64,
    last_index: u64,
    current_index: AtomicU64,
}

impl DqliteVfs {
    pub fn from_dir(dqlite: &DqliteDir, page_size: usize) -> Result<Self> {
        let mut conn = Connection::open_with_flags(
            "",
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        conn.pragma_update(None, "foreign_keys", true)?;

        let first_index = {
            let txn = conn.transaction()?;
            Self::create_schema(&txn)?;

            // This is different from the "first_index" of the dqlite log, as the first index
            // cannot be read as it might be before the earliest snapshot, which means that it
            // contains a diff over an older snapshot that doesn't exist anymore and can't be
            // read as such.
            let first_index = Self::load_first_snapshot(&txn, dqlite, page_size)?;
            Self::load_segments_from(&txn, dqlite, first_index)?;
            txn.commit()?;
            first_index
        };

        let databases = {
            let mut stmt = conn.prepare(
                "
                    SELECT database
                    FROM dqlite_database

                    UNION

                    SELECT database
                    FROM dqlite_transaction
                ",
            )?;
            stmt.query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };

        let last_index = dqlite.current_index()?;
        let connection = Arc::new(Mutex::new(conn));
        let current_index = AtomicU64::new(last_index);
        Ok(Self {
            connection,
            databases,
            page_size,
            first_index,
            last_index,
            current_index,
        })
    }

    fn create_schema(txn: &Transaction) -> Result<()> {
        txn.execute_batch(SCHEMA)?;
        Ok(())
    }

    fn load_first_snapshot(txn: &Transaction, dqlite: &DqliteDir, page_size: usize) -> Result<u64> {
        if let Some(snapshot) = dqlite.snapshots().first() {
            {
                // Load the configuration
                let mut config_stmt = txn.prepare(
                    "
                        INSERT INTO raft_configuration(id, address, role)
                        VALUES (:id, :address, :role)
                    ",
                )?;
                for server in &snapshot.configuration.servers {
                    config_stmt.execute(named_params![
                        ":id": server.id as i64,
                        ":address": server.address,
                        ":role": server.role.as_raw(),
                    ])?;
                }
            }

            // And the database data
            snapshot.read(SnapshotLoader::new(txn, page_size)?)?;
            Ok(snapshot.index)
        } else if dqlite.first_index() == 1 {
            // No snapshot and log starts at index 1
            Ok(dqlite.first_index())
        } else {
            Err(anyhow::anyhow!(
                "corrupted folder: no snapshot found and log doesn't start at index 1"
            ))
        }
    }

    fn load_segments_from(txn: &Transaction, dqlite: &DqliteDir, start_index: u64) -> Result<()> {
        let mut log_entry_stmt = txn.prepare(
            "
                INSERT INTO raft_log(idx, term)
                VALUES (:idx, :term)
            ",
        )?;
        let mut config_stmt = txn.prepare(
            "
                INSERT INTO raft_configuration(log_idx, id, address, role)
                VALUES (:log_idx, :id, :address, :role)
            ",
        )?;
        let mut page_stmt = txn.prepare(
            "
                INSERT INTO dqlite_page(data)
                VALUES (:data)
            ",
        )?;
        let mut db_stmt = txn.prepare(
            "
                INSERT INTO dqlite_transaction(log_idx, database, page_number, page_id)
                VALUES (:log_idx, :database, :page_number, :page_id)
            ",
        )?;
        let mut index = dqlite.first_index();
        for segment in dqlite.segments() {
            if let DqliteSegment::Closed { indexes, .. } = segment
                && *indexes.end() <= start_index
            {
                index = *indexes.end() + 1;
                continue;
            }

            let entries = segment.entries()?;
            for (i, entry) in entries.iter().enumerate() {
                let entry_index = index + i as u64;
                if entry_index <= start_index {
                    continue;
                }
                log_entry_stmt.execute(named_params![
                    ":idx": entry_index as i64,
                    ":term": entry.term as i64,
                ])?;

                match &entry.content {
                    DqliteLogEntryContent::Change(configuration) => {
                        for server in &configuration.servers {
                            config_stmt.execute(named_params![
                                ":log_idx": entry_index as i64,
                                ":id": server.id as i64,
                                ":address": server.address,
                                ":role": server.role.as_raw(),
                            ])?;
                        }
                    }
                    DqliteLogEntryContent::CommandFrames {
                        filename, frames, ..
                    } => {
                        for frame in frames {
                            page_stmt.execute(named_params![
                                ":data": &frame.data,
                            ])?;
                            let page_id = txn.last_insert_rowid();
                            db_stmt.execute(named_params![
                                ":log_idx": entry_index as i64,
                                ":database": ToSqlOutput::Borrowed(ValueRef::Text(filename.as_bytes())),
                                ":page_number": frame.page_number as i64,
                                ":page_id": page_id,
                            ])?;
                        }
                    }
                    _ => {}
                }
            }
            index += entries.len() as u64;
        }
        Ok(())
    }

    pub fn set_current_index(&self, index: u64) -> Result<()> {
        if index < self.first_index || index > self.last_index {
            return Err(anyhow::anyhow!(
                "invalid index {}: must be between {} and {}",
                index,
                self.first_index,
                self.last_index
            ));
        }
        self.current_index.store(index, Ordering::SeqCst);
        Ok(())
    }

    pub fn databases(&self) -> Result<Vec<String>> {
        let conn = self
            .connection
            .lock()
            .expect("internal error: poisoned mutex");
        let raft_index = self.current_index.load(Ordering::SeqCst);
        let mut result = Vec::new();
        for db in &self.databases {
            if read_database_size(&conn, raft_index, db)? > 0 {
                result.push(db.clone());
            }
        }
        Ok(result)
    }
}

struct SnapshotLoader<'stmt> {
    conn: &'stmt Connection,
    page_stmt: Statement<'stmt>,
    db_stmt: Statement<'stmt>,
    page_size: usize,
}

impl<'stmt> SnapshotLoader<'stmt> {
    fn new(conn: &'stmt Connection, page_size: usize) -> Result<Self> {
        let page_stmt = conn.prepare(
            "
                INSERT INTO dqlite_page(data)
                VALUES (:data)
            ",
        )?;
        let db_stmt = conn.prepare(
            "
                REPLACE INTO dqlite_database(database, page_number, page_id)
                VALUES (:database, :page_number, :page_id)
            ",
        )?;
        Ok(SnapshotLoader {
            conn,
            page_stmt,
            db_stmt,
            page_size,
        })
    }
}

impl<'stmt> DqliteSnapshotLoader for SnapshotLoader<'stmt> {
    type DatabaseLoader<'a>
        = DatabaseLoader<'a, 'stmt>
    where
        Self: 'a;

    type Output = ();

    fn database_loader<'a>(&'a mut self, name: &'a OsStr) -> Result<Self::DatabaseLoader<'a>> {
        Ok(DatabaseLoader {
            loader: self,
            database: name,
        })
    }

    fn finish(self) -> Result<Self::Output> {
        Ok(())
    }
}

struct DatabaseLoader<'db, 'stmt> {
    loader: &'db mut SnapshotLoader<'stmt>,
    database: &'db OsStr,
}

impl<'db, 'stmt> DqliteDatabaseLoader for DatabaseLoader<'db, 'stmt> {
    fn load_main(&mut self, read: impl Read) -> Result<()> {
        let mut bufreader = BufReader::new(read);
        let mut page = vec![0u8; self.loader.page_size];
        let mut page_number = 1;
        loop {
            bufreader.fill_buf()?;
            if bufreader.buffer().is_empty() {
                break;
            }
            bufreader.read_exact(&mut page)?;
            self.loader.page_stmt.execute(named_params![
                ":data": &page,
            ])?;
            let page_id = self.loader.conn.last_insert_rowid();
            self.loader.db_stmt.execute(named_params![
                ":database": str::from_utf8(self.database.as_bytes())?,
                ":page_number": page_number,
                ":page_id": page_id,
            ])?;
            page_number += 1;
        }

        Ok(())
    }

    fn load_wal(&mut self, read: impl Read) -> Result<()> {
        let mut bufreader = BufReader::new(read);
        bufreader.fill_buf()?;
        if bufreader.buffer().is_empty() {
            return Ok(()); // Empty WAL
        }

        // Discard the header
        const WAL_HEADER_SIZE: u64 = 32;
        io::copy(
            &mut bufreader.by_ref().take(WAL_HEADER_SIZE),
            &mut io::sink(),
        )?;

        const WAL_FRAME_HEADER_SIZE: u64 = 24;
        let mut frame_header = [0u8; WAL_FRAME_HEADER_SIZE as usize];
        let mut page = vec![0u8; self.loader.page_size];

        loop {
            bufreader.fill_buf()?;
            if bufreader.buffer().is_empty() {
                break;
            }

            bufreader.read_exact(&mut frame_header)?;
            bufreader.read_exact(&mut page)?;

            let page_number = u32::from_be_bytes(frame_header[0..4].try_into().unwrap());
            debug_assert!(page_number > 0);

            self.loader.page_stmt.execute(named_params![
                ":data": &page,
            ])?;
            let page_id = self.loader.conn.last_insert_rowid();
            self.loader.db_stmt.execute(named_params![
                ":database": str::from_utf8(self.database.as_bytes())?,
                ":page_number": page_number,
                ":page_id": page_id,
            ])?;
        }

        // Make sure to remove any unreferenced page, if any
        // FIXME: not nice and forces an additional index.
        // FIXME: remove pages that are above database size.
        self.loader.conn.execute_batch(
            "
                DELETE FROM dqlite_page
                WHERE id NOT IN (
                    SELECT page_id
                    FROM dqlite_database
                )
            ",
        )?;

        Ok(())
    }
}

impl Vfs for DqliteVfs {
    type File = File;

    fn open(
        &self,
        name: Option<VfsPath<'_>>,
        flags: OpenFlags,
    ) -> rusqlite_ext::Result<(Self::File, OpenFlags)> {
        if flags.file_type() != FileType::MainDb {
            // Only main database files are supported in this VFS.
            return Err(SqliteError::from_rc(sqlite3::SQLITE_CANTOPEN).unwrap());
        }
        let name = name.ok_or(SqliteError::from_rc(sqlite3::SQLITE_CANTOPEN).unwrap())?;
        let name = str::from_utf8(name.inner().as_bytes())
            .or(Err(SqliteError::from_rc(sqlite3::SQLITE_CANTOPEN).unwrap()))?;

        // Check that the database exists. In dqlite a database does not exist if the in-header
        // database size is zero or if there are no pages associated to it.
        let raft_index = {
            let conn = self.connection.lock().unwrap();

            let current_index = self.current_index.load(Ordering::SeqCst);
            let raft_index = get_change_index(&conn, name, current_index)
                .or(Err(
                    SqliteError::from_rc(sqlite3::SQLITE_IOERR_READ).unwrap()
                ))?
                .map(|idx| idx as u64)
                .unwrap_or(self.first_index);

            match read_database_size(&conn, raft_index, name) {
                Ok(0) => return Err(SqliteError::from_rc(sqlite3::SQLITE_CANTOPEN).unwrap()),
                Ok(_) => raft_index,
                Err(_) => return Err(SqliteError::from_rc(sqlite3::SQLITE_IOERR_READ).unwrap()),
            }
        };

        let mut out_flags = flags;
        out_flags.set_read_only();

        Ok((
            File {
                connection: Arc::clone(&self.connection),
                raft_index,
                page_size: self.page_size,
                name: name.to_string(),
            },
            out_flags,
        ))
    }

    fn delete(&self, _name: VfsPath<'_>, _sync_dir: bool) -> rusqlite_ext::Result<()> {
        Ok(())
    }

    fn exists(&self, _name: VfsPath<'_>) -> rusqlite_ext::Result<bool> {
        // All databases "exist" and can be opened in dqlite. Indeed, deleting a database
        // means just removing all its contents, as the database file itself is virtual.
        Ok(true)
    }

    fn can_read(&self, _name: VfsPath<'_>) -> rusqlite_ext::Result<bool> {
        Ok(true)
    }

    fn can_write(&self, _name: VfsPath<'_>) -> rusqlite_ext::Result<bool> {
        // This VFS is read-only.
        Ok(false)
    }

    fn write_full_path(
        &self,
        name: VfsPath<'_>,
        mut out: &mut [u8],
    ) -> rusqlite_ext::Result<usize> {
        let inner = name.inner();
        out.write_all(inner.as_bytes())
            .or(Err(SqliteError::from_rc(sqlite3::SQLITE_CANTOPEN).unwrap()))?;
        Ok(inner.len())
    }

    fn last_error(&self) -> i32 {
        0
    }
}

fn get_change_index(
    conn: &Connection,
    database: &str,
    current_index: u64,
) -> rusqlite::Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "
            SELECT MAX(log_idx)
            FROM dqlite_transaction
            WHERE database = :database
                AND log_idx <= :index
        ",
    )?;
    let index = stmt.query_one(
        named_params! {
            ":database": database,
            ":index": current_index as i64,
        },
        |row| row.get(0),
    )?;
    Ok(index)
}

pub struct File {
    connection: Arc<Mutex<Connection>>,
    raft_index: u64,
    page_size: usize,
    name: String,
}

impl VfsFile for File {
    // FIXME: in theory, this might read more than one page at a time, but I think
    // it happens only when:
    // - POWERSAFE is off (our case)
    // - and LCM(page_size, sector_size) != page_size
    // I also think it doesn't happen for read-only VFSes like this one, but need to double-check.
    // This VFS returns sector_size == page_size, so it should be fine.
    fn read_at(&mut self, buf: &mut [u8], offset: u64) -> rusqlite_ext::Result<()> {
        assert!(offset.is_multiple_of(self.page_size as u64));
        assert!(buf.len() == self.page_size || buf.len() == HEADER_SIZE);

        let conn = self
            .connection
            .lock()
            .expect("internal error: poisoned mutex");
        let page_number = (offset / self.page_size as u64) as u32 + 1;
        match read_database_page(&conn, self.raft_index, &self.name, page_number, buf) {
            Ok(true) => Ok(()),
            Ok(false) => Err(SqliteError::from_rc(sqlite3::SQLITE_IOERR_SHORT_READ).unwrap()),
            Err(_) => Err(SqliteError::from_rc(sqlite3::SQLITE_IOERR_READ).unwrap()),
        }
    }

    fn write_at(&mut self, _buf: &[u8], _offset: u64) -> rusqlite_ext::Result<()> {
        Err(SqliteError::from_rc(sqlite3::SQLITE_IOERR_WRITE).unwrap())
    }

    fn truncate(&mut self, _size: u64) -> rusqlite_ext::Result<()> {
        Err(SqliteError::from_rc(sqlite3::SQLITE_IOERR_WRITE).unwrap())
    }

    fn sync(&mut self, _op: SyncOptions) -> rusqlite_ext::Result<()> {
        Ok(())
    }

    fn len(&self) -> rusqlite_ext::Result<u64> {
        let conn = self
            .connection
            .lock()
            .expect("internal error: poisoned mutex");

        match read_database_size(&conn, self.raft_index, &self.name) {
            Ok(size) => Ok(size),
            Err(_) => Err(SqliteError::from_rc(sqlite3::SQLITE_IOERR_FSTAT).unwrap()),
        }
    }

    fn lock(&mut self, _level: LockLevel) -> rusqlite_ext::Result<()> {
        Ok(())
    }

    fn unlock(&mut self, _level: LockLevel) -> rusqlite_ext::Result<()> {
        Ok(())
    }

    fn is_write_locked(&self) -> rusqlite_ext::Result<bool> {
        Ok(false)
    }

    fn sector_len(&self) -> u32 {
        self.page_size as u32
    }

    fn io_capabilities(&self) -> IoCapabilities {
        IoCapabilities {
            immutable: true,
            subpage_read: false,
            ..IoCapabilities::default()
        }
    }

    fn lock_level(&self) -> LockLevel {
        LockLevel::None
    }

    fn last_errno(&self) -> i32 {
        0
    }

    fn pragma(&mut self, name: &str, arg: Option<&str>) -> PragmaResult {
        match (name, arg) {
            ("raft_last_update", None) => Ok(Some(Cow::Owned(self.raft_index.to_string()))),
            _ => Err(PragmaError::from(
                SqliteError::from_rc(sqlite3::SQLITE_NOTFOUND).unwrap(),
            )),
        }
    }
}

fn read_database_size(conn: &Connection, raft_index: u64, database: &str) -> rusqlite::Result<u64> {
    let header = &mut [0u8; HEADER_SIZE];
    if read_database_page(conn, raft_index, database, 1, header)? {
        Ok(decode_database_size(header))
    } else {
        Ok(0)
    }
}

fn read_database_page(
    conn: &Connection,
    raft_index: u64,
    database: &str,
    page_number: u32,
    buf: &mut [u8],
) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare_cached(
        "
            WITH transaction_page AS (
                SELECT max(log_idx) AS log_idx, page_id
                FROM dqlite_transaction
                WHERE log_idx <= :index
                    AND database = :database
                    AND page_number = :page_number
                
                UNION ALL

                SELECT -1, page_id
                FROM dqlite_database
                WHERE database = :database 
                    AND page_number = :page_number
            )
            SELECT data
            FROM dqlite_page
            WHERE id = (
                SELECT page_id
                FROM (
                    SELECT MAX(log_idx) AS log_idx, page_id
                    FROM transaction_page
                )
            )
        ",
    )?;

    let mut rows = stmt.query(named_params! {
        ":index": raft_index as i64,
        ":database": database,
        ":page_number": page_number as i64,
    })?;

    if let Some(row) = rows.next()? {
        let col = row.get_ref(0)?;
        let data = col.as_blob()?;
        assert!(buf.len() <= data.len());
        buf.copy_from_slice(&data[..buf.len()]);
        Ok(true)
    } else {
        buf.fill(0);
        Ok(false)
    }
}

fn decode_database_size(data: &[u8; HEADER_SIZE]) -> u64 {
    let page_size = u16::from_be_bytes(data[16..18].try_into().unwrap()) as u64;
    let page_count = u32::from_be_bytes(data[28..32].try_into().unwrap()) as u64;

    page_size * page_count
}
