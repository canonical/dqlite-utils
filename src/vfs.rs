use libsqlite3_sys::*;
use rand::RngCore;
use std::{
    error,
    ffi::{CStr, OsStr, c_char, c_int},
    fmt,
    num::NonZero,
    os::{raw::c_void, unix::ffi::OsStrExt},
    result, slice,
    sync::Arc,
    thread,
    time::{Duration, SystemTime},
};

#[derive(Debug)]
pub struct SQLiteCode(c_int);

impl From<c_int> for SQLiteCode {
    fn from(code: c_int) -> Self {
        SQLiteCode(code)
    }
}

impl Into<c_int> for SQLiteCode {
    fn into(self) -> c_int {
        self.0
    }
}

impl fmt::Display for SQLiteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", code_to_str(self.0), self.0,)
    }
}

#[derive(Debug)]
pub struct SQLiteError(NonZero<c_int>);

impl fmt::Display for SQLiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SQLiteCode(self.0.get()).fmt(f)
    }
}

impl error::Error for SQLiteError {
    fn description(&self) -> &str {
        code_to_str(self.0.get())
    }
}

impl Into<c_int> for SQLiteError {
    fn into(self) -> c_int {
        self.0.get()
    }
}

pub type Result<T> = result::Result<T, SQLiteError>;

impl From<SQLiteCode> for Result<()> {
    fn from(code: SQLiteCode) -> Self {
        if code.0 == rusqlite::ffi::SQLITE_OK {
            Ok(())
        } else {
            Err(SQLiteError(NonZero::new(code.0).unwrap()))
        }
    }
}

trait AsCode {
    fn as_code(self) -> SQLiteCode;
}

impl AsCode for Result<()> {
    fn as_code(self) -> SQLiteCode {
        match self {
            Ok(_) => SQLiteCode(SQLITE_OK),
            Err(e) => SQLiteCode(e.0.get()),
        }
    }
}

trait AsOutput<T> {
    fn as_output(self, out: &mut impl From<T>) -> SQLiteCode;
}

impl<T> AsOutput<T> for Result<T> {
    fn as_output(self, out: &mut impl From<T>) -> SQLiteCode {
        match self {
            Ok(value) => {
                *out = value.into();
                SQLiteCode(SQLITE_OK)
            }
            Err(e) => SQLiteCode(e.0.get()),
        }
    }
}

pub struct OpenFlags {
    bits: c_int,
}

impl OpenFlags {
    fn new(bits: c_int) -> Self {
        let open_flags = Self { bits };
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

    pub fn file_type(&self) -> FileType {
        match self.bits & 0x0FFF00 {
            SQLITE_OPEN_MAIN_DB => FileType::MainDb,
            SQLITE_OPEN_MAIN_JOURNAL => FileType::MainJournal,
            SQLITE_OPEN_TEMP_DB => FileType::TempDb,
            SQLITE_OPEN_TEMP_JOURNAL => FileType::TempJournal,
            SQLITE_OPEN_TRANSIENT_DB => FileType::TransientDb,
            SQLITE_OPEN_SUBJOURNAL => FileType::Subjournal,
            SQLITE_OPEN_SUPER_JOURNAL => FileType::SuperJournal,
            SQLITE_OPEN_WAL => FileType::Wal,
            _ => {
                debug_assert!(false, "invalid file type");
                unreachable!();
            }
        }
    }

    pub fn create(&self) -> bool {
        (self.bits & SQLITE_OPEN_CREATE) != 0
    }

    pub fn read_only(&self) -> bool {
        (self.bits & SQLITE_OPEN_READONLY) != 0
    }

    pub fn read_write(&self) -> bool {
        (self.bits & SQLITE_OPEN_READWRITE) != 0
    }

    pub fn delete_on_close(&self) -> bool {
        (self.bits & SQLITE_OPEN_DELETEONCLOSE) != 0
    }

    pub fn exclusive(&self) -> bool {
        (self.bits & SQLITE_OPEN_EXCLUSIVE) != 0
    }

    pub fn autoproxy(&self) -> bool {
        (self.bits & SQLITE_OPEN_AUTOPROXY) != 0
    }
}

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

pub enum AccessFlags {
    Exists,
    Read,
    ReadWrite,
}

impl AccessFlags {
    fn from_raw(flags: c_int) -> Self {
        match flags {
            SQLITE_ACCESS_EXISTS => AccessFlags::Exists,
            SQLITE_ACCESS_READ => AccessFlags::Read,
            SQLITE_ACCESS_READWRITE => AccessFlags::ReadWrite,
            _ => {
                debug_assert!(false, "invalid access flag");
                unreachable!();
            }
        }
    }
}

pub trait Vfs: 'static {
    type File: VfsFile;

    fn open(&self, name: Option<&OsStr>, flags: OpenFlags) -> Result<(Self::File, OpenFlags)>;
    fn delete(&self, name: &OsStr, sync_dir: bool) -> Result<()>;
    fn access(&self, name: &OsStr, flags: AccessFlags) -> Result<bool>;
    fn full_pathname(&self, name: &OsStr, out: &mut [u8]) -> Result<()>;

    fn randomness(&self, out: &mut [u8]) -> Result<()> {
        let mut rng = rand::rng();
        rng.fill_bytes(out);
        Ok(())
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }

    fn current_time(&self) -> Result<SystemTime> {
        Ok(SystemTime::now())
    }

    fn get_last_error(&self) -> SQLiteCode;
}

pub struct SyncOptions {
    /// True for Mac OS X style fullsync, false for Unix style fsync
    pub full: bool,
    /// True to sync only the data of the file and not its inode (fdatasync)
    pub data_only: bool,
}

pub enum LockLevel {
    None,
    Shared,
    Reserved,
    Pending,
    Exclusive,
}

impl LockLevel {
    fn from_raw(level: c_int) -> Option<Self> {
        match level {
            SQLITE_LOCK_NONE => Some(LockLevel::None),
            SQLITE_LOCK_SHARED => Some(LockLevel::Shared),
            SQLITE_LOCK_RESERVED => Some(LockLevel::Reserved),
            SQLITE_LOCK_PENDING => Some(LockLevel::Pending),
            SQLITE_LOCK_EXCLUSIVE => Some(LockLevel::Exclusive),
            _ => None,
        }
    }

    fn to_raw(&self) -> c_int {
        match self {
            LockLevel::None => SQLITE_LOCK_NONE,
            LockLevel::Shared => SQLITE_LOCK_SHARED,
            LockLevel::Reserved => SQLITE_LOCK_RESERVED,
            LockLevel::Pending => SQLITE_LOCK_PENDING,
            LockLevel::Exclusive => SQLITE_LOCK_EXCLUSIVE,
        }
    }
}

pub enum AtomicWrite {
    Never,
    Block {
        _512: bool,
        _1k: bool,
        _2k: bool,
        _4k: bool,
        _8k: bool,
        _16k: bool,
        _32k: bool,
        _64k: bool,
    },
    Always,
}

pub struct IoCap {
    pub write_cap: AtomicWrite,
    pub safe_append: bool,
    pub sequential: bool,
    pub undeletable_when_open: bool,
    pub powersafe_overwrite: bool,
    pub immutable: bool,
    pub batch_atomic: bool,
    pub subpage_read: bool,
}

impl Into<c_int> for IoCap {
    fn into(self) -> c_int {
        let mut flags = 0;

        match self.write_cap {
            AtomicWrite::Never => {}
            AtomicWrite::Block {
                _512,
                _1k,
                _2k,
                _4k,
                _8k,
                _16k,
                _32k,
                _64k,
            } => {
                if _512 {
                    flags |= SQLITE_IOCAP_ATOMIC512;
                }
                if _1k {
                    flags |= SQLITE_IOCAP_ATOMIC1K;
                }
                if _2k {
                    flags |= SQLITE_IOCAP_ATOMIC2K;
                }
                if _4k {
                    flags |= SQLITE_IOCAP_ATOMIC4K;
                }
                if _8k {
                    flags |= SQLITE_IOCAP_ATOMIC8K;
                }
                if _16k {
                    flags |= SQLITE_IOCAP_ATOMIC16K;
                }
                if _32k {
                    flags |= SQLITE_IOCAP_ATOMIC32K;
                }
                if _64k {
                    flags |= SQLITE_IOCAP_ATOMIC64K;
                }
            }
            AtomicWrite::Always => {
                flags |= SQLITE_IOCAP_ATOMIC;
            }
        }
        if self.safe_append {
            flags |= SQLITE_IOCAP_SAFE_APPEND;
        }
        if self.sequential {
            flags |= SQLITE_IOCAP_SEQUENTIAL;
        }
        if self.undeletable_when_open {
            flags |= SQLITE_IOCAP_UNDELETABLE_WHEN_OPEN;
        }
        if self.powersafe_overwrite {
            flags |= SQLITE_IOCAP_POWERSAFE_OVERWRITE;
        }
        if self.immutable {
            flags |= SQLITE_IOCAP_IMMUTABLE;
        }
        if self.batch_atomic {
            flags |= SQLITE_IOCAP_BATCH_ATOMIC;
        }
        if self.subpage_read {
            flags |= SQLITE_IOCAP_SUBPAGE_READ;
        }
        flags
    }
}

pub trait VfsFile: VfsFileControl + 'static {
    fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<()>;
    fn write_at(&mut self, buf: &[u8], offset: u64) -> Result<()>;
    fn truncate(&mut self, size: u64) -> Result<()>;
    fn sync(&mut self, op: SyncOptions) -> Result<()>;
    fn size(&mut self) -> Result<u64>;
    fn lock(&mut self, level: LockLevel) -> Result<()>;
    fn unlock(&mut self, level: LockLevel) -> Result<()>;
    // FIXME: this returns a bool indicating whether there is a reserved lock
    // held by another process. The name is not very clear IMHO, it could be
    // something like `is_write_locked` or similar. But `check_reserved_lock` is
    // the name used by sqlite3, so let's keep it for now.
    fn check_reserved_lock(&mut self) -> Result<bool>;

    fn sector_size(&mut self) -> u32 {
        4096
    }
    fn device_characteristics(&mut self) -> IoCap {
        IoCap {
            write_cap: AtomicWrite::Never,
            safe_append: false,
            sequential: false,
            undeletable_when_open: false,
            powersafe_overwrite: false,
            immutable: false,
            batch_atomic: false,
            subpage_read: false,
        }
    }
}

pub trait VfsFileControl {
    fn lockstate(&mut self) -> LockLevel;

    fn last_errno(&mut self) -> i32;

    fn size_hint(&mut self, _size: i64) -> Result<()> {
        Ok(())
    }

    fn overwrite_hint(&mut self, _size: u64) -> Result<()> {
        Err(SQLiteError(NonZero::new(SQLITE_NOTFOUND).unwrap()))
    }

    fn set_chunk_size(&mut self, _size: u32) -> Result<()> {
        Ok(())
    }

    // FIXME: this can also return a string both in case of error and in case of a result!
    fn pragma(&mut self, _name: &OsStr, _arg: Option<&OsStr>) -> Result<()> {
        Err(SQLiteError(NonZero::new(SQLITE_NOTFOUND).unwrap()))
    }

    fn set_mmap_size(&mut self, _size: u64) -> Result<()> {
        Ok(())
    }

    fn get_mmap_size(&mut self) -> u64 {
        0
    }

    fn has_moved(&mut self) -> bool {
        false
    }

    fn pre_sync(&mut self, _super_journal: Option<&OsStr>) -> Result<()> {
        Ok(())
    }

    fn commit_phase_two(&mut self) -> Result<()> {
        Ok(())
    }

    fn set_connection(&mut self, _conn: rusqlite::Connection) {}

    fn begin_atomic(&mut self) -> Result<()> {
        Ok(())
    }

    fn commit_atomic(&mut self) -> Result<()> {
        Ok(())
    }

    fn rollback_atomic(&mut self) {}

    fn get_lock_timeout(&mut self) -> Duration {
        Duration::from_millis(0)
    }

    fn set_lock_timeout(&mut self, _timeout: Duration) -> Result<()> {
        Ok(())
    }
}

pub struct VfsRegisterToken<V>(*const VfsStorage<V>);

impl<V> Drop for VfsRegisterToken<V> {
    fn drop(&mut self) {
        let rc = unsafe { sqlite3_vfs_unregister(&(*self.0).base as *const _ as *mut _) };
        if rc != rusqlite::ffi::SQLITE_OK {
            panic!("cannot unregister VFS: {}", rc);
        }

        // Reclaim the storage
        let _ = unsafe { Arc::from_raw(self.0) };
    }
}

// TODO: add options
// FIXME: the lifetime of the name is probably too restrictive. Maybe we can allocate a bit here?
#[allow(unused)]
pub fn register<T: Vfs>(name: &'static CStr, vfs: T) -> Result<VfsRegisterToken<T>> {
    let storage = Arc::new(VfsStorage {
        base: sqlite3_vfs {
            iVersion: 2,
            szOsFile: std::mem::size_of::<FileStorage<T::File>>() as c_int,
            mxPathname: 512,
            pNext: std::ptr::null_mut(),
            zName: name.as_ptr(),
            pAppData: std::ptr::null_mut(),
            xOpen: Some(x_open::<T>),
            xDelete: Some(x_delete::<T>),
            xAccess: Some(x_access::<T>),
            xFullPathname: Some(x_full_pathname::<T>),
            xDlOpen: Some(x_dlopen),
            xDlError: Some(x_dlerror),
            xDlSym: Some(x_dlsym),
            xDlClose: Some(x_dlclose),
            xRandomness: Some(x_randomness::<T>),
            xSleep: Some(x_sleep::<T>),
            xCurrentTime: Some(x_get_current_time_deprecated),
            xGetLastError: Some(x_get_last_error::<T>),
            xCurrentTimeInt64: Some(x_get_current_time::<T>),
            // TODO: implement these and bump to v3? Maybe not...
            xSetSystemCall: None,
            xGetSystemCall: None,
            xNextSystemCall: None,
        },
        vfs,
    });
    let storage_raw = Arc::into_raw(storage) as *mut VfsStorage<T>;
    let rc = unsafe {
        // FIXME: is this safe? We are assigning a *const that I casted to *mut...
        (*storage_raw).base.pAppData = storage_raw as *mut c_void;
        sqlite3_vfs_register(&mut (*storage_raw).base as *mut _, 1)
    };
    if rc != SQLITE_OK {
        // Make sure not to leak the storage
        let _ = unsafe { Arc::from_raw(storage_raw) };
        return Err(SQLiteError(NonZero::new(rc).unwrap()));
    }

    Ok(VfsRegisterToken(storage_raw))
}

struct VfsStorage<V> {
    base: sqlite3_vfs,
    vfs: V,
}

impl<T> VfsStorage<T> {
    unsafe fn from_raw(ptr: *mut sqlite3_vfs) -> &'static Self {
        unsafe {
            ptr.as_ref()
                .and_then(|vfs| vfs.pAppData.cast::<VfsStorage<T>>().as_ref())
                .expect("cannot get reference to empty vfs storage")
        }
    }
}

#[repr(C)]
struct FileStorage<T: VfsFile> {
    base: sqlite3_file,
    file: Option<T>, // Will be None when closed
}

impl<T: VfsFile> FileStorage<T> {
    const METHOD_TABLE: sqlite3_io_methods = sqlite3_io_methods {
        iVersion: 1,
        xClose: Some(x_file_close::<T>),
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
        xShmMap: None,
        xShmLock: None,
        xShmBarrier: None,
        xShmUnmap: None,
        xFetch: None,
        xUnfetch: None,
    };

    unsafe fn from_raw(ptr: *mut sqlite3_file) -> &'static mut Self {
        unsafe {
            ptr.cast::<FileStorage<T>>()
                .as_mut()
                .expect("cannot get reference to empty file storage")
        }
    }

    fn file(&mut self) -> &mut T {
        self.file
            .as_mut()
            .expect("internal error: file already taken")
    }
}

unsafe extern "C" fn x_open<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    filename: sqlite3_filename,
    out: *mut sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let path = if filename.is_null() {
        if (flags & SQLITE_OPEN_DELETEONCLOSE) == 0 {
            return SQLITE_MISUSE;
        }
        None
    } else {
        Some(OsStr::from_bytes(
            unsafe { CStr::from_ptr(filename) }.to_bytes(),
        ))
    };

    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    match storage.vfs.open(path, OpenFlags::new(flags)) {
        Ok((file, flags)) => {
            unsafe {
                out_flags.write(flags.bits);
                out.cast::<FileStorage<T::File>>().write(FileStorage {
                    base: sqlite3_file {
                        pMethods: &FileStorage::<T::File>::METHOD_TABLE as *const _,
                    },
                    file: Some(file),
                });
            }
            SQLITE_OK
        }
        Err(e) => e.into(),
    }
}

unsafe extern "C" fn x_delete<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    filename: *const c_char,
    sync: c_int,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = OsStr::from_bytes(unsafe { CStr::from_ptr(filename) }.to_bytes());
    storage.vfs.delete(name, sync != 0).as_code().into()
}

unsafe extern "C" fn x_access<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    filename: *const c_char,
    flags: c_int, // TODO: use bitflags
    outcome: *mut c_int,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = OsStr::from_bytes(unsafe { CStr::from_ptr(filename) }.to_bytes());

    storage
        .vfs
        .access(name, AccessFlags::from_raw(flags))
        .as_output(unsafe { outcome.as_mut().unwrap() })
        .into()
}

unsafe extern "C" fn x_full_pathname<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    name: *const c_char,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = OsStr::from_bytes(unsafe { CStr::from_ptr(name) }.to_bytes());
    let out_slice = unsafe {
        slice::from_raw_parts_mut(
            out as *mut u8,
            std::mem::size_of::<c_char>() * n_out as usize,
        )
    };
    storage.vfs.full_pathname(name, out_slice).as_code().into()
}

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
            sqlite3_snprintf(n, out, b"%s\0".as_ptr() as *const c_char, err);
            return;
        }
    }
}

// FIXME: the return type of this function is wrong:
//  - either it should be a pointer to a function with generic signature in C like void(*)()
//  - or it should be the only actual use this function is used for:
//      unsafe extern "C" fn(*mut sqlite3, *mut *mut char, *const sqlite3_api_routines) -> c_int.
unsafe extern "C" fn x_dlsym(
    _: *mut sqlite3_vfs,
    p: *mut c_void,
    sym: *const c_char,
) -> Option<unsafe extern "C" fn(*mut sqlite3_vfs, *mut c_void, *const i8)> {
    Some(unsafe {
        *((&dlsym(p, sym) as *const *mut _)
            as *const unsafe extern "C" fn(*mut sqlite3_vfs, *mut c_void, *const i8))
    })
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
        .randomness(unsafe { slice::from_raw_parts_mut(out as *mut u8, n_out as usize) })
        .as_code()
        .into()
}

unsafe extern "C" fn x_sleep<T: Vfs>(vfs: *mut sqlite3_vfs, microseconds: c_int) -> c_int {
    if microseconds > 0 {
        let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
        storage
            .vfs
            .sleep(Duration::from_micros(microseconds as u64));
    }
    microseconds
}

unsafe extern "C" fn x_get_current_time_deprecated(_: *mut sqlite3_vfs, _: *mut f64) -> c_int {
    debug_assert!(false, "deprecated function called");
    SQLITE_MISUSE
}

unsafe extern "C" fn x_get_current_time<T: Vfs>(vfs: *mut sqlite3_vfs, out: *mut i64) -> c_int {
    const UNIX_EPOCH: i64 = 24405875i64 * 8640000i64;

    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage
        .vfs
        .current_time()
        .map(|time| {
            time.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + UNIX_EPOCH
        })
        .as_output(unsafe { out.as_mut().unwrap() })
        .into()
}

unsafe extern "C" fn x_get_last_error<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    _: c_int,
    _: *mut c_char,
) -> i32 {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage.vfs.get_last_error().into()
}

unsafe extern "C" fn x_file_close<T: VfsFile>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    std::mem::take(&mut storage.file);
    SQLITE_OK
}

unsafe extern "C" fn x_read<T: VfsFile>(
    file: *mut sqlite3_file,
    data: *mut c_void,
    amount: i32,
    offset: i64,
) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.read_at(
        unsafe { slice::from_raw_parts_mut(data as *mut u8, amount as usize) },
        offset as u64,
    )
    .as_code()
    .into()
}

unsafe extern "C" fn x_write<T: VfsFile>(
    file: *mut sqlite3_file,
    data: *const c_void,
    amount: i32,
    offset: i64,
) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.write_at(
        unsafe { slice::from_raw_parts(data as *const u8, amount as usize) },
        offset as u64,
    )
    .as_code()
    .into()
}

unsafe extern "C" fn x_truncate<T: VfsFile>(file: *mut sqlite3_file, size: i64) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.truncate(size as u64).as_code().into()
}

unsafe extern "C" fn x_sync<T: VfsFile>(file: *mut sqlite3_file, flags: c_int) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let options = SyncOptions {
        full: (flags & SQLITE_SYNC_FULL) != 0,
        data_only: (flags & SQLITE_SYNC_DATAONLY) != 0,
    };
    file.sync(options).as_code().into()
}

unsafe extern "C" fn x_file_size<T: VfsFile>(
    file: *mut sqlite3_file,
    out: *mut sqlite3_int64,
) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.size()
        .map(|size| size as i64)
        .as_output(unsafe { out.as_mut().unwrap() })
        .into()
}

unsafe extern "C" fn x_lock<T: VfsFile>(file: *mut sqlite3_file, level: c_int) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_level = match LockLevel::from_raw(level) {
        Some(level) => level,
        None => return SQLITE_MISUSE,
    };
    file.lock(lock_level).as_code().into()
}

unsafe extern "C" fn x_unlock<T: VfsFile>(file: *mut sqlite3_file, level: c_int) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_level = match level {
        SQLITE_LOCK_NONE => LockLevel::None,
        SQLITE_LOCK_SHARED => LockLevel::Shared,
        _ => return SQLITE_MISUSE,
    };
    file.unlock(lock_level).as_code().into()
}

unsafe extern "C" fn x_check_reserved_lock<T: VfsFile>(
    file: *mut sqlite3_file,
    out: *mut c_int,
) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.check_reserved_lock()
        .as_output(unsafe { out.as_mut().unwrap() })
        .into()
}

unsafe extern "C" fn x_file_control<T: VfsFile>(
    _file: *mut sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(_file) };
    let file = storage.file();
    match op {
        SQLITE_FCNTL_LOCKSTATE => {
            let level = file.lockstate();
            unsafe { arg.cast::<c_int>().write(level.to_raw()) };
            SQLITE_OK
        }
        SQLITE_FCNTL_LAST_ERRNO => {
            let errno = file.last_errno();
            unsafe { arg.cast::<c_int>().write(errno) };
            SQLITE_OK
        }
        SQLITE_FCNTL_SIZE_HINT => {
            let size = unsafe { arg.cast::<i64>().read() };
            file.size_hint(size).as_code().into()
        }
        SQLITE_FCNTL_CHUNK_SIZE => {
            let size = unsafe { arg.cast::<c_int>().read() } as u32;
            file.set_chunk_size(size).as_code().into()
        }
        SQLITE_FCNTL_OVERWRITE => {
            let size = unsafe { arg.cast::<sqlite3_int64>().read() } as u64;
            file.overwrite_hint(size).as_code().into()
        }

        SQLITE_FCNTL_VFSNAME => todo!(), // How? We don't have access to VFS name here unless we tie a File to its VFS

        SQLITE_FCNTL_PRAGMA => {
            let args = unsafe { arg.cast::<*mut c_char>() };
            let name = OsStr::from_bytes(
                unsafe { CStr::from_ptr(*args.add(1)) }.to_bytes(),
            );
            let arg_raw = unsafe { *args.add(2) };
            let arg = if arg_raw.is_null() {
                None
            } else {
                Some(OsStr::from_bytes(
                    unsafe { CStr::from_ptr(arg_raw) }.to_bytes(),
                ))
            };
            file.pragma(name, arg).as_code().into()
        }
        SQLITE_FCNTL_MMAP_SIZE => {
            let size = unsafe { arg.cast::<sqlite3_int64>().as_mut() }.unwrap();
            let new_size = *size;
            let old_size = file.get_mmap_size();
            *size = old_size as i64;
            if new_size >= 0 {
                file.set_mmap_size(new_size as u64).as_code().into()
            } else {
                SQLITE_OK
            }
        }
        SQLITE_FCNTL_HAS_MOVED => {
            unsafe { arg.cast::<c_int>().write(file.has_moved() as c_int) };
            SQLITE_OK
        }
        SQLITE_FCNTL_SYNC => {
            let super_journal_raw = arg.cast::<c_char>();
            let super_journal = if super_journal_raw.is_null() {
                None
            } else {
                Some(OsStr::from_bytes(
                    unsafe { CStr::from_ptr(super_journal_raw) }.to_bytes(),
                ))
            };

            file.pre_sync(super_journal).as_code().into()
        }
        SQLITE_FCNTL_COMMIT_PHASETWO => file.commit_phase_two().as_code().into(),
        SQLITE_FCNTL_PDB => {
            file.set_connection(unsafe { rusqlite::Connection::from_handle(arg.cast()) }.unwrap());
            SQLITE_OK
        }
        // FIXME: it would be nice to have a struct representing atomic write options.
        SQLITE_FCNTL_BEGIN_ATOMIC_WRITE => file.begin_atomic().as_code().into(),
        SQLITE_FCNTL_COMMIT_ATOMIC_WRITE => file.commit_atomic().as_code().into(),
        SQLITE_FCNTL_ROLLBACK_ATOMIC_WRITE => {
            file.rollback_atomic();
            SQLITE_OK
        }

        SQLITE_FCNTL_LOCK_TIMEOUT => {
            let timeout = unsafe { arg.cast::<i32>().as_mut() }.unwrap();
            let new_timeout = Duration::from_millis(*timeout as u64);
            let old_timeout = file.get_lock_timeout();
            *timeout = old_timeout.as_millis() as i32;
            file.set_lock_timeout(new_timeout).as_code().into()
        }

        // Not sure what to do with these
        SQLITE_FCNTL_BUSYHANDLER
        | SQLITE_FCNTL_TEMPFILENAME
        | SQLITE_FCNTL_NULL_IO
        | SQLITE_FCNTL_SIZE_LIMIT // FIXME: do we want to support `sqlite3_[de]serialize`?
        | SQLITE_FCNTL_EXTERNAL_READER => SQLITE_NOTFOUND,

        // Not available as there can't be a wal with v1 io methods
        SQLITE_FCNTL_PERSIST_WAL
        | SQLITE_FCNTL_WAL_BLOCK
        | SQLITE_FCNTL_BLOCK_ON_CONNECT
        | SQLITE_FCNTL_CKPT_DONE
        | SQLITE_FCNTL_CKPT_START => SQLITE_MISUSE,

        // Not available as they are specific VFS detail
        SQLITE_FCNTL_GET_LOCKPROXYFILE
        | SQLITE_FCNTL_SET_LOCKPROXYFILE
        | SQLITE_FCNTL_POWERSAFE_OVERWRITE
        | SQLITE_FCNTL_WIN32_GET_HANDLE
        | SQLITE_FCNTL_WIN32_SET_HANDLE
        | SQLITE_FCNTL_WIN32_AV_RETRY
        | SQLITE_FCNTL_ZIPVFS
        | SQLITE_FCNTL_RBU
        | SQLITE_FCNTL_CKSM_FILE => SQLITE_NOTFOUND,

        // Should be implemented by SQLite core
        SQLITE_FCNTL_DATA_VERSION
        | SQLITE_FCNTL_RESERVE_BYTES
        | SQLITE_FCNTL_FILE_POINTER
        | SQLITE_FCNTL_VFS_POINTER
        | SQLITE_FCNTL_SYNC_OMITTED
        | SQLITE_FCNTL_RESET_CACHE => SQLITE_MISUSE,

        // Not necessary.
        SQLITE_FCNTL_TRACE => SQLITE_OK,

        // Newer codes that we don't need to handle yet
        fcntl if fcntl <= 100 => SQLITE_NOTFOUND,

        // FIXME: how do we allow extensions to handle custom opcodes?
        _ => SQLITE_NOTFOUND,
    }
}

unsafe extern "C" fn x_sector_size<T: VfsFile>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.sector_size() as c_int
}

unsafe extern "C" fn x_device_characteristics<T: VfsFile>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { FileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.device_characteristics().into()
}
