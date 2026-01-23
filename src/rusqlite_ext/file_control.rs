use libsqlite3_sys::{self as sqlite3, sqlite3_int64};
use std::{
    ffi::{CStr, OsStr, c_int, c_void},
    ptr,
    time::Duration,
};

use rusqlite::Connection;

use crate::rusqlite_ext::{Result, SmallCString, SqliteError, vfs::LockLevel};

pub trait FileControlExt {
    /// Gets the current lock state.
    ///
    /// See [`SQLITE_FCNTL_LOCKSTATE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllockstate).
    fn lock_level(&self, db: Option<&OsStr>) -> Result<LockLevel>;

    /// Gets the last OS error number.
    ///
    /// See [`SQLITE_FCNTL_LAST_ERRNO`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllasterrno).
    fn last_errno(&self, db: Option<&OsStr>) -> Result<i32>;

    /// Sets the database chunk size.
    ///
    /// See [`SQLITE_FCNTL_CHUNK_SIZE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlchunksize).
    fn set_chunk_size(&mut self, db: Option<&OsStr>, size: u32) -> Result<()>;

    /// Sets the max mmap size.
    ///
    /// See [`SQLITE_FCNTL_MMAP_SIZE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlmmapsize).
    fn set_mmap_size(&mut self, db: Option<&OsStr>, size: u64) -> Result<()>;

    /// Gets the max mmap size.
    ///
    /// See [`SQLITE_FCNTL_MMAP_SIZE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlmmapsize).
    fn mmap_size(&self, db: Option<&OsStr>) -> Result<u64>;

    /// Reports whether the file has moved.
    ///
    /// See [`SQLITE_FCNTL_HAS_MOVED`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcnthasmoved).
    fn has_moved(&self, db: Option<&OsStr>) -> Result<bool>;

    /// Sets the lock timeout.
    ///
    /// See [`SQLITE_FCNTL_LOCK_TIMEOUT`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllocktimeout).
    fn set_lock_timeout(&mut self, db: Option<&OsStr>, timeout: Duration) -> Result<Duration>;

    /// Gets WAL persistence.
    ///
    /// See [`SQLITE_FCNTL_PERSIST_WAL`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpersistwal).
    fn is_wal_persistent(&self, db: Option<&OsStr>) -> Result<bool>;

    /// Sets WAL persistence.
    ///
    /// See [`SQLITE_FCNTL_PERSIST_WAL`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpersistwal).
    fn set_wal_persistent(&mut self, db: Option<&OsStr>, persist: bool) -> Result<()>;

    /// Gets whether powersafe overwrite is enabled.
    ///
    /// See [`SQLITE_FCNTL_POWERSAFE_OVERWRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpowersafeoverwrite).
    fn powersafe_overwrite(&self, db: Option<&OsStr>) -> Result<bool>;

    /// Sets whether powersafe overwrite is enabled.
    ///
    /// See [`SQLITE_FCNTL_POWERSAFE_OVERWRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpowersafeoverwrite).
    fn set_powersafe_overwrite(&mut self, db: Option<&OsStr>, powersafe: bool) -> Result<()>;

    /// Gets the name of the VFS this file belongs to.
    ///
    /// See [`SQLITE_FCNTL_VFSNAME`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlvfsname).
    fn vfs_name(&self, db: Option<&OsStr>) -> Result<SmallCString>;

    // TODO: SQLITE_FCNTL_TEMPFILENAME
}

impl FileControlExt for Connection {
    fn lock_level(&self, db: Option<&OsStr>) -> Result<LockLevel> {
        let mut ret: c_int = 0;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_LOCKSTATE,
            &mut ret as *mut c_int as *mut c_void,
        )?;
        Ok(LockLevel::from_raw(ret))
    }

    fn last_errno(&self, db: Option<&OsStr>) -> Result<i32> {
        let mut ret: c_int = 0;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_LAST_ERRNO,
            &mut ret as *mut c_int as *mut c_void,
        )?;
        Ok(ret as i32)
    }

    fn set_chunk_size(&mut self, db: Option<&OsStr>, size: u32) -> Result<()> {
        if size as i64 > c_int::MAX as i64 {
            return Err(SqliteError::from_rc(sqlite3::SQLITE_MISUSE).unwrap());
        }
        let arg = size as c_int;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_CHUNK_SIZE,
            &arg as *const c_int as *mut c_void,
        )?;
        Ok(())
    }

    fn set_mmap_size(&mut self, db: Option<&OsStr>, size: u64) -> Result<()> {
        if size > i64::MAX as u64 {
            return Err(SqliteError::from_rc(sqlite3::SQLITE_MISUSE).unwrap());
        }
        let mut size = size as sqlite3_int64;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_MMAP_SIZE,
            &mut size as *mut sqlite3_int64 as *mut c_void,
        )?;
        Ok(())
    }

    fn mmap_size(&self, db: Option<&OsStr>) -> Result<u64> {
        let mut ret: sqlite3_int64 = -1;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_MMAP_SIZE,
            &mut ret as *mut sqlite3_int64 as *mut c_void,
        )?;
        Ok(ret as u64)
    }

    fn has_moved(&self, db: Option<&OsStr>) -> Result<bool> {
        let mut ret: c_int = 0;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_HAS_MOVED,
            &mut ret as *mut c_int as *mut c_void,
        )?;
        Ok(ret != 0)
    }

    fn set_lock_timeout(&mut self, db: Option<&OsStr>, timeout: Duration) -> Result<Duration> {
        let mut ret: i32 = timeout
            .as_millis()
            .try_into()
            .map_err(|_| SqliteError::from_rc(sqlite3::SQLITE_MISUSE).unwrap())?;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_LOCK_TIMEOUT,
            &mut ret as *mut i32 as *mut c_void,
        )?;
        Ok(Duration::from_millis(ret as u64))
    }

    fn is_wal_persistent(&self, db: Option<&OsStr>) -> Result<bool> {
        let mut ret: c_int = -1;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_PERSIST_WAL,
            &mut ret as *mut c_int as *mut c_void,
        )?;
        Ok(ret != 0)
    }

    fn set_wal_persistent(&mut self, db: Option<&OsStr>, persist: bool) -> Result<()> {
        let persist: c_int = if persist { 1 } else { 0 };
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_PERSIST_WAL,
            &persist as *const c_int as *mut c_void,
        )?;
        Ok(())
    }

    fn powersafe_overwrite(&self, db: Option<&OsStr>) -> Result<bool> {
        let mut ret: c_int = -1;
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_POWERSAFE_OVERWRITE,
            &mut ret as *mut c_int as *mut c_void,
        )?;
        Ok(ret != 0)
    }

    fn set_powersafe_overwrite(&mut self, db: Option<&OsStr>, powersafe: bool) -> Result<()> {
        let powersafe: c_int = if powersafe { 1 } else { 0 };
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_POWERSAFE_OVERWRITE,
            &powersafe as *const c_int as *mut c_void,
        )?;
        Ok(())
    }

    fn vfs_name(&self, db: Option<&OsStr>) -> Result<SmallCString> {
        let mut name_ptr: *const i8 = ptr::null();
        file_control(
            self,
            db,
            sqlite3::SQLITE_FCNTL_VFSNAME,
            &mut name_ptr as *mut *const i8 as *mut c_void,
        )?;
        if name_ptr.is_null() {
            return Err(SqliteError::from_rc(sqlite3::SQLITE_ERROR).unwrap());
        }
        let cstr = unsafe { CStr::from_ptr(name_ptr) };
        Ok(SmallCString::from(cstr.to_bytes()))
    }
}

fn file_control(conn: &Connection, db: Option<&OsStr>, op: c_int, arg: *mut c_void) -> Result<()> {
    let db = db.map(SmallCString::from);
    let rc = unsafe {
        sqlite3::sqlite3_file_control(
            conn.handle(),
            db.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
            op,
            arg,
        )
    };

    if let Some(err) = SqliteError::from_rc(rc) {
        return Err(err);
    }

    Ok(())
}
