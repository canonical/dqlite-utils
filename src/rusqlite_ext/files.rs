use std::borrow::Cow;
use std::ffi::{CString, OsStr, c_char, c_int, c_void};
use std::num::NonZero;
use std::os::unix::ffi::OsStrExt;
use std::ptr;

use libsqlite3_sys::{self as sqlite3, sqlite3_file, sqlite3_io_methods};
use rusqlite::Connection;

use crate::rusqlite_ext::vfs::IoCapabilities;

use super::{
    Result, SqliteCode, SqliteError,
    vfs::{LockLevel, PragmaError, PragmaResult, SyncOptions, VfsFile, VfsPath},
};

#[allow(unused)]
pub trait Files {
    type FileType<'a>
    where
        Self: 'a;
    fn main_file<'a>(&'a self, db: Option<&OsStr>) -> Self::FileType<'a>;
    fn journal_file<'a>(&'a self, db: Option<&OsStr>) -> Option<Self::FileType<'a>>;
}

enum CStringOpt<const MAX_SIZE: usize = 128> {
    CString(CString),
    Stack([u8; MAX_SIZE]),
}

impl<const MAX_SIZE: usize> CStringOpt<MAX_SIZE> {
    const MAX_SIZE: usize = MAX_SIZE;
}

impl CStringOpt {
    pub fn new(s: &str) -> Self {
        if s.len() < Self::MAX_SIZE {
            let mut stack = [0u8; Self::MAX_SIZE];
            stack[..s.len()].copy_from_slice(s.as_bytes());
            CStringOpt::Stack(stack)
        } else {
            CStringOpt::CString(CString::new(s.as_bytes()).unwrap())
        }
    }

    pub fn from_os_str(s: &OsStr) -> Self {
        let bytes = s.as_bytes();
        if bytes.len() < Self::MAX_SIZE {
            let mut stack = [0u8; Self::MAX_SIZE];
            stack[..bytes.len()].copy_from_slice(bytes);
            CStringOpt::Stack(stack)
        } else {
            CStringOpt::CString(CString::new(bytes).unwrap())
        }
    }

    pub fn as_ptr(&self) -> *const c_char {
        match self {
            CStringOpt::CString(cstr) => cstr.as_ptr(),
            CStringOpt::Stack(stack) => stack.as_ptr() as *const c_char,
        }
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
    let db = db.map(|d| CStringOpt::from_os_str(d));

    let rc = unsafe {
        sqlite3::sqlite3_file_control(
            handle,
            db.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
            op,
            &mut file_raw as *mut *mut _ as *mut c_void,
        )
    };
    debug_assert!(rc == sqlite3::SQLITE_OK);

    file_raw
}

#[allow(unused)]
impl Files for Connection {
    type FileType<'a>
        = File<'a>
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

#[allow(unused)]
pub struct File<'a> {
    handle: *mut sqlite3_file,
    busy_handler: Option<Box<dyn Fn() -> bool + 'a>>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> File<'a> {
    fn new(handle: *mut sqlite3_file) -> Self {
        File {
            handle,
            busy_handler: None,
            _marker: std::marker::PhantomData,
        }
    }

    fn handle(&self) -> &'a mut sqlite3_file {
        unsafe { &mut *self.handle }
    }

    fn methods(&self) -> &'a sqlite3_io_methods {
        unsafe { self.handle().pMethods.as_ref().unwrap() }
    }

    unsafe fn call<T>(
        &self,
        select: impl FnOnce(&sqlite3_io_methods) -> Option<T>,
        call: impl FnOnce(T, *mut sqlite3_file) -> c_int,
    ) -> Result<()> {
        if let Some(method) = select(self.methods()) {
            let rc = call(method, self.handle as *mut sqlite3_file);
            return SqliteCode::from_rc(rc).into();
        } else {
            // This should only happen if the sqlite3_file is a memory file.
            SqliteCode::from_rc(rusqlite::ffi::SQLITE_IOERR).into()
        }
    }

    unsafe fn call_output<T, O: Default + Sized>(
        &self,
        select: impl FnOnce(&sqlite3_io_methods) -> Option<T>,
        call: impl FnOnce(T, *mut sqlite3_file, *mut O) -> c_int,
    ) -> Result<O> {
        if let Some(method) = select(self.methods()) {
            let mut out: O = Default::default();
            let rc = call(method, self.handle as *mut sqlite3_file, &mut out as *mut O);

            if rc == sqlite3::SQLITE_OK {
                return Ok(out);
            } else {
                return Err(SqliteError(NonZero::new(rc).unwrap()));
            }
        } else {
            // This should only happen if the sqlite3_file is a memory file.
            Err(SqliteError(
                NonZero::new(rusqlite::ffi::SQLITE_IOERR).unwrap(),
            ))
        }
    }

    unsafe fn control_read<T: Default + Sized>(&self, op: c_int) -> Result<T> {
        unsafe {
            self.call_output(
                |methods| methods.xFileControl,
                |file_control, file, out: *mut T| file_control(file, op, out as *mut c_void),
            )
        }
    }

    unsafe fn control_write<T>(&self, op: c_int, mut arg: T) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xFileControl,
                |file_control, file| file_control(file, op, &mut arg as *mut _ as *mut c_void),
            )
        }
    }

    unsafe fn control<T>(&self, op: c_int, mut arg: T) -> Result<T> {
        unsafe {
            self.call(
                |methods| methods.xFileControl,
                |file_control, file| file_control(file, op, &mut arg as *mut _ as *mut c_void),
            )
        }
        .map(move |_| arg)
    }
}

impl<'a> VfsFile for File<'a> {
    fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xRead,
                |read, file| {
                    read(
                        file,
                        buf.as_mut_ptr() as *mut _,
                        buf.len() as i32,
                        offset as i64,
                    )
                },
            )
        }
    }

    fn write_at(&mut self, buf: &[u8], offset: u64) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xWrite,
                |write, file| {
                    write(
                        file,
                        buf.as_ptr() as *const _,
                        buf.len() as i32,
                        offset as i64,
                    )
                },
            )
        }
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xTruncate,
                |truncate, file| truncate(file, size as i64),
            )
        }
    }

    fn sync(&mut self, op: SyncOptions) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xSync,
                |sync, file| sync(file, op.to_raw()),
            )
        }
    }

    fn len(&self) -> Result<u64> {
        unsafe {
            self.call_output(
                |methods| methods.xFileSize,
                |file_size, file, out| file_size(file, out as *mut i64),
            )
        }
    }

    fn lock(&mut self, level: LockLevel) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xLock,
                |lock, file| lock(file, level.to_raw()),
            )
        }
    }

    fn unlock(&mut self, level: LockLevel) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xUnlock,
                |unlock, file| unlock(file, level.to_raw()),
            )
        }
    }

    fn is_write_locked(&self) -> Result<bool> {
        unsafe {
            let res: i32 = self.call_output(
                |methods| methods.xCheckReservedLock,
                |check_reserved_lock, file, out| check_reserved_lock(file, out as *mut c_int),
            )?;

            Ok(res != 0)
        }
    }

    fn sector_len(&self) -> u32 {
        let sector_size = self.methods().xSectorSize.unwrap();
        (unsafe { sector_size(self.handle as *mut sqlite3_file) }) as u32
    }

    fn io_capabilities(&self) -> IoCapabilities {
        let device_characteristics = self.methods().xDeviceCharacteristics.unwrap();
        let result = unsafe { device_characteristics(self.handle as *mut sqlite3_file) };
        IoCapabilities::from_raw(result)
    }

    fn lock_level(&self) -> LockLevel {
        let out = unsafe { self.control_read(sqlite3::SQLITE_FCNTL_LOCKSTATE) };
        LockLevel::from_raw(out.unwrap())
    }

    fn last_errno(&self) -> i32 {
        unsafe { self.control_read(sqlite3::SQLITE_FCNTL_LAST_ERRNO) }.unwrap()
    }

    fn hint_size(&mut self, size: i64) -> Result<()> {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_SIZE_HINT, size) }
    }

    fn hint_overwrite(&mut self, size: u64) -> Result<()> {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_OVERWRITE, size) }
    }

    fn set_chunk_size(&mut self, size: u32) -> Result<()> {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_CHUNK_SIZE, size) }
    }

    fn pragma(&mut self, name: &str, arg: Option<&str>) -> PragmaResult {
        let name = CStringOpt::new(name);
        let arg = arg.map(|a| CStringOpt::new(a));
        let mut arg = [
            ptr::null(), // Output parameter
            name.as_ptr(),
            arg.as_ref().map_or(ptr::null(), |a| a.as_ptr()),
        ];
        let file_control = self.methods().xFileControl.unwrap();
        let rc = unsafe {
            file_control(
                self.handle as *mut sqlite3_file,
                sqlite3::SQLITE_FCNTL_PRAGMA,
                &mut arg as *mut _ as *mut c_void,
            )
        };
        let message = if arg[0].is_null() {
            None
        } else {
            Some(Cow::Owned(
                unsafe { CString::from_raw(arg[0] as *mut c_char) }
                    .into_string()
                    .unwrap(),
            ))
        };

        if rc != sqlite3::SQLITE_OK {
            Err(PragmaError::new(SqliteError::from_rc(rc).unwrap(), message))
        } else {
            Ok(message)
        }
    }

    fn set_mmap_size(&mut self, size: u64) -> Result<()> {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_MMAP_SIZE, size as i64) }
    }

    fn mmap_size(&self) -> u64 {
        unsafe { self.control(sqlite3::SQLITE_FCNTL_MMAP_SIZE, -1i64) }.unwrap() as u64
    }

    fn has_moved(&self) -> bool {
        unsafe { self.control_read::<c_int>(sqlite3::SQLITE_FCNTL_HAS_MOVED) }.unwrap() != 0
    }

    fn pre_sync_single_db(&mut self) -> Result<()> {
        unsafe {
            self.call(
                |methods| methods.xFileControl,
                |file_control, file| {
                    file_control(file, sqlite3::SQLITE_FCNTL_SYNC, ptr::null_mut())
                },
            )
        }
    }

    fn pre_sync_multiple_db(&mut self, super_journal: VfsPath<'_>) -> Result<()> {
        let name = CStringOpt::from_os_str(super_journal.inner());

        unsafe {
            self.call(
                |methods| methods.xFileControl,
                |file_control, file| {
                    file_control(
                        file,
                        sqlite3::SQLITE_FCNTL_SYNC,
                        name.as_ptr() as *mut c_void,
                    )
                },
            )
        }
    }

    fn commit_phase_two(&mut self) -> Result<()> {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_COMMIT_PHASETWO, ()) }
    }

    fn begin_atomic(&mut self) -> Result<()> {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_BEGIN_ATOMIC_WRITE, ()) }
    }

    fn commit_atomic(&mut self) -> Result<()> {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_COMMIT_ATOMIC_WRITE, ()) }
    }

    fn rollback_atomic(&mut self) {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_ROLLBACK_ATOMIC_WRITE, ()) }.ok();
    }

    fn lock_timeout(&self) -> std::time::Duration {
        unsafe { self.control(sqlite3::SQLITE_FCNTL_LOCK_TIMEOUT, -1 as c_int) }
            .map(|timeout| std::time::Duration::from_millis(timeout as u64))
            .unwrap()
    }

    fn set_lock_timeout(&mut self, timeout: std::time::Duration) -> Result<()> {
        unsafe {
            self.control_write(
                sqlite3::SQLITE_FCNTL_LOCK_TIMEOUT,
                timeout.as_millis() as c_int,
            )
        }
    }

    fn set_busy_handler(&mut self, handler: impl Fn() -> bool + 'static) {
        self.busy_handler = Some(Box::new(handler));

        unsafe extern "C" fn busy_handler(ptr: *mut c_void) -> c_int {
            let handler = unsafe { &*(ptr as *const Box<dyn Fn() -> bool>) };
            if handler() { 1 } else { 0 }
        }

        unsafe {
            self.control_write(
                sqlite3::SQLITE_FCNTL_BUSYHANDLER,
                [
                    busy_handler as *mut c_void,
                    self.busy_handler.as_ref().unwrap() as *const _ as *mut c_void,
                ],
            )
        }
        .ok();
    }

    fn is_wal_persistent(&self) -> bool {
        unsafe {
            self.control::<c_int>(sqlite3::SQLITE_FCNTL_PERSIST_WAL, -1)
                .unwrap()
                != 0
        }
    }

    fn set_wal_persistent(&mut self, persist: bool) {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_PERSIST_WAL, persist as c_int) }.ok();
    }

    fn hint_wal_lock(&mut self) {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_WAL_BLOCK, ()) }.ok();
    }

    fn hint_block_on_connect(&mut self, block: bool) {
        unsafe { self.control_write(sqlite3::SQLITE_FCNTL_BLOCK_ON_CONNECT, block as c_int) }.ok();
    }

    fn on_checkpoint_start(&mut self) {
        unsafe { self.control(sqlite3::SQLITE_FCNTL_CKPT_START, ()) }.ok();
    }

    fn on_checkpoint_done(&mut self) {
        unsafe { self.control(sqlite3::SQLITE_FCNTL_CKPT_DONE, ()) }.ok();
    }
}

impl Drop for File<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.control_write(sqlite3::SQLITE_FCNTL_BUSYHANDLER, ptr::null::<c_void>());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{File, Files};
    use crate::rusqlite_ext::vfs::{SyncOptions, VfsFile, VfsPath};
    use libsqlite3_sys as sqlite3;
    use rusqlite::Connection;
    use std::ffi::OsStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
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

        drop(conn);
        tmp_path.close().unwrap();
    }

    fn open_file<'a>(conn: &'a Connection, kind: FileKind) -> (File<'a>, bool) {
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
        F: Fn(&mut File<'_>, JournalMode, FileKind),
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

    fn run_setup(mode: JournalMode, kind: FileKind, test: impl Fn(&mut File<'_>)) {
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

    #[test]
    fn test_truncate() {
        run_for_all_setups(|file, _mode, _kind| {
            let len_before = file.len().unwrap();
            file.truncate(len_before).unwrap();
        });
    }

    #[test]
    fn test_sync() {
        run_for_all_setups(|file, _mode, _kind| {
            file.sync(SyncOptions {
                full: false,
                data_only: false,
            })
            .unwrap();
        });
    }

    #[test]
    fn test_lock_and_unlock() {
        run_for_all_setups(|file, _mode, _kind| {
            let current_level = file.lock_level();
            file.lock(current_level).unwrap();
            file.unlock(current_level).unwrap();
        });
    }

    #[test]
    fn test_is_write_locked() {
        run_for_all_setups(|file, _mode, _kind| {
            let locked = file.is_write_locked().unwrap();
            assert!(!locked);
        });
    }

    #[test]
    fn test_sector_len() {
        run_for_all_setups(|file, _mode, _kind| {
            assert!(file.sector_len() > 0);
        });
    }

    #[test]
    fn test_io_capabilities() {
        run_for_all_setups(|file, _mode, _kind| {
            let caps = file.io_capabilities();
            assert!(caps.to_raw() > 0);
        });
    }

    #[test]
    fn test_last_errno() {
        run_for_all_setups(|file, _mode, _kind| {
            assert_eq!(file.last_errno(), 0);
        });
    }

    #[test]
    fn test_hint_size() {
        run_for_all_setups(|file, _mode, _kind| {
            let len = file.len().unwrap();
            file.hint_size(len as i64).unwrap();
        });
    }

    #[test]
    fn test_hint_overwrite() {
        run_for_all_setups(|file, _mode, _kind| {
            let len = file.len().unwrap();
            if let Err(err) = file.hint_overwrite(len) {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
        });
    }

    #[test]
    fn test_set_chunk_size() {
        run_for_all_setups(|file, _mode, _kind| {
            if let Err(err) = file.set_chunk_size(4096) {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
        });
    }

    #[test]
    fn test_mmap_size() {
        run_for_all_setups(|file, _mode, _kind| {
            const MMAP_SIZE: u64 = 10 * 1024 * 1024;
            if let Err(err) = file.set_mmap_size(MMAP_SIZE) {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
            let mmap_size = file.mmap_size();
            assert!(mmap_size == MMAP_SIZE);
        });
    }

    #[test]
    fn test_has_moved() {
        run_for_all_setups(|file, _mode, _kind| {
            assert!(!file.has_moved());
        });
    }

    #[test]
    fn test_pre_sync_single_db() {
        run_for_all_setups(|file, _mode, _kind| {
            if let Err(err) = file.pre_sync_single_db() {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
        });
    }

    #[test]
    fn test_pre_sync_multiple_db() {
        run_for_all_setups(|file, _mode, _kind| {
            let super_journal = VfsPath::new(OsStr::new("super_journal"));
            if let Err(err) = file.pre_sync_multiple_db(super_journal) {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
        });
    }

    #[test]
    fn test_commit_phase_two() {
        run_for_all_setups(|file, _mode, _kind| {
            if let Err(err) = file.commit_phase_two() {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
        });
    }

    #[test]
    fn test_begin_and_commit_atomic() {
        run_for_all_setups(|file, _mode, _kind| {
            if let Err(err) = file.begin_atomic() {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
            if let Err(err) = file.commit_atomic() {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
        });
    }

    #[test]
    fn test_rollback_atomic() {
        run_for_all_setups(|file, _mode, _kind| {
            if let Err(err) = file.begin_atomic() {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
            }
            file.rollback_atomic();
        });
    }

    #[test]
    fn test_lock_timeout() {
        run_for_all_setups(|file, _mode, _kind| {
            const TIMEOUT: Duration = Duration::from_millis(5000);
            if let Err(err) = file.set_lock_timeout(TIMEOUT) {
                assert_eq!(err.into_rc(), sqlite3::SQLITE_NOTFOUND);
                return;
            }
            let timeout = file.lock_timeout();
            assert_eq!(timeout, TIMEOUT);
        });
    }

    #[test]
    fn test_set_busy_handler() {
        run_for_all_setups(|file, _mode, _kind| {
            static INVOKED: AtomicBool = AtomicBool::new(false);
            INVOKED.store(false, Ordering::SeqCst);
            file.set_busy_handler(|| {
                INVOKED.store(true, Ordering::SeqCst);
                true
            });
            assert!(!INVOKED.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn test_wal_persistence() {
        run_setup(JournalMode::Delete, FileKind::Main, |file| {
            let persistent = file.is_wal_persistent();
            file.set_wal_persistent(!persistent);
            assert!(file.is_wal_persistent() != persistent);
            file.set_wal_persistent(persistent);
            assert!(file.is_wal_persistent() == persistent);
        });
    }

    #[test]
    fn test_hint_wal_lock() {
        run_for_all_setups(|file, _mode, _kind| {
            file.hint_wal_lock();
        });
    }

    #[test]
    fn test_hint_block_on_connect() {
        run_for_all_setups(|file, _mode, _kind| {
            file.hint_block_on_connect(true);
            file.hint_block_on_connect(false);
        });
    }

    #[test]
    fn test_checkpoint_operations() {
        run_for_all_setups(|file, _mode, _kind| {
            file.on_checkpoint_start();
            file.on_checkpoint_done();
        });
    }
}
