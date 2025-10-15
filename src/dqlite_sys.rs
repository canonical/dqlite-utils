use std::{
    error::Error,
    ffi::{CStr, CString},
    fmt::{Debug, Display},
    fs::File,
    io::Read,
    ops::RangeInclusive,
    path::{Path, PathBuf},
    ptr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use crate::dqlite_sys::bindings::raft_buffer;

use self::bindings::{RAFT_ERRMSG_BUF_SIZE, raft_free, uvSegmentInfo, uvSnapshotInfo};

mod bindings {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    use std::ffi::{CStr, OsStr};
    use std::fmt::Debug;
    use std::os::unix::ffi::OsStrExt;

    impl raft_buffer {
        pub fn to_vec<T: Clone>(&self) -> Vec<T> {
            unsafe { std::slice::from_raw_parts(self.base as *const _, self.len) }.to_vec()
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
}

struct RaftError([u8; RAFT_ERRMSG_BUF_SIZE as usize]);

impl RaftError {
    fn new() -> Self {
        Self([0u8; RAFT_ERRMSG_BUF_SIZE as usize])
    }

    fn as_str(&self) -> &str {
        CStr::from_bytes_until_nul(self.0.as_slice())
            .expect("display malformet error message")
            .to_str()
            .expect("cannot display malformet error message")
    }

    fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.0.as_mut_ptr() as *mut T
    }
}

impl Error for RaftError {}

impl Debug for RaftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Display for RaftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

struct RaftPtr<T>(*mut T);

impl<T> RaftPtr<T> {
    unsafe fn new(ptr: *mut T) -> Self {
        Self(ptr)
    }

    fn as_ptr(&self) -> *const T {
        self.0
    }

    fn as_mut_ptr(&mut self) -> *mut T {
        self.0
    }

    unsafe fn as_slice(&self, len: usize) -> &[T] {
        assert!(len != 0 || !self.0.is_null());
        unsafe { std::slice::from_raw_parts(self.0, len) }
    }

    unsafe fn as_mut_slice(&mut self, len: usize) -> &mut [T] {
        assert!(len != 0 || !self.0.is_null());
        unsafe { std::slice::from_raw_parts_mut(self.0, len) }
    }
}

impl<T> Drop for RaftPtr<T> {
    fn drop(&mut self) {
        unsafe { raft_free(self.0 as *mut _) };
    }
}

#[derive(Debug)]
pub struct DqliteDir {
    dir: PathBuf,
    snapshots: Vec<DqliteSnapshot>,
    segments: Vec<DqliteSegment>,
    start_index: u64,
}

impl DqliteDir {
    pub fn new(dir: &Path) -> Result<Self> {
        let mut err = RaftError::new();

        let mut snapshots = ptr::null_mut();
        let mut n_snapshots = 0usize;

        let mut segments = ptr::null_mut();
        let mut n_segments = 0usize;

        let result = unsafe {
            bindings::UvList(
                CString::new(dir.to_str().unwrap()).unwrap().as_ptr(),
                &mut snapshots,
                &mut n_snapshots,
                &mut segments,
                &mut n_segments,
                err.as_mut_ptr(),
            )
        };

        if result != bindings::RAFT_OK as i32 {
            return Err(anyhow!("failed to list snapshots and segments: {}", err));
        }

        if n_snapshots == 0 && n_segments == 0 {
            return Err(anyhow!("not ad dqlite folder"));
        }
        assert!(n_snapshots == 0 || snapshots != ptr::null_mut());
        assert!(n_segments == 0 || segments != ptr::null_mut());

        let snapshots = unsafe { RaftPtr::new(snapshots) };
        let segments = unsafe { RaftPtr::new(segments) };

        let snapshots: Vec<_> = unsafe { snapshots.as_slice(n_snapshots) }
            .iter()
            .map(|s| DqliteSnapshot::new(&dir, s))
            .collect::<Result<_>>()?;

        let segments: Vec<_> = unsafe { segments.as_slice(n_segments) }
            .iter()
            .map(|s| DqliteSegment::new(&dir, *s))
            .collect::<Result<_>>()?;

        let start_index = segments
            .first()
            .map(|s| match &s {
                DqliteSegment::Closed { indexes, .. } => *indexes.start(),
                DqliteSegment::Open { .. } => snapshots.first().map(|s| s.index()).unwrap_or(1),
            })
            .unwrap_or(1);

        Ok(Self {
            dir: PathBuf::from(dir),
            snapshots,
            segments,
            start_index,
        })
    }

    pub fn snapshots(&self) -> &[DqliteSnapshot] {
        &self.snapshots
    }

    pub fn segments(&self) -> &[DqliteSegment] {
        &self.segments
    }

    pub fn segments_mut(&mut self) -> &mut [DqliteSegment] {
        &mut self.segments
    }
}

#[derive(Debug)]
pub struct DqliteSnapshot {
    snapshot: uvSnapshotInfo,
    file: File,
}

impl DqliteSnapshot {
    pub fn new(dir: &Path, snapshot: &uvSnapshotInfo) -> Result<Self> {
        let path = dir.join(snapshot.filename());

        let file = File::open(path)?;

        Ok(Self {
            snapshot: *snapshot,
            file,
        })
    }

    pub fn term(&self) -> u64 {
        self.snapshot.term
    }

    pub fn index(&self) -> u64 {
        self.snapshot.index
    }

    pub fn timestamp(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.snapshot.timestamp)
    }
}

#[derive(Debug)]
pub enum DqliteSegment {
    Open {
        counter: u64,
        content: DqliteSegmentContent,
    },
    Closed {
        indexes: RangeInclusive<u64>,
        content: DqliteSegmentContent,
    },
}

impl DqliteSegment {
    pub fn load_entries(&mut self) -> Result<&[DqliteLogEntry]> {
        match self {
            DqliteSegment::Open { content, .. } => content.load_entries(),
            DqliteSegment::Closed { content, .. } => content.load_entries(),
        }
    }
}

#[derive(Debug)]
pub enum DqliteSegmentContent {
    Unloaded(File),
    Cached(Vec<DqliteLogEntry>),
}

#[derive(Debug)]
pub struct DqliteLogEntry {
    term: u64,
    /* TODO: save the deserialied entry instead of the raw data. This will also remove the need for the `entry_type` field. */
    entry_type: u16,
    data: Vec<u8>,
}

impl DqliteSegment {
    pub fn new(dir: &Path, segment: uvSegmentInfo) -> Result<Self> {
        let path = dir.join(segment.filename());
        let file = File::open(path)?;

        if segment.is_open {
            Ok(Self::Open {
                counter: unsafe { segment.info.open.counter },
                content: DqliteSegmentContent::Unloaded(file),
            })
        } else {
            let closed = unsafe { segment.info.closed };
            Ok(Self::Closed {
                indexes: closed.first_index..=closed.end_index,
                content: DqliteSegmentContent::Unloaded(file),
            })
        }
    }
}

impl DqliteSegmentContent {
    fn load_segment_file(file: &mut File) -> Result<Vec<DqliteLogEntry>> {
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        if buf.len() == 0 {
            return Ok(Vec::new());
        } else if buf.len() < 8 {
            return Err(anyhow!("invalid segment file"));
        }

        let format = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        if format == 0 {
            if buf.iter().all(|b| *b == 0) {
                return Ok(Vec::new());
            }

            return Err(anyhow!("invalid segment file"));
        } else if format != bindings::UV__DISK_FORMAT as _ {
            return Err(anyhow!("unsupported segment file format"));
        }

        let mut result = Vec::new();
        let mut offset = 8;
        let mut err = RaftError::new();
        let mut last = false;
        while !last {
            let mut entries = ptr::null_mut();
            let mut n_entries = 0;

            let rv = unsafe {
                bindings::uvLoadEntriesBatch(
                    &raft_buffer {
                        base: buf.as_mut_ptr() as *mut _,
                        len: buf.len(),
                    },
                    &mut entries,
                    &mut n_entries,
                    &mut offset,
                    &mut last,
                    err.as_mut_ptr() as *mut _,
                )
            };

            if rv == bindings::RAFT_CORRUPT as _ {
                if buf[offset..].iter().all(|b| *b == 0) {
                    break;
                } else {
                    return Err(anyhow!("corrupt segment file"));
                }
            } else if rv != bindings::RAFT_OK as _ {
                return Err(anyhow!("failed to load segment file: {}", err));
            }

            for i in 0..n_entries {
                let entry = unsafe { &*entries.offset(i as _) };

                result.push(DqliteLogEntry {
                    term: entry.term,
                    entry_type: entry.type_,
                    data: entry.buf.to_vec(),
                });
            }
        }

        return Ok(result);
    }

    pub fn load_entries(&mut self) -> Result<&[DqliteLogEntry]> {
        if let DqliteSegmentContent::Unloaded(file) = self {
            *self = DqliteSegmentContent::Cached(Self::load_segment_file(file)?);
        }

        if let DqliteSegmentContent::Cached(entries) = self {
            return Ok(entries);
        }

        unreachable!();
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn test_non_dqlite_folder() {
        let dir = Path::new(".");
        let err = DqliteDir::new(dir).unwrap_err();

        assert!(err.to_string().contains("not ad dqlite folder"));
    }

    #[test]
    fn test_load_folder() {
        let dir = env::var_os("DQLITE_DATA_DIR");
        let dir = match &dir {
            Some(dir) => Path::new(dir),
            None => return,
        };

        let mut dqlite = DqliteDir::new(dir).expect("cannot open dqlite dir");

        assert!(dqlite.snapshots().len() > 0);
        assert!(dqlite.segments().len() > 0);

        let mut min_index = u64::MAX;
        let mut max_index = 0;

        for segment in dqlite.segments() {
            let indexes = match segment {
                DqliteSegment::Closed { indexes, .. } => indexes,
                DqliteSegment::Open { .. } => continue,
            };
            let start = *indexes.start();
            let end = *indexes.end();

            if start < min_index {
                min_index = start;
            }
            if end > max_index {
                max_index = end;
            }
        }

        assert!(min_index > 0);
        assert!(max_index > min_index);

        for snapshot in dqlite.snapshots() {
            assert!(snapshot.index() >= min_index);
            assert!(snapshot.index() <= max_index);
        }

        for segment in dqlite.segments_mut() {
            match segment {
                DqliteSegment::Closed {
                    content, indexes, ..
                } => {
                    let entries = content.load_entries().expect("cannot load entries");
                    assert!(entries.len() == indexes.count());
                }
                DqliteSegment::Open { content, .. } => {
                    content.load_entries().expect("cannot load entries");
                }
            }
        }
    }
}
