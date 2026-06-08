use std::ffi::{OsStr, c_void};
use std::marker::PhantomData;
use std::mem;
use std::ptr::{self, NonNull};

use libsqlite3_sys::{self as sqlite3, sqlite3_file, sqlite3_io_methods};
use rusqlite::Connection;

use crate::{Result, SmallCString, SqliteCode, SqliteError};

pub trait ConnectionFilesExt {
    fn main_file(&self, db: Option<&OsStr>) -> Result<ConnectionFile<'_>>;
    fn journal_file(&self, db: Option<&OsStr>) -> Result<Option<ConnectionFile<'_>>>;
}

impl ConnectionFilesExt for Connection {
    fn main_file(&self, db: Option<&OsStr>) -> Result<ConnectionFile<'_>> {
        let handle = unsafe { get_file_handle(self, db, false)? };
        Ok(ConnectionFile::new(handle))
    }

    fn journal_file(&self, db: Option<&OsStr>) -> Result<Option<ConnectionFile<'_>>> {
        let handle = unsafe { get_file_handle(self, db, true)?.as_mut() };
        let handle = match handle {
            Some(handle) => handle,
            None => return Ok(None),
        };
        if handle.pMethods.is_null() {
            return Ok(None);
        }
        Ok(Some(ConnectionFile::new(handle)))
    }
}

unsafe fn get_file_handle(
    conn: &Connection,
    db: Option<&OsStr>,
    journal: bool,
) -> Result<*mut sqlite3_file> {
    let handle = unsafe { conn.handle() };
    let db = db.map(SmallCString::from);
    let op = if journal {
        sqlite3::SQLITE_FCNTL_JOURNAL_POINTER
    } else {
        sqlite3::SQLITE_FCNTL_FILE_POINTER
    };
    let mut ret: *mut sqlite3_file = ptr::null_mut();
    let rc = unsafe {
        sqlite3::sqlite3_file_control(
            handle,
            db.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
            op,
            mem::transmute::<&mut *mut sqlite3_file, *mut std::ffi::c_void>(&mut ret),
        )
    };
    if let Some(err) = SqliteError::from_rc(rc) {
        return Err(err);
    }
    Ok(ret)
}

#[allow(unused)]
pub struct ConnectionFile<'conn> {
    handle: NonNull<sqlite3_file>,
    _conn: PhantomData<&'conn Connection>,
}

impl ConnectionFile<'_> {
    fn new(handle: *mut sqlite3_file) -> Self {
        ConnectionFile {
            handle: NonNull::new(handle)
                .expect("internal error: cannot create file with null handle"),
            _conn: PhantomData,
        }
    }

    fn methods(&self) -> &sqlite3_io_methods {
        unsafe { self.handle.as_ref().pMethods.as_ref().unwrap() }
    }

    pub fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<()> {
        let read = self
            .methods()
            .xRead
            .ok_or(SqliteError::from_rc(sqlite3::SQLITE_MISUSE).unwrap())?;
        let rc = unsafe {
            read(
                self.handle.as_ptr(),
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as i32,
                offset as i64,
            )
        };
        SqliteCode::from_rc(rc).into()
    }

    #[allow(unused)]
    pub fn write_at(&mut self, buf: &[u8], offset: u64) -> Result<()> {
        let write = self
            .methods()
            .xWrite
            .ok_or(SqliteError::from_rc(sqlite3::SQLITE_MISUSE).unwrap())?;
        let rc = unsafe {
            write(
                self.handle.as_ptr(),
                buf.as_ptr() as *mut c_void,
                buf.len() as i32,
                offset as i64,
            )
        };
        SqliteCode::from_rc(rc).into()
    }

    pub fn len(&self) -> Result<u64> {
        let file_size = self
            .methods()
            .xFileSize
            .ok_or(SqliteError::from_rc(sqlite3::SQLITE_MISUSE).unwrap())?;
        let mut size: i64 = 0;
        let rc = unsafe { file_size(self.handle.as_ptr(), &mut size as *mut i64) };
        if let Some(err) = SqliteError::from_rc(rc) {
            return Err(err);
        }
        Ok(size as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    #[derive(Clone, Copy)]
    enum JournalMode {
        Delete,
        Wal,
    }

    impl JournalMode {
        fn pragma_value(self) -> &'static str {
            match self {
                JournalMode::Delete => "DELETE",
                JournalMode::Wal => "WAL",
            }
        }
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum FileKind {
        Main,
        Journal,
    }

    fn with_prepared_connection<F>(mode: JournalMode, test: F)
    where
        F: FnOnce(&Connection),
    {
        let tmp = NamedTempFile::new().unwrap();
        let tmp_path = tmp.into_temp_path();
        let path_buf = tmp_path.to_path_buf();
        let conn = Connection::open(&path_buf).unwrap();

        conn.pragma_update(None, "journal_mode", mode.pragma_value())
            .unwrap();
        conn.execute("CREATE TABLE data(id INTEGER PRIMARY KEY, value TEXT)", ())
            .unwrap();
        conn.execute("INSERT INTO data(value) VALUES ('alpha'), ('beta')", ())
            .unwrap();

        test(&conn);
    }

    fn open_file(conn: &Connection, kind: FileKind) -> ConnectionFile<'_> {
        match kind {
            FileKind::Main => conn.main_file(None).unwrap(),
            FileKind::Journal => conn
                .journal_file(None)
                .unwrap()
                .expect("internal error: no journal file"),
        }
    }

    fn run_all_setups<F>(test: F)
    where
        F: Fn(&mut ConnectionFile<'_>, JournalMode, FileKind),
    {
        let run_setups = [
            (JournalMode::Delete, FileKind::Main),
            (JournalMode::Wal, FileKind::Main),
            (JournalMode::Wal, FileKind::Journal),
        ];
        for (mode, kind) in run_setups {
            with_prepared_connection(mode, |conn| {
                let mut file = open_file(conn, kind);
                test(&mut file, mode, kind);
                drop(file);
            });
        }
    }

    #[test]
    fn test_read_at() {
        run_all_setups(|file, _mode, kind| {
            let mut header = [0u8; 16];
            file.read_at(&mut header, 0).unwrap();
            if kind == FileKind::Journal {
                // Journal files may start with a different header
                return;
            }
            assert!(header.starts_with(b"SQLite format 3"));
        });
    }

    #[test]
    fn test_write_at() {
        run_all_setups(|file, _mode, _kind| {
            let mut header = [0u8; 16];
            file.read_at(&mut header, 0).unwrap();
            file.write_at(&header, 0).unwrap();
        });
    }

    #[test]
    fn test_len() {
        run_all_setups(|file, _mode, _kind| {
            let len = file.len().unwrap();
            assert!(len > 0);
        });
    }
}
