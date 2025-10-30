#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(unused)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::error::Error;
use std::ffi::{CStr, OsStr};
use std::fmt::{Debug, Display};
use std::os::unix::ffi::OsStrExt;

impl raft_result {
    pub const OK: Self = Self(raft_result_code::RAFT_OK as _);
    pub const NOMEM: Self = Self(raft_result_code::RAFT_NOMEM as _);
    pub const BADID: Self = Self(raft_result_code::RAFT_BADID as _);
    pub const DUPLICATEID: Self = Self(raft_result_code::RAFT_DUPLICATEID as _);
    pub const DUPLICATEADDRESS: Self = Self(raft_result_code::RAFT_DUPLICATEADDRESS as _);
    pub const BADROLE: Self = Self(raft_result_code::RAFT_BADROLE as _);
    pub const MALFORMED: Self = Self(raft_result_code::RAFT_MALFORMED as _);
    pub const NOTLEADER: Self = Self(raft_result_code::RAFT_NOTLEADER as _);
    pub const LEADERSHIPLOST: Self = Self(raft_result_code::RAFT_LEADERSHIPLOST as _);
    pub const SHUTDOWN: Self = Self(raft_result_code::RAFT_SHUTDOWN as _);
    pub const CANTBOOTSTRAP: Self = Self(raft_result_code::RAFT_CANTBOOTSTRAP as _);
    pub const CANTCHANGE: Self = Self(raft_result_code::RAFT_CANTCHANGE as _);
    pub const CORRUPT: Self = Self(raft_result_code::RAFT_CORRUPT as _);
    pub const CANCELED: Self = Self(raft_result_code::RAFT_CANCELED as _);
    pub const NAMETOOLONG: Self = Self(raft_result_code::RAFT_NAMETOOLONG as _);
    pub const TOOBIG: Self = Self(raft_result_code::RAFT_TOOBIG as _);
    pub const NOCONNECTION: Self = Self(raft_result_code::RAFT_NOCONNECTION as _);
    pub const BUSY: Self = Self(raft_result_code::RAFT_BUSY as _);
    pub const IOERR: Self = Self(raft_result_code::RAFT_IOERR as _);
    pub const NOTFOUND: Self = Self(raft_result_code::RAFT_NOTFOUND as _);
    pub const INVALID: Self = Self(raft_result_code::RAFT_INVALID as _);
    pub const UNAUTHORIZED: Self = Self(raft_result_code::RAFT_UNAUTHORIZED as _);
    pub const NOSPACE: Self = Self(raft_result_code::RAFT_NOSPACE as _);
    pub const TOOMANY: Self = Self(raft_result_code::RAFT_TOOMANY as _);
    pub const ERROR: Self = Self(raft_result_code::RAFT_ERROR as _);
}

impl Error for raft_result {}

impl Debug for raft_result {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = unsafe { CStr::from_ptr(raft_strerror(*self)) }
            .to_str()
            .unwrap();
        write!(f, "{msg:?}")
    }
}

impl Display for raft_result {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = unsafe { CStr::from_ptr(raft_strerror(*self)) }
            .to_str()
            .unwrap();
        write!(f, "{msg}")
    }
}

impl raft_buffer {
    pub unsafe fn as_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.base as *const u8, self.len) }
    }
}

impl uv_buf_t {
    pub unsafe fn as_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.base as *const _, self.len) }
    }
}

impl uvSnapshotInfo {
    pub fn filename(&self) -> &OsStr {
        OsStr::from_bytes(unsafe { CStr::from_ptr(self.filename.as_ptr()).to_bytes() })
    }
}

impl Debug for uvSnapshotInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("uvSnapshotInfo")
            .field("term", &self.term)
            .field("index", &self.index)
            .field("timestamp", &self.timestamp)
            .field("filename", &self.filename())
            .finish()
    }
}

impl uvSegmentInfo {
    pub fn filename(&self) -> &OsStr {
        OsStr::from_bytes(unsafe { CStr::from_ptr(self.filename.as_ptr()).to_bytes() })
    }
}

impl Debug for uvSegmentInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("uvSegmentInfo");
        debug
            .field("is_open", &self.is_open)
            .field("filename", &self.filename());

        if !self.is_open {
            debug
                .field("first_index", unsafe { &self.info.closed.first_index })
                .field("end_index", unsafe { &self.info.closed.end_index });
        } else {
            debug.field("counter", unsafe { &self.info.open.counter });
        }

        debug.finish()
    }
}

impl Drop for uvSegmentBuffer {
    fn drop(&mut self) {
        unsafe { uvSegmentBufferClose(self) };
    }
}

impl Drop for raft_configuration {
    fn drop(&mut self) {
        unsafe { configurationClose(self) };
    }
}

impl Drop for raft_snapshot {
    fn drop(&mut self) {
        unsafe { snapshotClose(self) };
    }
}
