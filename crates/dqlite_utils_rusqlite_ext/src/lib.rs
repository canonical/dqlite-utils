use libsqlite3_sys as sqlite3;
use static_assertions::const_assert;
use std::{
    error::Error,
    ffi::{CStr, CString, OsStr, c_int},
    fmt::{self, Display},
    num::NonZero,
    ops::Deref,
    os::unix::ffi::OsStrExt,
};

pub mod config;
pub mod file_control;
pub mod files;

/// Stores a SQLite result code.
#[derive(Copy, Clone, Debug)]
pub struct SqliteCode(c_int);

impl SqliteCode {
    /// Represents success.
    pub const OK: Self = Self(sqlite3::SQLITE_OK);

    /// Creates a new SQLite result code from a raw result code.
    ///
    /// Returns `None` if the passed result code is zero.
    pub fn from_rc(rc: c_int) -> Self {
        SqliteCode(rc)
    }

    /// Returns the raw result code.
    pub fn into_rc(self) -> c_int {
        self.0
    }

    /// Returns whether the code is [`Self::OK`].
    pub fn is_ok(&self) -> bool {
        self.0 == sqlite3::SQLITE_OK
    }
}

impl Display for SqliteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self(code) = self;
        write!(f, "{code} ({})", libsqlite3_sys::code_to_str(*code))
    }
}

/// Stores a SQLite error code.
///
/// This is a non-ok [`SqliteCode`].
#[derive(Copy, Clone, Debug)]
pub struct SqliteError(NonZero<c_int>);

impl SqliteError {
    // Creates a new SQLite error from a raw result code.
    //
    // Returns `None` if the passed result code is zero.
    pub fn from_rc(rc: c_int) -> Option<Self> {
        const_assert!(sqlite3::SQLITE_OK == 0);
        Some(Self(NonZero::new(rc)?))
    }

    // Returns the raw result code of this SQLite error.
    pub fn into_rc(&self) -> c_int {
        self.0.get()
    }
}

impl Display for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SqliteCode::from(*self).fmt(f)
    }
}

impl Error for SqliteError {}

impl From<SqliteError> for SqliteCode {
    fn from(err: SqliteError) -> Self {
        Self(err.0.get())
    }
}

/// A specialised result type for [`rusqlite::vfs::Vfs`] operations.
pub type Result<T, E = SqliteError> = std::result::Result<T, E>;

impl From<SqliteCode> for Result<()> {
    fn from(code: SqliteCode) -> Self {
        if let Some(err) = SqliteError::from_rc(code.0) {
            return Err(err);
        }
        Ok(())
    }
}

enum SmallCString<const MAX_SIZE: usize = 128> {
    CString(CString),
    Stack { len: usize, buf: [u8; MAX_SIZE] },
}

impl<const MAX_SIZE: usize> SmallCString<MAX_SIZE> {
    const MAX_SIZE: usize = MAX_SIZE;
}

impl Deref for SmallCString {
    type Target = CStr;

    fn deref(&self) -> &Self::Target {
        match self {
            SmallCString::CString(cstr) => cstr,
            SmallCString::Stack { len, buf } => {
                let slice = &buf[..*len];
                unsafe { CStr::from_bytes_with_nul_unchecked(slice) }
            }
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
            SmallCString::Stack {
                len: s.len() + 1,
                buf: stack,
            }
        } else {
            SmallCString::CString(CString::new(s).unwrap())
        }
    }
}
