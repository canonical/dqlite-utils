use std::{
    error::Error,
    ffi::{CStr, CString},
    fmt::{Debug, Display},
    fs::File,
    ops::Range,
    path::{Path, PathBuf},
    ptr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use crate::dqlite_sys::bindings::uvMetadata;

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
    term: u64,
    voted_for: u64,
}

impl DqliteDir {
    pub fn new(dir: &Path) -> Result<Self> {
        let cdir = CString::new(dir.to_str().unwrap()).unwrap();
        let mut err = RaftError::new();

        let mut metadata = uvMetadata::default();
        let rv = unsafe {
            bindings::uvMetadataLoad(cdir.as_ptr(), &mut metadata as *mut _, err.as_mut_ptr())
        };
        if rv != bindings::RAFT_OK as _ {
            return Err(anyhow!("failed to load metadata: {}", err));
        }

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

        if result != bindings::RAFT_OK as _ {
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

        Ok(Self {
            dir: PathBuf::from(dir),
            snapshots,
            segments,
            term: metadata.term,
            voted_for: metadata.voted_for,
        })
    }

    pub fn snapshots(&self) -> &[DqliteSnapshot] {
        &self.snapshots
    }

    pub fn segments(&self) -> &[DqliteSegment] {
        &self.segments
    }

    pub fn term(&self) -> u64 {
        self.term
    }

    pub fn voted_for(&self) -> u64 {
        self.voted_for
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
pub struct DqliteSegment {
    segment: uvSegmentInfo,
    file: File,
}

impl DqliteSegment {
    pub fn new(dir: &Path, segment: uvSegmentInfo) -> Result<Self> {
        let path = dir.join(segment.filename());

        let file = File::open(path)?;

        Ok(Self { segment, file })
    }

    pub fn indexes(&self) -> Result<Range<u64>> {
        if self.is_open() {
            return Err(anyhow!(
                "cannot get indexes from an open segment: not implemented yet"
            ));
        }

        let closed = unsafe { self.segment.info.closed };
        Ok(closed.first_index..closed.end_index)
    }

    pub fn is_open(&self) -> bool {
        self.segment.is_open
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

        let dqlite = DqliteDir::new(dir).expect("cannot open dqlite dir");

        assert!(dqlite.term() > 0);
        assert!(dqlite.voted_for() == 0);
        assert!(dqlite.snapshots().len() > 0);
        assert!(dqlite.segments().len() > 0);

        let mut min_index = u64::MAX;
        let mut max_index = 0;

        for segment in dqlite.segments() {
            if segment.is_open() {
                continue;
            }

            let Range { start, end } = segment.indexes().expect("cannot get indexes");
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
    }
}
