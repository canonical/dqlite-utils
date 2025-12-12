use std::ffi::{CString, OsStr, c_char, c_void};
use std::mem;
use std::os::unix::ffi::OsStrExt;
use std::ptr::{self, NonNull};

use libsqlite3_sys::{self as sqlite3, sqlite3_file, sqlite3_io_methods};
use rusqlite::Connection;

use crate::rusqlite_ext::{Result, SqliteCode, SqliteError};

#[allow(unused)]
pub trait Files {
    type FileType<'a>
    where
        Self: 'a;
    fn main_file<'a>(&'a self, db: Option<&OsStr>) -> Self::FileType<'a>;
    fn journal_file<'a>(&'a self, db: Option<&OsStr>) -> Option<Self::FileType<'a>>;
}

enum SmallCString<const MAX_SIZE: usize = 128> {
    CString(CString),
    Stack([u8; MAX_SIZE]),
}

impl<const MAX_SIZE: usize> SmallCString<MAX_SIZE> {
    const MAX_SIZE: usize = MAX_SIZE;
}

impl SmallCString {
    pub fn as_ptr(&self) -> *const c_char {
        match self {
            SmallCString::CString(cstr) => cstr.as_ptr(),
            SmallCString::Stack(stack) => stack.as_ptr() as *const c_char,
        }
    }
}

impl From<&str> for SmallCString {
    fn from(s: &str) -> Self {
        Self::from(s.as_bytes())
    }
}

impl From<&OsStr> for SmallCString {
    fn from(s: &OsStr) -> Self {
        Self::from(s.as_bytes())
    }
}

impl From<&[u8]> for SmallCString {
    fn from(s: &[u8]) -> Self {
        if s.len() < Self::MAX_SIZE {
            let mut stack = [0u8; Self::MAX_SIZE];
            stack[..s.len()].copy_from_slice(s);
            assert!(stack[s.len()] == 0);
            SmallCString::Stack(stack)
        } else {
            SmallCString::CString(CString::new(s).unwrap())
        }
    }
}

#[allow(unused)]
impl Files for Connection {
    type FileType<'a>
        = File
    where
        Self: 'a;

    fn main_file<'a>(&'a self, db: Option<&OsStr>) -> Self::FileType<'a> {
        let handle = unsafe { get_file_handle(self, db, false).as_mut() }.unwrap();
        File::new(handle)
    }

    fn journal_file<'a>(&'a self, db: Option<&OsStr>) -> Option<Self::FileType<'a>> {
        let handle = unsafe { get_file_handle(self, db, true).as_mut() }?;
        Some(File::new(handle))
    }
}

unsafe fn get_file_handle(
    conn: &Connection,
    db: Option<&OsStr>,
    journal: bool,
) -> *mut sqlite3_file {
    let handle = unsafe { conn.handle() };
    let mut file_raw: *mut sqlite3_file = ptr::null_mut();
    let op = if journal {
        sqlite3::SQLITE_FCNTL_JOURNAL_POINTER
    } else {
        sqlite3::SQLITE_FCNTL_FILE_POINTER
    };
    let db = db.map(SmallCString::from);

    let rc = unsafe {
        sqlite3::sqlite3_file_control(
            handle,
            db.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
            op,
            mem::transmute(&mut file_raw),
        )
    };
    debug_assert!(rc == sqlite3::SQLITE_OK);

    file_raw
}

#[allow(unused)]
pub struct File {
    handle: NonNull<sqlite3_file>,
}

#[allow(unused)]
impl File {
    fn new(handle: *mut sqlite3_file) -> Self {
        File {
            handle: NonNull::new(handle).unwrap(),
        }
    }

    fn handle(&self) -> &mut sqlite3_file {
        unsafe { &mut *self.handle.as_ptr() }
    }

    fn methods(&self) -> &sqlite3_io_methods {
        unsafe { self.handle().pMethods.as_ref().unwrap() }
    }

    pub fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<()> {
        let read = self.methods().xRead.unwrap();
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

    pub fn write_at(&mut self, buf: &[u8], offset: u64) -> Result<()> {
        let write = self.methods().xRead.unwrap();
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
        let file_size = self.methods().xFileSize.unwrap();
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
    use super::{File, Files};
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

    fn open_file(conn: &Connection, kind: FileKind) -> (File, bool) {
        match kind {
            FileKind::Main => (conn.main_file(None), false),
            FileKind::Journal => conn
                .journal_file(None)
                .map(|file| (file, false))
                .unwrap_or_else(|| {
                    conn.execute("BEGIN IMMEDIATE", ()).unwrap();
                    conn.execute("INSERT INTO data(value) VALUES ('gamma')", ())
                        .unwrap();
                    (
                        conn.journal_file(None)
                            .expect("journal handle available after write"),
                        true,
                    )
                }),
        }
    }

    fn run_for_all_setups<F>(test: F)
    where
        F: Fn(&mut File, JournalMode, FileKind),
    {
        let setups = [
            (JournalMode::Delete, FileKind::Main),
            (JournalMode::Wal, FileKind::Main),
            (JournalMode::Wal, FileKind::Journal),
        ];

        for (mode, kind) in setups {
            run_setup(mode, kind, |file| {
                test(file, mode, kind);
            });
        }
    }

    fn run_setup(mode: JournalMode, kind: FileKind, test: impl Fn(&mut File)) {
        with_prepared_connection(mode, |conn| {
            let (mut file, needs_cleanup) = open_file(conn, kind);
            test(&mut file);
            drop(file);

            if needs_cleanup {
                conn.execute("ROLLBACK", ()).unwrap();
            }
        });
    }

    #[test]
    fn test_read_at() {
        run_for_all_setups(|file, _mode, kind| {
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
        run_for_all_setups(|file, _mode, _kind| {
            let mut header = [0u8; 16];
            file.read_at(&mut header, 0).unwrap();
            file.write_at(&header, 0).unwrap();
        });
    }

    #[test]
    fn test_len() {
        run_for_all_setups(|file, _mode, _kind| {
            let len = file.len().unwrap();
            assert!(len > 0);
        });
    }
}
