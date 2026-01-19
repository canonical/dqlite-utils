use libsqlite3_sys as sqlite3;
use static_assertions::const_assert;
use std::{
    error::Error,
    ffi::c_int,
    fmt::{self, Display},
    num::NonZero,
};

pub mod config;
pub mod files;
pub mod vfs;

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

/// A specialised result type for [`Vfs`] operations.
pub type Result<T> = std::result::Result<T, SqliteError>;

impl From<SqliteCode> for Result<()> {
    fn from(code: SqliteCode) -> Self {
        if let Some(err) = SqliteError::from_rc(code.0) {
            return Err(err);
        }
        Ok(())
    }
}

/// Extension trait to convert a [`Result`] to a [`SqliteCode`].
trait ToCodeResultExt {
    /// Convert `self` to a [`SqliteCode`].
    ///
    /// If `self` is:
    /// - `Ok(_)`, this function returns [`SqliteCode::OK`]
    /// - `Err(err)`, this function returns `err`.
    fn to_code_result(self) -> SqliteCode;
}

impl ToCodeResultExt for Result<()> {
    fn to_code_result(self) -> SqliteCode {
        match self {
            Ok(_) => SqliteCode::OK,
            Err(e) => SqliteCode(e.0.get()),
        }
    }
}

/// Extension trait to write results to output parameters, returning an appropriate [`SqliteCode`].
trait WriteOutputResultExt<T> {
    /// Converts `self` into the `sqlite`-expected `out` param + return code form.
    ///
    /// If `self` is:
    /// - `Ok(value)`, then `value` is written to `*output` and [`SqliteCode::OK`]
    ///    is returned.
    /// - `Err(err)`, then `*output` is unchanged and `err` is returned.
    fn write_to_output(self, output: &mut impl From<T>) -> SqliteCode;
}

impl<T> WriteOutputResultExt<T> for Result<T> {
    fn write_to_output(self, output: &mut impl From<T>) -> SqliteCode {
        match self {
            Ok(value) => {
                *output = value.into();
                SqliteCode::OK
            }
            Err(e) => SqliteCode(e.0.get()),
        }
    }
}
