use std::borrow::Cow;
use std::error::Error;
use std::ffi::{CStr, CString, OsStr, c_char, c_int};
use std::fmt::Display;
use std::marker::PhantomData;
use std::num::NonZero;
use std::os::raw::c_void;
use std::os::unix::ffi::OsStrExt;
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::atomic::{self, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};
use std::{fmt, mem, slice};

use libsqlite3_sys::{
    self as sqlite3, sqlite3_file, sqlite3_filename, sqlite3_int64, sqlite3_io_methods, sqlite3_vfs,
};
use rand::RngCore;

use super::{Result, SqliteCode, SqliteError, ToCodeResultExt, WriteOutputResultExt};

/// Flags passed to [`Vfs::open`].
pub struct OpenFlags {
    bits: c_int,
}

impl OpenFlags {
    /// Creates `OpenFlags` from raw flags.
    fn new(bits: c_int) -> Self {
        let open_flags = Self { bits };

        // The following checks match SQLite exactly.

        debug_assert!(
            (!open_flags.read_only() || !open_flags.read_write())
                && (open_flags.read_write() || open_flags.read_only())
        );
        debug_assert!(!open_flags.create() || open_flags.read_write());
        debug_assert!(!open_flags.exclusive() || open_flags.create());
        debug_assert!(!open_flags.delete_on_close() || open_flags.create());

        debug_assert!(!open_flags.delete_on_close() || open_flags.file_type() != FileType::MainDb);
        debug_assert!(
            !open_flags.delete_on_close() || open_flags.file_type() != FileType::MainJournal
        );
        debug_assert!(
            !open_flags.delete_on_close() || open_flags.file_type() != FileType::SuperJournal
        );
        debug_assert!(!open_flags.delete_on_close() || open_flags.file_type() != FileType::Wal);

        debug_assert!(
            open_flags.file_type() == FileType::MainDb
                || open_flags.file_type() == FileType::TempDb
                || open_flags.file_type() == FileType::MainJournal
                || open_flags.file_type() == FileType::TempJournal
                || open_flags.file_type() == FileType::Subjournal
                || open_flags.file_type() == FileType::SuperJournal
                || open_flags.file_type() == FileType::TransientDb
                || open_flags.file_type() == FileType::Wal,
        );

        open_flags
    }

    /// Returns the type of file being opened.
    pub fn file_type(&self) -> FileType {
        match self.bits & 0x0FFF00 {
            sqlite3::SQLITE_OPEN_MAIN_DB => FileType::MainDb,
            sqlite3::SQLITE_OPEN_MAIN_JOURNAL => FileType::MainJournal,
            sqlite3::SQLITE_OPEN_TEMP_DB => FileType::TempDb,
            sqlite3::SQLITE_OPEN_TEMP_JOURNAL => FileType::TempJournal,
            sqlite3::SQLITE_OPEN_TRANSIENT_DB => FileType::TransientDb,
            sqlite3::SQLITE_OPEN_SUBJOURNAL => FileType::Subjournal,
            sqlite3::SQLITE_OPEN_SUPER_JOURNAL => FileType::SuperJournal,
            sqlite3::SQLITE_OPEN_WAL => FileType::Wal,
            _ => panic!("internal error: invalid file type"),
        }
    }

    /// Returns whether the file should be created if it doesn't exist.
    pub fn create(&self) -> bool {
        (self.bits & sqlite3::SQLITE_OPEN_CREATE) != 0
    }

    /// Returns whether the file is opened in read-only mode.
    pub fn read_only(&self) -> bool {
        (self.bits & sqlite3::SQLITE_OPEN_READONLY) != 0
    }

    /// Returns whether the file is opened in read-write mode.
    pub fn read_write(&self) -> bool {
        (self.bits & sqlite3::SQLITE_OPEN_READWRITE) != 0
    }

    /// Returns whether the file should be deleted when closed.
    pub fn delete_on_close(&self) -> bool {
        (self.bits & sqlite3::SQLITE_OPEN_DELETEONCLOSE) != 0
    }

    /// Returns whether the file should be opened exclusively.
    pub fn exclusive(&self) -> bool {
        (self.bits & sqlite3::SQLITE_OPEN_EXCLUSIVE) != 0
    }

    /// Returns whether the autoproxy locking style should be used.
    #[allow(unused)]
    pub fn autoproxy(&self) -> bool {
        (self.bits & sqlite3::SQLITE_OPEN_AUTOPROXY) != 0
    }
}

/// The type of file being opened.
#[derive(Debug, PartialEq, Eq)]
pub enum FileType {
    MainDb,
    MainJournal,
    TempDb,
    TempJournal,
    TransientDb,
    Subjournal,
    SuperJournal,
    Wal,
}

/// Represents a sqlite virtual file system.
///
/// This trait abstracts [sqlite3_vfs](https://www.sqlite.org/c3ref/vfs.html).
pub trait Vfs: Sync {
    /// The type of files stored within this VFS.
    type File: VfsFile;

    /// Opens a file. Returns the file and the actual flags used.
    fn open(&self, name: Option<VfsPath<'_>>, flags: OpenFlags) -> Result<(Self::File, OpenFlags)>;
    /// Deletes a file, optionally syncing the directory afterward.
    fn delete(&self, name: VfsPath<'_>, sync_dir: bool) -> Result<()>;
    /// Checks if a file exists.
    fn exists(&self, name: VfsPath<'_>) -> Result<bool>;
    /// Checks if a file is readable.
    fn can_read(&self, name: VfsPath<'_>) -> Result<bool>;
    /// Checks if a file is writable.
    fn can_write(&self, name: VfsPath<'_>) -> Result<bool>;
    /// Writes the full pathname of a file to the output buffer.
    fn write_full_path(&self, name: VfsPath<'_>, out: &mut [u8]) -> Result<usize>;
    /// Returns the last error code.
    fn last_error(&self) -> SqliteCode;

    /// Fills a buffer with random bytes.
    fn fill_random_bytes(&self, out: &mut [u8]) -> Result<()> {
        let mut rng = rand::rng();
        rng.fill_bytes(out);
        Ok(())
    }

    /// Sleeps for the given duration.
    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }

    /// Returns the current system time.
    fn now(&self) -> Result<SystemTime> {
        Ok(SystemTime::now())
    }
}

/// A file path passed to VFS operations.
#[allow(unused)]
pub struct VfsPath<'a>(&'a OsStr);

#[allow(unused)]
impl<'a> VfsPath<'a> {
    pub fn new(path: &'a OsStr) -> Self {
        Self(path)
    }

    /// Returns the inner path as an `OsStr`.
    pub fn inner(&self) -> &OsStr {
        self.0
    }
}

/// Represents the most basic file I/O bahaviours required by a [`Vfs`].
///
/// This trait is optional and corresponds to [`sqlite3_io_methods` v1](https://www.sqlite.org/c3ref/io_methods.html).
pub trait VfsFile {
    /// Reads from the file at an offset.
    ///
    /// See [xRead](https://www.sqlite.org/c3ref/io_methods.html#xRead).
    fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<()>;

    /// Writes to the file at an offset.
    ///
    /// See [xWrite](https://www.sqlite.org/c3ref/io_methods.html#xWrite).
    fn write_at(&mut self, buf: &[u8], offset: u64) -> Result<()>;

    /// Truncates the file to a size.
    ///
    /// See [xTruncate](https://www.sqlite.org/c3ref/io_methods.html#xTruncate).
    fn truncate(&mut self, size: u64) -> Result<()>;

    /// Syncs the file to disk.
    ///
    /// See [xSync](https://www.sqlite.org/c3ref/io_methods.html#xSync).
    fn sync(&mut self, op: SyncOptions) -> Result<()>;

    /// Gets the file size.
    ///
    /// See [xFileSize](https://www.sqlite.org/c3ref/io_methods.html#xFileSize).
    fn len(&self) -> Result<u64>;

    /// Acquires a file lock at the given `level`.
    ///
    /// See [xLock](https://www.sqlite.org/c3ref/io_methods.html#xLock).
    fn lock(&mut self, level: LockLevel) -> Result<()>;

    /// Releases a file lock at the given `level`.
    ///
    /// See [xUnlock](https://www.sqlite.org/c3ref/io_methods.html#xUnlock).
    fn unlock(&mut self, level: LockLevel) -> Result<()>;

    /// Checks if a write lock is held.
    ///
    /// See [xCheckReservedLock](https://www.sqlite.org/c3ref/io_methods.html#xCheckReservedLock).
    fn is_write_locked(&self) -> Result<bool>;

    /// Gets the sector size.
    ///
    /// See [xSectorSize](https://www.sqlite.org/c3ref/io_methods.html#xSectorSize).
    fn sector_len(&self) -> u32;

    /// Gets I/O characteristics.
    ///
    /// See [xDeviceCharacteristics](https://www.sqlite.org/c3ref/io_methods.html#xDeviceCharacteristics).
    fn io_capabilities(&self) -> IoCapabilities;

    /// Gets the current lock state.
    ///
    /// See [SQLITE_FCNTL_LOCKSTATE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllockstate).
    fn lock_level(&self) -> LockLevel;

    /// Gets the last OS error number.
    ///
    /// See [SQLITE_FCNTL_LAST_ERRNO](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllasterrno).
    fn last_errno(&self) -> i32;

    /// Handles the size hint for a transaction.
    ///
    /// See [SQLITE_FCNTL_SIZE_HINT](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlsizehint).
    fn hint_size(&mut self, size: i64) -> Result<()> {
        let _ = size;
        Ok(())
    }

    /// Hints that subsequent writes overwrite existing content.
    ///
    /// See [SQLITE_FCNTL_OVERWRITE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntloverwrite).
    fn hint_overwrite(&mut self, size: u64) -> Result<()> {
        let _ = size;
        Err(SqliteError::from_rc(sqlite3::SQLITE_NOTFOUND).unwrap())
    }

    /// Sets the database chunk size.
    ///
    /// See [SQLITE_FCNTL_CHUNK_SIZE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlchunksize).
    fn set_chunk_size(&mut self, size: u32) -> Result<()> {
        let _ = size;
        Ok(())
    }

    /// Handles PRAGMA forwarding.
    ///
    /// See [SQLITE_FCNTL_PRAGMA](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpragma).
    fn pragma(&mut self, name: &str, arg: Option<&str>) -> PragmaResult {
        let _ = name;
        let _ = arg;
        Err(PragmaError::from(
            SqliteError::from_rc(sqlite3::SQLITE_NOTFOUND).unwrap(),
        ))
    }

    /// Sets the max mmap size.
    ///
    /// See [SQLITE_FCNTL_MMAP_SIZE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlmmapsize).
    fn set_mmap_size(&mut self, size: u64) -> Result<()> {
        let _ = size;
        Ok(())
    }

    /// Gets the max mmap size.
    ///
    /// See [SQLITE_FCNTL_MMAP_SIZE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlmmapsize).
    fn mmap_size(&self) -> u64 {
        0
    }

    /// Reports whether the file has moved.
    ///
    /// See [SQLITE_FCNTL_HAS_MOVED](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcnthasmoved).
    fn has_moved(&self) -> bool {
        false
    }

    /// Pre-sync hook for a single database.
    ///
    /// See [SQLITE_FCNTL_SYNC](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlsync).
    fn pre_sync_single_db(&mut self) -> Result<()> {
        Ok(())
    }

    /// Pre-sync hook for multiple databases (with super-journal).
    ///
    /// See [SQLITE_FCNTL_SYNC](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlsync).
    fn pre_sync_multiple_db(&mut self, super_journal: VfsPath<'_>) -> Result<()> {
        let _ = super_journal;
        Ok(())
    }

    /// Completes commit phase two.
    ///
    /// See [SQLITE_FCNTL_COMMIT_PHASETWO](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlcommitphasetwo).
    fn commit_phase_two(&mut self) -> Result<()> {
        Ok(())
    }

    // /// Sets the parent connection.
    // ///
    // /// See [SQLITE_FCNTL_PDB](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpdb)
    // fn set_parent_connection<'a>(&'a mut self, conn: Connection<'a>) {
    //     let _ = conn;
    // }

    /// Begins an atomic-write sequence.
    ///
    /// See [SQLITE_FCNTL_BEGIN_ATOMIC_WRITE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlbeginatomicwrite).
    fn begin_atomic(&mut self) -> Result<()> {
        Ok(())
    }

    /// Commits an atomic-write sequence.
    ///
    /// See [SQLITE_FCNTL_COMMIT_ATOMIC_WRITE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlcommitatomicwrite).
    fn commit_atomic(&mut self) -> Result<()> {
        Ok(())
    }

    /// Rolls back an atomic-write sequence.
    ///
    /// See [SQLITE_FCNTL_ROLLBACK_ATOMIC_WRITE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlrollbackatomicwrite).
    fn rollback_atomic(&mut self) {}

    /// Gets the lock timeout.
    ///
    /// See [SQLITE_FCNTL_LOCK_TIMEOUT](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllocktimeout).
    fn lock_timeout(&self) -> Duration {
        Duration::from_millis(0)
    }

    /// Sets the busy handler.
    ///
    /// See [SQLITE_FCNTL_BUSYHANDLER](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlbusyhandler).
    fn set_busy_handler(&mut self, handler: impl Fn() -> bool + 'static) {
        let _ = handler;
    }

    /// Sets the lock timeout.
    ///
    /// See [SQLITE_FCNTL_LOCK_TIMEOUT](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllocktimeout).
    fn set_lock_timeout(&mut self, timeout: Duration) -> Result<()> {
        let _ = timeout;
        Ok(())
    }

    /// Gets WAL persistence.
    ///
    /// See [SQLITE_FCNTL_PERSIST_WAL](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpersistwal).
    fn is_wal_persistent(&self) -> bool {
        false
    }

    /// Sets WAL persistence.
    ///
    /// See [SQLITE_FCNTL_PERSIST_WAL](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpersistwal).
    fn set_wal_persistent(&mut self, persist: bool) {
        let _ = persist;
    }

    /// Hints WAL lock behavior.
    ///
    /// See [SQLITE_FCNTL_WAL_BLOCK](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlwalblock).
    fn hint_wal_lock(&mut self) {}

    /// Controls blocking behavior during connect.
    ///
    /// See [SQLITE_FCNTL_BLOCK_ON_CONNECT](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlblockonconnect).
    fn hint_block_on_connect(&mut self, block: bool) {
        let _ = block;
    }

    /// Signals checkpoint start.
    ///
    /// See [SQLITE_FCNTL_CKPT_START](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlckptstart).
    fn on_checkpoint_start(&mut self) {}

    /// Signals checkpoint completion.
    ///
    /// See [SQLITE_FCNTL_CKPT_DONE](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlckptdone).
    fn on_checkpoint_done(&mut self) {}
}

/// Represents pragma operation results.
pub type PragmaResult = std::result::Result<Option<Cow<'static, str>>, PragmaError>;

/// Represents errors in pragma operations.
#[derive(Debug)]
pub struct PragmaError {
    pub code: SqliteError,
    pub message: Option<Cow<'static, str>>,
}

impl PragmaError {
    #[allow(unused)]
    pub fn new(code: SqliteError, message: impl Into<Cow<'static, str>>) -> Self {
        PragmaError {
            code,
            message: Some(message.into()),
        }
    }
}

impl From<SqliteError> for PragmaError {
    fn from(code: SqliteError) -> Self {
        PragmaError {
            code,
            message: None,
        }
    }
}

impl Display for PragmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { code, message } = self;
        match message {
            Some(msg) => write!(f, "{code}: {msg}"),
            None => write!(f, "{}", code),
        }
    }
}

impl Error for PragmaError {}

/// Represents file I/O behaviours required to use a write-ahead log with shared-memory support
/// with a [`Vfs`].
///
/// This trait corresponds to [`sqlite3_io_methods` v2](https://www.sqlite.org/c3ref/io_methods.html).
pub trait VfsWalFile: VfsFile {
    /// Maps a shared-memory region.
    ///
    /// See [xShmMap](https://www.sqlite.org/c3ref/io_methods.html#xShmMap).
    fn map_shm(
        &mut self,
        region_number: NonZero<u32>,
        region_size: usize,
        extend: bool,
    ) -> Result<&mut [u8]>;

    /// Acquires a shared-memory lock.
    ///
    /// See [xShmLock](https://www.sqlite.org/c3ref/io_methods.html#xShmLock).
    fn lock_shm(&mut self, locks: WalLock, mode: WalLockMode) -> Result<()>;

    /// Releases a shared-memory lock.
    ///
    /// See [xShmLock](https://www.sqlite.org/c3ref/io_methods.html#xShmLock).
    fn unlock_shm(&mut self, locks: WalLock, mode: WalLockMode) -> Result<()>;

    /// Unmaps the shared-memory, optionally deleting.
    ///
    /// See [xShmUnmap](https://www.sqlite.org/c3ref/io_methods.html#xShmUnmap).
    fn unmap_shm(&mut self, delete: bool) -> Result<()>;

    /// Issues a memory barrier.
    ///
    /// See [xShmBarrier](https://www.sqlite.org/c3ref/io_methods.html#xShmBarrier).
    fn barrier(&mut self) {
        atomic::fence(Ordering::SeqCst);
    }
}

/// Lock mode for WAL shared-memory operations.
#[derive(Copy, Clone, Debug)]
#[allow(unused)]
pub enum WalLockMode {
    Shared,
    Exclusive,
}

impl WalLockMode {
    /// Converts from raw SQLite flags.
    pub fn from_raw(raw: c_int) -> Self {
        if raw & sqlite3::SQLITE_SHM_SHARED != 0 {
            WalLockMode::Shared
        } else if raw & sqlite3::SQLITE_SHM_EXCLUSIVE != 0 {
            WalLockMode::Exclusive
        } else {
            panic!("internal error: invalid wal lock mode");
        }
    }

    #[allow(unused)]
    /// Converts to raw SQLite flags.
    pub fn to_raw(&self) -> c_int {
        match self {
            WalLockMode::Shared => sqlite3::SQLITE_SHM_SHARED,
            WalLockMode::Exclusive => sqlite3::SQLITE_SHM_EXCLUSIVE,
        }
    }
}

/// A set representing WAL locks.
#[allow(unused)]
pub struct WalLock {
    mask: u16,
}

#[allow(unused)]
impl WalLock {
    pub const WAL_WRITE_LOCK: usize = 0;
    pub const WAL_CKPT_LOCK: usize = 1;
    pub const WAL_RECOVER_LOCK: usize = 2;
    pub const WAL_READ_LOCK_0: usize = 3;

    /// Creates a set from an offset and count.
    pub const fn new(offset: usize, n: usize) -> Self {
        let mask: u16 = (1 << (offset + n)) - (1 << offset);
        WalLock { mask }
    }

    /// Creates a set from a raw u16 mask.
    pub const fn from_mask(mask: u16) -> Self {
        WalLock { mask }
    }

    /// Returns true if the write lock is included.
    pub fn write(&self) -> bool {
        self.mask & (1 << Self::WAL_WRITE_LOCK) != 0
    }

    /// Returns true if the checkpoint lock is included.
    pub fn checkpoint(&self) -> bool {
        self.mask & (1 << Self::WAL_CKPT_LOCK) != 0
    }

    /// Returns true if the recover lock is included.
    pub fn recover(&self) -> bool {
        self.mask & (1 << Self::WAL_RECOVER_LOCK) != 0
    }

    /// Returns true if the given read lock index is included.
    /// index must be in 0..5
    pub fn read(&self, index: usize) -> bool {
        if index >= 5 {
            return false;
        }
        self.mask & (1 << (Self::WAL_READ_LOCK_0 + index)) != 0
    }
}

/// Represents file I/O behaviours for in-memory page access with a [`Vfs`].
///
/// This trait is optional and corresponds to ([`sqlite3_io_methods` v3](https://www.sqlite.org/c3ref/io_methods.html)).
pub trait VfsFetchFile: VfsFile {
    /// Fetches a page region into memory.
    ///
    /// See [xFetch](https://www.sqlite.org/c3ref/io_methods.html#xFetch).
    fn fetch(&mut self, offset: i64, amount: NonZero<usize>) -> Result<&mut [u8]>;

    /// Releases a previously fetched region.
    ///
    /// See [xUnfetch](https://www.sqlite.org/c3ref/io_methods.html#xUnfetch).
    fn unfetch(&mut self, offset: i64, ptr: NonNull<u8>) -> Result<()>;
}

/// Options for syncing a file.
#[allow(unused)]
pub struct SyncOptions {
    /// True for Mac OS X style fullsync, false for Unix style fsync
    pub full: bool,
    /// True to sync only the data of the file and not its inode (fdatasync)
    pub data_only: bool,
}

#[allow(unused)]
impl SyncOptions {
    pub fn to_raw(&self) -> c_int {
        let mut flags = 0;
        if self.full {
            flags |= sqlite3::SQLITE_SYNC_FULL;
        }
        if self.data_only {
            flags |= sqlite3::SQLITE_SYNC_DATAONLY;
        }
        flags
    }
}

/// File locking levels. See [File Locking](https://www.sqlite.org/lockingv3.html).
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LockLevel {
    None,
    Shared,
    Reserved,
    Pending,
    Exclusive,
}

impl LockLevel {
    pub(crate) fn from_raw(level: c_int) -> Self {
        match level {
            sqlite3::SQLITE_LOCK_NONE => LockLevel::None,
            sqlite3::SQLITE_LOCK_SHARED => LockLevel::Shared,
            sqlite3::SQLITE_LOCK_RESERVED => LockLevel::Reserved,
            sqlite3::SQLITE_LOCK_PENDING => LockLevel::Pending,
            sqlite3::SQLITE_LOCK_EXCLUSIVE => LockLevel::Exclusive,
            _ => panic!("invalid lock level"),
        }
    }

    pub(crate) fn to_raw(&self) -> c_int {
        match self {
            LockLevel::None => sqlite3::SQLITE_LOCK_NONE,
            LockLevel::Shared => sqlite3::SQLITE_LOCK_SHARED,
            LockLevel::Reserved => sqlite3::SQLITE_LOCK_RESERVED,
            LockLevel::Pending => sqlite3::SQLITE_LOCK_PENDING,
            LockLevel::Exclusive => sqlite3::SQLITE_LOCK_EXCLUSIVE,
        }
    }
}

/// I/O characteristics reported by [`VfsFile::io_capabilities`].
#[derive(Clone, Debug, Default)]
pub struct IoCapabilities {
    pub write_cap: AtomicWrite,
    pub safe_append: bool,
    pub sequential: bool,
    pub undeletable_when_open: bool,
    pub powersafe_overwrite: bool,
    pub immutable: bool,
    pub batch_atomic: bool,
    pub subpage_read: bool,
}

#[allow(unused)]
impl IoCapabilities {
    pub fn from_raw(raw: c_int) -> Self {
        let write_cap = if raw == 0 {
            AtomicWrite::Never
        } else if (raw & sqlite3::SQLITE_IOCAP_ATOMIC) != 0 {
            AtomicWrite::Always
        } else {
            AtomicWrite::Block {
                size_512: (raw & sqlite3::SQLITE_IOCAP_ATOMIC512) != 0,
                size_1k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC1K) != 0,
                size_2k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC2K) != 0,
                size_4k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC4K) != 0,
                size_8k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC8K) != 0,
                size_16k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC16K) != 0,
                size_32k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC32K) != 0,
                size_64k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC64K) != 0,
            }
        };

        IoCapabilities {
            write_cap,
            safe_append: (raw & sqlite3::SQLITE_IOCAP_SAFE_APPEND) != 0,
            sequential: (raw & sqlite3::SQLITE_IOCAP_SEQUENTIAL) != 0,
            undeletable_when_open: (raw & sqlite3::SQLITE_IOCAP_UNDELETABLE_WHEN_OPEN) != 0,
            powersafe_overwrite: (raw & sqlite3::SQLITE_IOCAP_POWERSAFE_OVERWRITE) != 0,
            immutable: (raw & sqlite3::SQLITE_IOCAP_IMMUTABLE) != 0,
            batch_atomic: (raw & sqlite3::SQLITE_IOCAP_BATCH_ATOMIC) != 0,
            subpage_read: (raw & sqlite3::SQLITE_IOCAP_SUBPAGE_READ) != 0,
        }
    }

    pub fn to_raw(&self) -> c_int {
        let mut flags = 0;

        let IoCapabilities {
            write_cap,
            safe_append,
            sequential,
            undeletable_when_open,
            powersafe_overwrite,
            immutable,
            batch_atomic,
            subpage_read,
        } = self;

        match *write_cap {
            AtomicWrite::Never => {}
            AtomicWrite::Block {
                size_512,
                size_1k,
                size_2k,
                size_4k,
                size_8k,
                size_16k,
                size_32k,
                size_64k,
            } => {
                if size_512 {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC512;
                }
                if size_1k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC1K;
                }
                if size_2k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC2K;
                }
                if size_4k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC4K;
                }
                if size_8k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC8K;
                }
                if size_16k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC16K;
                }
                if size_32k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC32K;
                }
                if size_64k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC64K;
                }
            }
            AtomicWrite::Always => {
                flags |= sqlite3::SQLITE_IOCAP_ATOMIC;
            }
        }
        if *safe_append {
            flags |= sqlite3::SQLITE_IOCAP_SAFE_APPEND;
        }
        if *sequential {
            flags |= sqlite3::SQLITE_IOCAP_SEQUENTIAL;
        }
        if *undeletable_when_open {
            flags |= sqlite3::SQLITE_IOCAP_UNDELETABLE_WHEN_OPEN;
        }
        if *powersafe_overwrite {
            flags |= sqlite3::SQLITE_IOCAP_POWERSAFE_OVERWRITE;
        }
        if *immutable {
            flags |= sqlite3::SQLITE_IOCAP_IMMUTABLE;
        }
        if *batch_atomic {
            flags |= sqlite3::SQLITE_IOCAP_BATCH_ATOMIC;
        }
        if *subpage_read {
            flags |= sqlite3::SQLITE_IOCAP_SUBPAGE_READ;
        }
        flags
    }
}

/// Atomic write capabilities.
#[derive(Clone, Debug, Default)]
#[allow(unused)]
pub enum AtomicWrite {
    #[default]
    Never,
    Block {
        size_512: bool,
        size_1k: bool,
        size_2k: bool,
        size_4k: bool,
        size_8k: bool,
        size_16k: bool,
        size_32k: bool,
        size_64k: bool,
    },
    Always,
}

/// RAII guard that unregisters a VFS on drop.
pub struct VfsRegistrationGuard<V>(Arc<VfsStorage<V>>);

impl<V> Drop for VfsRegistrationGuard<V> {
    fn drop(&mut self) {
        let rc = unsafe {
            sqlite3::sqlite3_vfs_unregister(&self.0.base as *const sqlite3_vfs as *mut _)
        };
        if let Some(err) = SqliteError::from_rc(rc) {
            panic!("cannot unregister VFS: {err}");
        }
    }
}

/// Represents a lack of support for a category of file I/O methods.
pub struct NoSupport;

/// Stores info the I/O method categories supported by a [`Vfs`].
pub struct VfsSupport<T, W = NoSupport, F = NoSupport> {
    _base: PhantomData<T>,
    _wal_support: PhantomData<W>,
    _fetch_support: PhantomData<F>,
}

/// Extension trait to provide a pre-computed [`Vfs`] method table.
pub trait VfsMethodTableExt {
    /// The available methods derived from a [`VfsSupport`].
    const METHODS: sqlite3_io_methods;
}

// Base implementation without WAL and Fetch support
impl<T> VfsSupport<T, NoSupport, NoSupport>
where
    T: Vfs,
{
    const fn methods() -> sqlite3_io_methods {
        sqlite3_io_methods {
            iVersion: 1,
            xClose: Some(x_close::<T>),
            xRead: Some(x_read::<T>),
            xWrite: Some(x_write::<T>),
            xTruncate: Some(x_truncate::<T>),
            xSync: Some(x_sync::<T>),
            xFileSize: Some(x_file_size::<T>),
            xLock: Some(x_lock::<T>),
            xUnlock: Some(x_unlock::<T>),
            xCheckReservedLock: Some(x_check_reserved_lock::<T>),
            xFileControl: Some(x_file_control::<T>),
            xSectorSize: Some(x_sector_size::<T>),
            xDeviceCharacteristics: Some(x_device_characteristics::<T>),

            // No WAL support
            xShmMap: None,
            xShmLock: None,
            xShmBarrier: None,
            xShmUnmap: None,

            // No Fetch support
            xFetch: None,
            xUnfetch: None,
        }
    }
}

impl<T> VfsMethodTableExt for VfsSupport<T, NoSupport, NoSupport>
where
    T: Vfs,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

// Wal support implementation
impl<F, T> VfsSupport<T, F, NoSupport>
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    const fn methods() -> sqlite3_io_methods {
        let mut methods = VfsSupport::<T>::methods();
        methods.iVersion = 2;
        methods.xShmMap = Some(x_shm_map::<T, F>);
        methods.xShmLock = Some(x_shm_lock::<T, F>);
        methods.xShmBarrier = Some(x_shm_barrier::<T, F>);
        methods.xShmUnmap = Some(x_shm_unmap::<T, F>);
        methods
    }
}

impl<F, T> VfsMethodTableExt for VfsSupport<T, F, NoSupport>
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

// Fetch support implementation
impl<T, F> VfsSupport<T, NoSupport, F>
where
    F: VfsFetchFile,
    T: Vfs<File = F>,
{
    const fn methods() -> sqlite3_io_methods {
        let mut methods = VfsSupport::<T>::methods();
        methods.iVersion = 3;
        methods.xFetch = Some(x_fetch::<T, F>);
        methods.xUnfetch = Some(x_unfetch::<T, F>);
        methods
    }
}

impl<T, F> VfsMethodTableExt for VfsSupport<T, NoSupport, F>
where
    F: VfsFetchFile,
    T: Vfs<File = F>,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

impl<F, T> VfsSupport<T, F, F>
where
    F: VfsFetchFile + VfsWalFile,
    T: Vfs<File = F>,
{
    const fn methods() -> sqlite3_io_methods {
        let mut methods = VfsSupport::<T, F>::methods();
        methods.iVersion = 3;
        methods.xFetch = Some(x_fetch::<T, F>);
        methods.xUnfetch = Some(x_unfetch::<T, F>);
        methods
    }
}

impl<T, F> VfsMethodTableExt for VfsSupport<T, F, F>
where
    F: VfsFetchFile + VfsWalFile,
    T: Vfs<File = F>,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

/// Builder for VFS registration.
pub struct VfsRegistration<T, M> {
    vfs: T,
    max_pathlen: usize,
    make_default: bool,
    method_table: std::marker::PhantomData<M>,
}

impl<T: Vfs> VfsRegistration<T, VfsSupport<T>> {
    #[allow(unused)]
    /// Creates a new VFS registration builder.
    pub fn new(vfs: T) -> Self {
        Self {
            vfs,
            max_pathlen: 512,
            make_default: false,
            method_table: std::marker::PhantomData,
        }
    }
}

impl<T: Vfs, M: VfsMethodTableExt> VfsRegistration<T, M> {
    #[allow(unused)]
    /// Registers the VFS with SQLite.
    pub fn register(self, name: &str) -> Result<VfsRegistrationGuard<T>> {
        if name.is_empty() {
            return Err(SqliteError::from_rc(sqlite3::SQLITE_MISUSE).unwrap());
        }

        let Self {
            vfs,
            max_pathlen,
            make_default,
            method_table: _,
        } = self;

        let storage = Arc::new_cyclic(move |storage| {
            let name = CString::new(name).unwrap();
            let base = sqlite3_vfs {
                iVersion: 2,
                szOsFile: std::mem::size_of::<VfsFileStorage<T>>() as c_int,
                mxPathname: max_pathlen as c_int,
                pNext: ptr::null_mut(),
                zName: name.as_ptr(),
                pAppData: storage.as_ptr() as *mut c_void,
                xOpen: Some(x_open::<T, M>),
                xDelete: Some(x_delete::<T>),
                xAccess: Some(x_access::<T>),
                xFullPathname: Some(x_full_pathname::<T>),

                // FIXME: support for non-unix systems
                xDlOpen: Some(x_dlopen),
                xDlError: Some(x_dlerror),
                xDlSym: Some(x_dlsym),
                xDlClose: Some(x_dlclose),

                xRandomness: Some(x_randomness::<T>),
                xSleep: Some(x_sleep::<T>),
                xCurrentTime: Some(x_get_current_time_deprecated),
                xGetLastError: Some(x_get_last_error::<T>),
                xCurrentTimeInt64: Some(x_get_current_time::<T>),

                // NOTE: nice to have, but not strictly needed
                xSetSystemCall: None,
                xGetSystemCall: None,
                xNextSystemCall: None,
            };
            VfsStorage { base, name, vfs }
        });

        let rc = unsafe {
            sqlite3::sqlite3_vfs_register(
                &storage.base as *const sqlite3_vfs as *mut _,
                make_default as c_int,
            )
        };
        if let Some(err) = SqliteError::from_rc(rc) {
            return Err(err);
        }
        Ok(VfsRegistrationGuard(storage))
    }
}

impl<T, M> VfsRegistration<T, M> {
    #[allow(unused)]
    /// Sets the maximum path length supported by the VFS.
    pub fn max_pathlen(mut self, len: usize) -> Self {
        self.max_pathlen = len;
        self
    }

    #[allow(unused)]
    /// Makes this VFS the default one.
    pub fn make_default(mut self) -> Self {
        self.make_default = true;
        self
    }
}

impl<T: Vfs, Wal> VfsRegistration<T, VfsSupport<T, Wal, NoSupport>>
where
    T::File: VfsFetchFile,
{
    #[allow(unused)]
    /// Enables fetch support (io_methods v3).
    pub fn with_fetch(self) -> VfsRegistration<T, VfsSupport<T, Wal, T::File>> {
        let Self {
            vfs,
            max_pathlen,
            make_default,
            method_table: _,
        } = self;
        VfsRegistration {
            vfs,
            max_pathlen,
            make_default,
            method_table: std::marker::PhantomData,
        }
    }
}

impl<T: Vfs, Fetch> VfsRegistration<T, VfsSupport<T, NoSupport, Fetch>>
where
    T::File: VfsWalFile,
{
    #[allow(unused)]
    /// Enables WAL support (io_methods v2).
    pub fn with_wal(self) -> VfsRegistration<T, VfsSupport<T, T::File, Fetch>> {
        let Self {
            vfs,
            max_pathlen,
            make_default,
            method_table: _,
        } = self;
        VfsRegistration {
            vfs,
            max_pathlen,
            make_default,
            method_table: std::marker::PhantomData,
        }
    }
}

struct VfsStorage<V> {
    base: sqlite3_vfs,
    name: CString,
    vfs: V,
}

impl<T> VfsStorage<T> {
    unsafe fn from_raw(ptr: *mut sqlite3_vfs) -> Arc<Self> {
        let vfs = unsafe { ptr.as_ref() }.expect("cannot get reference to empty vfs storage");
        let storage_ptr = vfs.pAppData.cast::<VfsStorage<T>>();
        if storage_ptr.is_null() {
            panic!("cannot get reference to empty vfs storage");
        }
        unsafe {
            Arc::increment_strong_count(storage_ptr);
            Arc::from_raw(storage_ptr)
        }
    }
}

#[repr(C)]
struct VfsFileStorage<T: Vfs> {
    base: sqlite3_file,
    state: FileStorageState<T>,
}

enum FileStorageState<T: Vfs> {
    Open {
        vfs: Arc<VfsStorage<T>>,
        file: T::File,
    },
    Closed,
}

impl<T: Vfs> VfsFileStorage<T> {
    /// Returns a mutable reference to the VfsFileStorage from a raw pointer.
    /// SAFETY: The reference is valid as long as the underlying pointer is valid,
    /// and should generally be used only within the scope of a function called by SQLite.
    unsafe fn from_raw<'sqlite>(ptr: *mut sqlite3_file) -> &'sqlite mut Self {
        unsafe {
            ptr.cast::<VfsFileStorage<T>>()
                .as_mut()
                .expect("cannot get reference to empty file storage")
        }
    }

    fn file(&mut self) -> &mut T::File {
        match &mut self.state {
            FileStorageState::Open { file, .. } => file,
            FileStorageState::Closed => panic!("internal error: file already closed"),
        }
    }

    fn vfs(&self) -> &VfsStorage<T> {
        match &self.state {
            FileStorageState::Open { vfs, .. } => vfs,
            FileStorageState::Closed => panic!("internal error: file already closed"),
        }
    }
}

unsafe extern "C" fn x_open<T: Vfs, M: VfsMethodTableExt>(
    vfs: *mut sqlite3_vfs,
    filename: sqlite3_filename,
    out: *mut sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let path = if filename.is_null() {
        if (flags & sqlite3::SQLITE_OPEN_DELETEONCLOSE) == 0 {
            return sqlite3::SQLITE_MISUSE;
        }
        None
    } else {
        Some(OsStr::from_bytes(
            unsafe { CStr::from_ptr(filename) }.to_bytes(),
        ))
    };

    let vfs_storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let path = path.map(VfsPath);
    let flags = OpenFlags::new(flags);
    let (file, flags) = match vfs_storage.vfs.open(path, flags) {
        Ok(r) => r,
        Err(e) => return e.into_rc(),
    };

    let methods: &'static sqlite3_io_methods = &M::METHODS;
    let storage = VfsFileStorage {
        base: sqlite3_file {
            pMethods: methods as *const _,
        },
        state: FileStorageState::Open {
            vfs: vfs_storage,
            file,
        },
    };
    unsafe {
        out_flags.write(flags.bits);
        out.cast::<VfsFileStorage<T>>().write(storage);
    }
    sqlite3::SQLITE_OK
}

unsafe extern "C" fn x_delete<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    filename: *const c_char,
    sync: c_int,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = OsStr::from_bytes(unsafe { CStr::from_ptr(filename) }.to_bytes());
    storage
        .vfs
        .delete(VfsPath(name), sync != 0)
        .to_code_result()
        .into_rc()
}

unsafe extern "C" fn x_access<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    filename: *const c_char,
    flags: c_int,
    outcome: *mut c_int,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = OsStr::from_bytes(unsafe { CStr::from_ptr(filename) }.to_bytes());
    let out = unsafe { outcome.as_mut().unwrap() };

    let result = match flags {
        sqlite3::SQLITE_ACCESS_EXISTS => storage.vfs.exists(VfsPath(name)),
        sqlite3::SQLITE_ACCESS_READ => storage.vfs.can_read(VfsPath(name)),
        sqlite3::SQLITE_ACCESS_READWRITE => storage.vfs.can_write(VfsPath(name)),
        _ => return sqlite3::SQLITE_MISUSE,
    };

    result.write_to_output(out).into_rc()
}

unsafe extern "C" fn x_full_pathname<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    name: *const c_char,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = OsStr::from_bytes(unsafe { CStr::from_ptr(name) }.to_bytes());
    let out_len = std::mem::size_of::<c_char>() * n_out as usize;
    let out_slice = unsafe { slice::from_raw_parts_mut(out as *mut u8, out_len) };

    storage
        .vfs
        .write_full_path(VfsPath(name), &mut out_slice[..(out_len - 1)])
        .map(|len| {
            // Null-terminate
            out_slice[len] = 0;
        })
        .to_code_result()
        .into_rc()
}

// On linux, these function are available by default in libc. On other platforms `-ldl` is probably needed.
// Also, this code is unix-only and does not work on windows.
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
}

unsafe extern "C" fn x_dlopen(_: *mut sqlite3_vfs, filename: *const c_char) -> *mut c_void {
    // Linux only, but that's ok as dqlite-utils is for linux only
    const RTLD_NOW: c_int = 1;
    const RTLD_GLOBAL: c_int = 4;

    unsafe { dlopen(filename, RTLD_NOW | RTLD_GLOBAL) }
}

unsafe extern "C" fn x_dlerror(_: *mut sqlite3_vfs, n: c_int, out: *mut c_char) {
    unsafe {
        let err = dlerror();
        if !err.is_null() {
            sqlite3::sqlite3_snprintf(n, out, c"%s".as_ptr() as *const c_char, err);
        }
    }
}

// FIXME: the return type of this function is wrong:
//  - either it should be a pointer to a function with generic signature in C like void(*)()
//  - or it should be the only actual use this function is used for:
//      unsafe extern "C" fn(*mut sqlite3, *mut *mut char, *const sqlite3_api_routines) -> c_int.
// See https://github.com/rust-lang/rust-bindgen/issues/2713
unsafe extern "C" fn x_dlsym(
    _: *mut sqlite3_vfs,
    p: *mut c_void,
    sym: *const c_char,
) -> Option<unsafe extern "C" fn(*mut sqlite3_vfs, *mut c_void, *const i8)> {
    Some(unsafe { std::mem::transmute(dlsym(p, sym)) })
}

unsafe extern "C" fn x_dlclose(_: *mut sqlite3_vfs, handle: *mut core::ffi::c_void) {
    unsafe { dlclose(handle) };
}

unsafe extern "C" fn x_randomness<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage
        .vfs
        .fill_random_bytes(unsafe { slice::from_raw_parts_mut(out as *mut u8, n_out as usize) })
        .to_code_result()
        .into_rc()
}

unsafe extern "C" fn x_sleep<T: Vfs>(vfs: *mut sqlite3_vfs, microseconds: c_int) -> c_int {
    if microseconds <= 0 {
        return 0;
    }
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage
        .vfs
        .sleep(Duration::from_micros(microseconds as u64));
    microseconds
}

unsafe extern "C" fn x_get_current_time_deprecated(_: *mut sqlite3_vfs, _: *mut f64) -> c_int {
    panic!("deprecated xCurrentTime called");
}

unsafe extern "C" fn x_get_current_time<T: Vfs>(vfs: *mut sqlite3_vfs, out_ptr: *mut i64) -> c_int {
    const UNIX_EPOCH: i64 = 24405875i64 * 8640000i64;

    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let out = unsafe { out_ptr.as_mut().unwrap() };
    storage
        .vfs
        .now()
        .map(|time| {
            time.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + UNIX_EPOCH
        })
        .write_to_output(out)
        .into_rc()
}

unsafe extern "C" fn x_get_last_error<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    _: c_int,
    _: *mut c_char,
) -> i32 {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage.vfs.last_error().into_rc()
}

unsafe extern "C" fn x_close<T: Vfs>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    storage.state = FileStorageState::Closed;
    sqlite3::SQLITE_OK
}

unsafe extern "C" fn x_read<T: Vfs>(
    file: *mut sqlite3_file,
    data: *mut c_void,
    amount: i32,
    offset: i64,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let buf = unsafe { slice::from_raw_parts_mut(data as *mut u8, amount as usize) };
    file.read_at(buf, offset as u64).to_code_result().into_rc()
}

unsafe extern "C" fn x_write<T: Vfs>(
    file: *mut sqlite3_file,
    data: *const c_void,
    amount: i32,
    offset: i64,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let buf = unsafe { slice::from_raw_parts(data as *const u8, amount as usize) };
    file.write_at(buf, offset as u64).to_code_result().into_rc()
}

unsafe extern "C" fn x_truncate<T: Vfs>(file: *mut sqlite3_file, size: i64) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.truncate(size as u64).to_code_result().into_rc()
}

unsafe extern "C" fn x_sync<T: Vfs>(file: *mut sqlite3_file, flags: c_int) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let options = SyncOptions {
        full: (flags & sqlite3::SQLITE_SYNC_FULL) != 0,
        data_only: (flags & sqlite3::SQLITE_SYNC_DATAONLY) != 0,
    };
    file.sync(options).to_code_result().into_rc()
}

unsafe extern "C" fn x_file_size<T: Vfs>(
    file: *mut sqlite3_file,
    out_ptr: *mut sqlite3_int64,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe { out_ptr.as_mut().unwrap() };
    file.len()
        .map(|size| size as i64)
        .write_to_output(out)
        .into_rc()
}

unsafe extern "C" fn x_lock<T: Vfs>(file: *mut sqlite3_file, level: c_int) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_level = LockLevel::from_raw(level);
    file.lock(lock_level).to_code_result().into_rc()
}

unsafe extern "C" fn x_unlock<T: Vfs>(file: *mut sqlite3_file, level: c_int) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_level = LockLevel::from_raw(level);
    file.unlock(lock_level).to_code_result().into_rc()
}

unsafe extern "C" fn x_check_reserved_lock<T: Vfs>(
    file: *mut sqlite3_file,
    out_ptr: *mut c_int,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe { out_ptr.as_mut().unwrap() };
    file.is_write_locked().write_to_output(out).into_rc()
}

unsafe extern "C" fn x_file_control<T: Vfs>(
    file: *mut sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    match op {
        sqlite3::SQLITE_FCNTL_LOCKSTATE => {
            let level = file.lock_level();
            unsafe { arg.cast::<c_int>().write(level.to_raw()) };
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_LAST_ERRNO => {
            let errno = file.last_errno();
            unsafe { arg.cast::<c_int>().write(errno) };
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_SIZE_HINT => {
            let size = unsafe { arg.cast::<i64>().read() };
            file.hint_size(size).to_code_result().into_rc()
        }
        sqlite3::SQLITE_FCNTL_CHUNK_SIZE => {
            let size = unsafe { arg.cast::<c_int>().read() } as u32;
            file.set_chunk_size(size).to_code_result().into_rc()
        }
        sqlite3::SQLITE_FCNTL_OVERWRITE => {
            let size = unsafe { arg.cast::<sqlite3_int64>().read() } as u64;
            file.hint_overwrite(size).to_code_result().into_rc()
        }
        sqlite3::SQLITE_FCNTL_VFSNAME => {
            let name_ptr = arg.cast::<*mut c_char>();
            unsafe {
                name_ptr.write(sqlite3::sqlite3_mprintf(
                    c"%s".as_ptr() as *const c_char,
                    storage.vfs().name.as_ptr(),
                ));
            }
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_PRAGMA => {
            // The arg is a pointer to an array of 3 *char:
            //   arg[0]: output **char (either error or result)
            //   arg[1]: input *char (pragma name)
            //   arg[2]: input *char or NULL (pragma argument)
            let args = unsafe { slice::from_raw_parts_mut(arg.cast::<*mut c_char>(), 3) };
            let name = str::from_utf8(unsafe { CStr::from_ptr(args[1]) }.to_bytes()).unwrap();
            let arg_raw = args[2];
            let arg = if !arg_raw.is_null() {
                Some(
                    str::from_utf8(unsafe { CStr::from_ptr(arg_raw) }.to_bytes())
                        .expect("internal error: pragma argument is not valid utf-8"),
                )
            } else {
                None
            };
            match file.pragma(name, arg) {
                Ok(Some(result_msg)) => {
                    unsafe {
                        args[0] = sqlite3::sqlite3_mprintf(
                            c"%*s".as_ptr() as *const c_char,
                            result_msg.len(),
                            result_msg.as_bytes(),
                        );
                    }
                    sqlite3::SQLITE_OK
                }
                Ok(None) => sqlite3::SQLITE_OK,
                Err(PragmaError {
                    code,
                    message: None,
                }) => code.into_rc(),
                Err(PragmaError {
                    code,
                    message: Some(err_msg),
                }) => {
                    unsafe {
                        args[0] = sqlite3::sqlite3_mprintf(
                            c"%*s".as_ptr() as *const c_char,
                            err_msg.len(),
                            err_msg.as_bytes(),
                        );
                    }
                    code.into_rc()
                }
            }
        }
        sqlite3::SQLITE_FCNTL_MMAP_SIZE => {
            let size = unsafe { arg.cast::<sqlite3_int64>().as_mut() }.unwrap();
            let new_size = *size;
            let old_size = file.mmap_size();
            *size = old_size as i64;
            if new_size < 0 {
                return sqlite3::SQLITE_OK;
            }
            file.set_mmap_size(new_size as u64)
                .to_code_result()
                .into_rc()
        }
        sqlite3::SQLITE_FCNTL_HAS_MOVED => {
            unsafe { arg.cast::<c_int>().write(file.has_moved() as c_int) };
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_SYNC => {
            let super_journal_raw = arg.cast::<c_char>();
            if super_journal_raw.is_null() {
                return file.pre_sync_single_db().to_code_result().into_rc();
            }
            file.pre_sync_multiple_db(VfsPath(OsStr::from_bytes(
                unsafe { CStr::from_ptr(super_journal_raw) }.to_bytes(),
            )))
            .to_code_result()
            .into_rc()
        }
        sqlite3::SQLITE_FCNTL_COMMIT_PHASETWO => file.commit_phase_two().to_code_result().into_rc(),
        sqlite3::SQLITE_FCNTL_PDB => {
            // TODO: this is blocked on rusqlite as a non-owning connection still changes the connection (it unregisters all hooks on drop)
            // See https://github.com/rusqlite/rusqlite/issues/1784
            // TODO: we should wrap the rusqlite::Connection in a newtype that includes a lifetime so that implementations cannot leak it.
            // Something like NonOwningConnection<'conn>

            // let pdb = unsafe { arg.cast::<*mut sqlite3>().read() };
            // let connection = unsafe { rusqlite::Connection::from_handle(pdb) }.unwrap();
            // file.set_parent_connection(connection);
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_BEGIN_ATOMIC_WRITE => file.begin_atomic().to_code_result().into_rc(),
        sqlite3::SQLITE_FCNTL_COMMIT_ATOMIC_WRITE => {
            file.commit_atomic().to_code_result().into_rc()
        }
        sqlite3::SQLITE_FCNTL_ROLLBACK_ATOMIC_WRITE => {
            file.rollback_atomic();
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_LOCK_TIMEOUT => {
            let timeout = unsafe { arg.cast::<i32>().as_mut() }.unwrap();
            let new_timeout = Duration::from_millis(*timeout as u64);
            let old_timeout = file.lock_timeout();
            *timeout = old_timeout.as_millis() as i32;
            file.set_lock_timeout(new_timeout)
                .to_code_result()
                .into_rc()
        }

        sqlite3::SQLITE_FCNTL_BUSYHANDLER => {
            let args = unsafe { slice::from_raw_parts(arg.cast::<*mut c_void>(), 2) };
            let busy_handler: extern "C" fn(*mut c_void) -> c_int =
                unsafe { mem::transmute(args[0]) };
            let busy_handler_arg = args[1];
            let wrapped_handler = move || {
                let rc = busy_handler(busy_handler_arg);
                rc != 0
            };
            file.set_busy_handler(wrapped_handler);
            sqlite3::SQLITE_OK
        }

        sqlite3::SQLITE_FCNTL_NULL_IO => {
            storage.state = FileStorageState::Closed;
            sqlite3::SQLITE_OK
        }

        sqlite3::SQLITE_FCNTL_PERSIST_WAL => {
            let persist = unsafe { arg.cast::<i32>().as_mut() }.unwrap();
            if *persist < 0 {
                *persist = file.is_wal_persistent() as i32;
                return sqlite3::SQLITE_OK;
            }
            file.set_wal_persistent(*persist != 0);
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_WAL_BLOCK => {
            file.hint_wal_lock();
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_BLOCK_ON_CONNECT => {
            let block = unsafe { arg.cast::<i32>().as_mut() }.unwrap();
            file.hint_block_on_connect(*block != 0);
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_CKPT_START => {
            file.on_checkpoint_start();
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_CKPT_DONE => {
            file.on_checkpoint_done();
            sqlite3::SQLITE_OK
        }

        // Not available as they are specific VFS detail
        sqlite3::SQLITE_FCNTL_GET_LOCKPROXYFILE
        | sqlite3::SQLITE_FCNTL_SET_LOCKPROXYFILE
        | sqlite3::SQLITE_FCNTL_SIZE_LIMIT
        | sqlite3::SQLITE_FCNTL_POWERSAFE_OVERWRITE
        | sqlite3::SQLITE_FCNTL_WIN32_GET_HANDLE
        | sqlite3::SQLITE_FCNTL_WIN32_SET_HANDLE
        | sqlite3::SQLITE_FCNTL_WIN32_AV_RETRY
        | sqlite3::SQLITE_FCNTL_ZIPVFS
        | sqlite3::SQLITE_FCNTL_RBU
        | sqlite3::SQLITE_FCNTL_CKSM_FILE
        | sqlite3::SQLITE_FCNTL_EXTERNAL_READER => sqlite3::SQLITE_NOTFOUND,

        // Should be implemented by SQLite core
        sqlite3::SQLITE_FCNTL_DATA_VERSION
        | sqlite3::SQLITE_FCNTL_RESERVE_BYTES
        | sqlite3::SQLITE_FCNTL_FILE_POINTER
        | sqlite3::SQLITE_FCNTL_JOURNAL_POINTER
        | sqlite3::SQLITE_FCNTL_VFS_POINTER
        | sqlite3::SQLITE_FCNTL_SYNC_OMITTED
        | sqlite3::SQLITE_FCNTL_RESET_CACHE => sqlite3::SQLITE_MISUSE,

        // Not supported.
        sqlite3::SQLITE_FCNTL_TRACE | sqlite3::SQLITE_FCNTL_TEMPFILENAME => sqlite3::SQLITE_OK,

        // Newer codes that we don't need to handle yet
        fcntl if fcntl <= 100 => sqlite3::SQLITE_NOTFOUND,

        // TODO: allow extensions to handle custom opcodes
        _ => sqlite3::SQLITE_NOTFOUND,
    }
}

unsafe extern "C" fn x_sector_size<T: Vfs>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.sector_len() as c_int
}

unsafe extern "C" fn x_device_characteristics<T: Vfs>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.io_capabilities().to_raw()
}

unsafe extern "C" fn x_shm_map<T, F>(
    file: *mut sqlite3_file,
    region: c_int,
    size: c_int,
    extend: c_int,
    out_ptr: *mut *mut c_void,
) -> c_int
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe { out_ptr.cast::<*mut u8>().as_mut().unwrap() };
    file.map_shm(
        NonZero::new(region as u32).unwrap(),
        size as usize,
        extend != 0,
    )
    .map(|s| s.as_mut_ptr())
    .write_to_output(out)
    .into_rc()
}

unsafe extern "C" fn x_shm_lock<T, F>(
    file: *mut sqlite3_file,
    offset: c_int,
    n: c_int,
    flags: c_int,
) -> c_int
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_mode = WalLockMode::from_raw(flags);
    let wal_lock = WalLock::new(offset as usize, n as usize);

    if flags & sqlite3::SQLITE_SHM_LOCK != 0 {
        file.lock_shm(wal_lock, lock_mode)
            .to_code_result()
            .into_rc()
    } else if flags & sqlite3::SQLITE_SHM_UNLOCK != 0 {
        file.unlock_shm(wal_lock, lock_mode)
            .to_code_result()
            .into_rc()
    } else {
        panic!("internal error: invalid shm lock flags");
    }
}

unsafe extern "C" fn x_shm_barrier<T, F>(file: *mut sqlite3_file)
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.barrier();
}

unsafe extern "C" fn x_shm_unmap<T, F>(file: *mut sqlite3_file, delete: c_int) -> c_int
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.unmap_shm(delete != 0).to_code_result().into_rc()
}

unsafe extern "C" fn x_fetch<T, F>(
    file: *mut sqlite3_file,
    offset: i64,
    amount: i32,
    out_ptr: *mut *mut c_void,
) -> c_int
where
    F: VfsFetchFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe { out_ptr.cast::<*mut u8>().as_mut().unwrap() };
    file.fetch(offset, NonZero::new(amount as usize).unwrap())
        .map(|s| s.as_mut_ptr())
        .write_to_output(out)
        .into_rc()
}

unsafe extern "C" fn x_unfetch<T, F>(
    file: *mut sqlite3_file,
    offset: i64,
    ptr: *mut c_void,
) -> c_int
where
    F: VfsFetchFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.unfetch(offset, NonNull::new(ptr as *mut u8).unwrap())
        .to_code_result()
        .into_rc()
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::CStr,
        io::Write,
        num::NonZero,
        os::unix::ffi::OsStrExt,
        ptr::NonNull,
        time::{Duration, SystemTime},
    };

    use libsqlite3_sys as sqlite3;

    use super::*;

    struct DummyVfs;

    impl Vfs for DummyVfs {
        type File = DummyFile;

        fn open(
            &self,
            _path: Option<VfsPath<'_>>,
            _flags: OpenFlags,
        ) -> Result<(Self::File, OpenFlags)> {
            Ok((DummyFile, _flags))
        }

        fn delete(&self, _path: VfsPath<'_>, _sync: bool) -> Result<()> {
            unimplemented!()
        }

        fn write_full_path(&self, path: VfsPath<'_>, mut out: &mut [u8]) -> Result<usize> {
            Ok(out.write(path.inner().as_bytes()).unwrap())
        }

        fn fill_random_bytes(&self, _out: &mut [u8]) -> Result<()> {
            unimplemented!()
        }

        fn sleep(&self, _duration: Duration) {
            unimplemented!()
        }

        fn now(&self) -> Result<SystemTime> {
            unimplemented!()
        }

        fn last_error(&self) -> SqliteCode {
            SqliteCode::OK
        }

        fn exists(&self, _name: VfsPath<'_>) -> Result<bool> {
            unimplemented!()
        }

        fn can_read(&self, _name: VfsPath<'_>) -> Result<bool> {
            unimplemented!()
        }

        fn can_write(&self, _name: VfsPath<'_>) -> Result<bool> {
            unimplemented!()
        }
    }

    struct DummyFile;

    impl VfsFile for DummyFile {
        fn read_at(&mut self, buf: &mut [u8], _offset: u64) -> Result<()> {
            buf.fill(0);
            Err(SqliteError::from_rc(sqlite3::SQLITE_IOERR_SHORT_READ).unwrap())
        }

        fn write_at(&mut self, _buf: &[u8], _offset: u64) -> Result<()> {
            Ok(())
        }

        fn truncate(&mut self, _size: u64) -> Result<()> {
            Ok(())
        }

        fn sync(&mut self, _op: SyncOptions) -> Result<()> {
            Ok(())
        }

        fn len(&self) -> Result<u64> {
            Ok(0)
        }

        fn lock(&mut self, _level: LockLevel) -> Result<()> {
            Ok(())
        }

        fn unlock(&mut self, _level: LockLevel) -> Result<()> {
            Ok(())
        }

        fn is_write_locked(&self) -> Result<bool> {
            Ok(false)
        }

        fn lock_level(&self) -> LockLevel {
            unimplemented!()
        }

        fn last_errno(&self) -> i32 {
            0
        }

        fn sector_len(&self) -> u32 {
            4096
        }

        fn io_capabilities(&self) -> IoCapabilities {
            IoCapabilities::default()
        }
    }

    impl VfsWalFile for DummyFile {
        fn map_shm(
            &mut self,
            _region_number: NonZero<u32>,
            _region_size: usize,
            _extend: bool,
        ) -> Result<&mut [u8]> {
            unimplemented!()
        }

        fn lock_shm(&mut self, _locks: WalLock, _mode: WalLockMode) -> Result<()> {
            unimplemented!()
        }

        fn unlock_shm(&mut self, _locks: WalLock, _mode: WalLockMode) -> Result<()> {
            unimplemented!()
        }

        fn unmap_shm(&mut self, _delete: bool) -> Result<()> {
            unimplemented!()
        }
    }

    impl VfsFetchFile for DummyFile {
        fn unfetch(&mut self, _offset: i64, _ptr: NonNull<u8>) -> Result<()> {
            Ok(())
        }

        fn fetch(&mut self, _offset: i64, _amount: NonZero<usize>) -> Result<&mut [u8]> {
            unimplemented!()
        }
    }

    #[test]
    fn test_registration() {
        let token = VfsRegistration::new(DummyVfs)
            .make_default()
            .max_pathlen(16)
            .register("dummy")
            .unwrap();

        let default_vfs_ptr = unsafe { sqlite3::sqlite3_vfs_find(std::ptr::null()) };
        assert!(!default_vfs_ptr.is_null());
        let vfs_ptr = unsafe { sqlite3::sqlite3_vfs_find(c"dummy".as_ptr()) };

        assert!(!vfs_ptr.is_null());
        assert!(vfs_ptr == default_vfs_ptr);
        assert!(unsafe {
            CStr::from_ptr((*vfs_ptr).zName)
                .to_str()
                .unwrap()
                .eq("dummy")
        });
        assert!(unsafe { (*vfs_ptr).mxPathname } == 16);
        assert!(unsafe { (*vfs_ptr).iVersion } == 2);
        drop(token);

        let vfs_ptr = unsafe { sqlite3::sqlite3_vfs_find(c"dummy".as_ptr()) };
        assert!(vfs_ptr.is_null());
    }

    #[test]
    fn test_base_file_methods() {
        let token = VfsRegistration::new(DummyVfs).register("base").unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = rusqlite::Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
            "base",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 1);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_none());
        assert!(methods.xShmLock.is_none());
        assert!(methods.xShmBarrier.is_none());
        assert!(methods.xShmUnmap.is_none());
        assert!(methods.xFetch.is_none());
        assert!(methods.xUnfetch.is_none());
        drop(token);
    }

    #[test]
    fn test_fetch_file_methods() {
        let token = VfsRegistration::new(DummyVfs)
            .with_fetch()
            .register("fetch")
            .unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = rusqlite::Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
            "fetch",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 3);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_none());
        assert!(methods.xShmLock.is_none());
        assert!(methods.xShmBarrier.is_none());
        assert!(methods.xShmUnmap.is_none());
        assert!(methods.xFetch.is_some());
        assert!(methods.xUnfetch.is_some());
        drop(token);
    }

    #[test]
    fn test_wal_file_methods() {
        let token = VfsRegistration::new(DummyVfs)
            .with_wal()
            .register("wal")
            .unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = rusqlite::Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
            "wal",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 2);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_some());
        assert!(methods.xShmLock.is_some());
        assert!(methods.xShmBarrier.is_some());
        assert!(methods.xShmUnmap.is_some());
        assert!(methods.xFetch.is_none());
        assert!(methods.xUnfetch.is_none());
        drop(token);
    }

    #[test]
    fn test_complete_file_methods() {
        let token = VfsRegistration::new(DummyVfs)
            .with_wal()
            .with_fetch()
            .register("full")
            .unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = rusqlite::Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
            "full",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 3);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_some());
        assert!(methods.xShmLock.is_some());
        assert!(methods.xShmBarrier.is_some());
        assert!(methods.xShmUnmap.is_some());
        assert!(methods.xFetch.is_some());
        assert!(methods.xUnfetch.is_some());
        drop(token);
    }

    #[test]
    fn test_sqlite_code_ok() {
        assert_eq!(SqliteCode::OK.into_rc(), sqlite3::SQLITE_OK);
    }

    #[test]
    fn test_sqlite_code_is_ok() {
        assert!(SqliteCode::OK.is_ok());
        assert!(!SqliteCode::from_rc(1).is_ok());
    }

    #[test]
    fn test_sqlite_code_roundtrip() {
        let rc = 1234;
        assert_eq!(SqliteCode::from_rc(rc).into_rc(), rc);
    }

    #[test]
    fn test_sqlite_error_zero_creation() {
        assert!(SqliteError::from_rc(0).is_none());
    }

    #[test]
    fn test_sqlite_error_roundtrip() {
        let rc = 1234;
        assert_eq!(SqliteError::from_rc(rc).unwrap().into_rc(), rc);
    }
}
