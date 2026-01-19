use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use libsqlite3_sys as sqlite3;
use rusqlite::Connection;

use crate::rusqlite_ext::{Result, SqliteCode};

pub trait ConnectionConfigExt {
    fn set_main_name(&self, name: &'static CStr);
    fn set_lookaside_buffer<const SZ: usize>(&self, buf: &'static [[u8; SZ]]) -> Result<()>;
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

    fn set_lookaside_buffer<const SZ: usize>(&self, buf: &'static [[u8; SZ]]) -> Result<()> {
        let rc = unsafe {
            sqlite3::sqlite3_db_config(
                self.handle(),
                sqlite3::SQLITE_DBCONFIG_LOOKASIDE,
                buf.as_ptr() as *const c_void,
                SZ as c_int,
                buf.len() as c_int,
            )
        };

        SqliteCode::from_rc(rc).into()
    }

    fn set_lookaside_size(&self, sz: usize, cnt: usize) -> Result<()> {
        let rc = unsafe {
            sqlite3::sqlite3_db_config(
                self.handle(),
                sqlite3::SQLITE_DBCONFIG_LOOKASIDE,
                ptr::null::<c_void>(),
                sz as c_int,
                cnt as c_int,
            )
        };

        SqliteCode::from_rc(rc).into()
    }
}
