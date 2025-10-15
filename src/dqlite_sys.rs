use std::{
    ffi::{CStr, CString},
    fmt::Debug,
    fs::File,
    ops::{DerefMut, Range},
    path::{Path, PathBuf},
    ptr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use self::bindings::*;

mod bindings {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    use std::ffi::{CStr, OsStr};
    use std::fmt::{Debug, Display};
    use std::ops::{Deref, DerefMut};
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

    pub struct raft_error([u8; RAFT_ERRMSG_BUF_SIZE as usize]);

    impl raft_error {
        pub fn new() -> Self {
            Self([0u8; RAFT_ERRMSG_BUF_SIZE as usize])
        }

        pub fn as_str(&self) -> &str {
            CStr::from_bytes_until_nul(self.0.as_slice())
                .unwrap()
                .to_str()
                .unwrap()
        }

        pub fn as_mut_ptr<T>(&mut self) -> *mut T {
            self.0.as_mut_ptr() as *mut T
        }
    }

    impl Debug for raft_error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.as_str())
        }
    }

    impl Display for raft_error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.as_str())
        }
    }

    pub struct RaftPtr<T>(*mut T);

    impl<T> RaftPtr<T> {
        pub fn new(ptr: *mut T) -> Self {
            Self(ptr)
        }

        pub fn as_ptr(&self) -> *const T {
            self.0
        }

        pub fn as_mut_ptr(&mut self) -> *mut T {
            self.0
        }
    }

    impl<T> Drop for RaftPtr<T> {
        fn drop(&mut self) {
            unsafe { raft_free(self.0 as *mut _) };
        }
    }

    impl<T> Deref for RaftPtr<T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            unsafe { &*self.0 }
        }
    }

    impl<T> DerefMut for RaftPtr<T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            unsafe { &mut *self.0 }
        }
    }
}

#[derive(Debug)]
pub struct DqliteDir {
    dir: PathBuf,
    snapshots: Vec<DqliteSnapshot>,
    segments: Vec<DqliteSegment>,
}

impl DqliteDir {
    pub fn new(dir: &Path) -> Result<Self> {
        let mut err = raft_error::new();

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

        let snapshots = RaftPtr::new(snapshots);
        let segments = RaftPtr::new(segments);

        let snapshots: Vec<_> =
            unsafe { std::slice::from_raw_parts(snapshots.as_ptr(), n_snapshots) }
                .iter()
                .map(|s| DqliteSnapshot::new(&dir, s))
                .collect::<Result<_>>()?;

        let segments: Vec<_> = unsafe { std::slice::from_raw_parts(segments.as_ptr(), n_segments) }
            .iter()
            .map(|s| DqliteSegment::new(&dir, *s))
            .collect::<Result<_>>()?;

        Ok(Self {
            dir: PathBuf::from(dir),
            snapshots,
            segments,
        })
    }

    pub fn snapshots(&self) -> &[DqliteSnapshot] {
        &self.snapshots
    }

    pub fn segments(&self) -> &[DqliteSegment] {
        &self.segments
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
