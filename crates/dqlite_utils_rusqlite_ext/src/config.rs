use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use libsqlite3_sys as sqlite3;
use rusqlite::Connection;

use crate::{Result, SqliteCode};

/// Extension trait providing additional SQLite database configuration methods.
#[allow(unused)]
pub trait ConnectionConfigExt {
    /// Sets the name of the main database schema.
    ///
    /// See [`SQLITE_DBCONFIG_MAINDBNAME`](https://sqlite.org/c3ref/c_dbconfig_defensive.html#sqlitedbconfigmaindbname).
    fn set_main_name(&self, name: &'static CStr);

    /// Configures the lookaside memory allocator with a pre-allocated buffer.
    ///
    /// Each slot is `SZ` bytes; the buffer contains `buf.len()` slots. Returns an error
    /// if the configuration fails.
    ///
    /// See [`SQLITE_DBCONFIG_LOOKASIDE`](https://sqlite.org/c3ref/c_dbconfig_defensive.html#sqlitedbconfiglookaside).
    fn set_lookaside_buffer<const SZ: usize>(&self, buf: &'static [[u8; SZ]]) -> Result<()>;

    /// Configures the lookaside memory allocator to use internal allocation.
    ///
    /// `sz` is the size of each slot in bytes, `cnt` is the number of slots. Returns an error
    /// if the configuration fails.
    ///
    /// See [`SQLITE_DBCONFIG_LOOKASIDE`](https://sqlite.org/c3ref/c_dbconfig_defensive.html#sqlitedbconfiglookaside).
    fn set_lookaside_size(&self, sz: usize, cnt: usize) -> Result<()>;
}

impl ConnectionConfigExt for Connection {
    fn set_main_name(&self, name: &'static CStr) {
        let rc = unsafe {
            sqlite3::sqlite3_db_config(
                self.handle(),
                sqlite3::SQLITE_DBCONFIG_MAINDBNAME,
                name.as_ptr() as *const c_char,
            )
        };
        assert_eq!(rc, sqlite3::SQLITE_OK);
    }

    fn set_lookaside_buffer<const SIZE: usize>(&self, buf: &'static [[u8; SIZE]]) -> Result<()> {
        let rc = unsafe {
            sqlite3::sqlite3_db_config(
                self.handle(),
                sqlite3::SQLITE_DBCONFIG_LOOKASIDE,
                buf.as_ptr() as *const c_void,
                SIZE as c_int,
                buf.len() as c_int,
            )
        };
        SqliteCode::from_rc(rc).into()
    }

    fn set_lookaside_size(&self, size: usize, count: usize) -> Result<()> {
        let rc = unsafe {
            sqlite3::sqlite3_db_config(
                self.handle(),
                sqlite3::SQLITE_DBCONFIG_LOOKASIDE,
                ptr::null::<c_void>(),
                size as c_int,
                count as c_int,
            )
        };
        SqliteCode::from_rc(rc).into()
    }
}
