use std::{
    error::Error,
    ffi::{CStr, CString},
    fmt::{Debug, Display},
    fs::File,
    io::Read,
    ops::RangeInclusive,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use self::bindings::{
    RAFT_ERRMSG_BUF_SIZE, raft_buffer, raft_free, raft_result, uvMetadata, uvSegmentInfo,
    uvSnapshotInfo,
};

mod bindings {
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
            write!(
                f,
                "{}",
                unsafe { CStr::from_ptr(raft_strerror(*self)) }
                    .to_str()
                    .unwrap()
            )
        }
    }

    impl Display for raft_result {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "{}",
                unsafe { CStr::from_ptr(raft_strerror(*self)) }
                    .to_str()
                    .unwrap()
            )
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
}

struct RaftErrorStr([u8; RAFT_ERRMSG_BUF_SIZE as usize]);

impl RaftErrorStr {
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

impl Error for RaftErrorStr {}

impl Debug for RaftErrorStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Display for RaftErrorStr {
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
        if len == 0 {
            assert!(self.0.is_null());
            return &[];
        }
        assert!(!self.0.is_null());
        unsafe { std::slice::from_raw_parts(self.0, len) }
    }

    unsafe fn as_mut_slice(&mut self, len: usize) -> &mut [T] {
        if len == 0 {
            assert!(self.0.is_null());
            return &mut [];
        }
        assert!(!self.0.is_null());
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
    first_index: u64,
}

impl DqliteDir {
    pub fn new(dir: &Path) -> Result<Self> {
        let cdir = CString::new(dir.as_os_str().as_bytes()).unwrap();
        let mut err = RaftErrorStr::new();

        let mut metadata = uvMetadata::default();
        let rc = unsafe {
            bindings::uvMetadataLoad(cdir.as_ptr(), &mut metadata as *mut _, err.as_mut_ptr())
        };
        if rc != raft_result::OK {
            return Err(anyhow!("failed to load metadata: {}", err));
        }
        if metadata.version == 0 {
            return Err(anyhow!("not ad dqlite folder"));
        }

        let mut snapshots = ptr::null_mut();
        let mut n_snapshots = 0usize;

        let mut segments = ptr::null_mut();
        let mut n_segments = 0usize;

        let rc = unsafe {
            bindings::UvList(
                CString::new(dir.as_os_str().as_bytes()).unwrap().as_ptr(),
                &mut snapshots,
                &mut n_snapshots,
                &mut segments,
                &mut n_segments,
                err.as_mut_ptr(),
            )
        };
        if rc != raft_result::OK {
            return Err(anyhow!("failed to list snapshots and segments: {}", err));
        }

        assert!(n_snapshots == 0 || snapshots != ptr::null_mut());
        assert!(n_segments == 0 || segments != ptr::null_mut());

        let snapshots = unsafe { RaftPtr::new(snapshots) };
        let segments = unsafe { RaftPtr::new(segments) };

        let snapshots: Vec<_> = unsafe { snapshots.as_slice(n_snapshots) }
            .iter()
            .map(|s| DqliteSnapshot::new(&dir, *s))
            .collect::<Result<_>>()?;

        let segments: Vec<_> = unsafe { segments.as_slice(n_segments) }
            .iter()
            .map(|s| DqliteSegment::new(&dir, *s))
            .collect::<Result<_>>()?;

        let start_index = segments
            .first()
            .and_then(|s| match &s {
                DqliteSegment::Closed { indexes, .. } => Some(*indexes.start()),
                DqliteSegment::Open { .. } => snapshots.first().map(|s| s.index() + 1),
            })
            .unwrap_or(1);

        Ok(Self {
            dir: PathBuf::from(dir),
            snapshots,
            segments,
            term: metadata.term,
            voted_for: metadata.voted_for,
            first_index: start_index,
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

    pub fn first_index(&self) -> u64 {
        self.first_index
    }
}

#[derive(Debug)]
pub struct DqliteSnapshot {
    snapshot: uvSnapshotInfo,
    file: File,
}

impl DqliteSnapshot {
    pub fn new(dir: &Path, snapshot: uvSnapshotInfo) -> Result<Self> {
        let path = dir.join(snapshot.filename());

        let file = File::open(path)?;

        Ok(Self { snapshot, file })
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

type LazyCell<T> = std::cell::LazyCell<T, Box<dyn std::ops::FnOnce() -> T>>;

#[derive(Debug)]
pub enum DqliteSegment {
    Open {
        counter: u64,
        content: LazyCell<Result<Vec<DqliteLogEntry>>>,
    },
    Closed {
        indexes: RangeInclusive<u64>,
        content: LazyCell<Result<Vec<DqliteLogEntry>>>,
    },
}

impl DqliteSegment {
    pub fn entries(&self) -> Result<&[DqliteLogEntry]> {
        let content = match self {
            DqliteSegment::Open { content, .. } => content,
            DqliteSegment::Closed { content, .. } => content,
        };
        Ok(content
            .as_ref()
            .map_err(|err| anyhow!("cannot load entries: {err}"))?
            .as_slice())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DqliteLogEntry {
    term: u64,
    /* TODO: save the deserialied entry instead of the raw data. This will also remove the need for the `entry_type` field. */
    entry_type: u16,
    data: Vec<u8>,
}

impl DqliteSegment {
    pub fn new(dir: &Path, segment: uvSegmentInfo) -> Result<Self> {
        let path = dir.join(segment.filename());
        // It is important to open the file here, as soon as possible,
        // so that in case dqlite is running and decides to remove or
        // rename a segment file then we can still load the entries.
        let file = File::open(path)?;
        let content = LazyCell::new(Box::new(move || Self::load_segment_file(file)));
        if segment.is_open {
            Ok(Self::Open {
                counter: unsafe { segment.info.open.counter },
                content,
            })
        } else {
            let closed = unsafe { segment.info.closed };
            Ok(Self::Closed {
                indexes: closed.first_index..=closed.end_index,
                content,
            })
        }
    }

    fn load_segment_file(mut file: File) -> Result<Vec<DqliteLogEntry>> {
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
        } else if format != bindings::UV__DISK_FORMAT as u64 {
            return Err(anyhow!("unsupported segment file format"));
        }

        let mut ret = Vec::new();
        let mut offset = 8;
        let mut err = RaftErrorStr::new();
        let mut last = false;
        while !last {
            let mut entries = ptr::null_mut();
            let mut n_entries = 0;

            let rc = unsafe {
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
            if rc == raft_result::CORRUPT {
                if buf[offset..].iter().all(|b| *b == 0) {
                    break;
                } else {
                    return Err(anyhow!("corrupt segment file"));
                }
            } else if rc != raft_result::OK {
                return Err(anyhow!("failed to load segment file: {err}"));
            }

            for i in 0..n_entries {
                let entry = unsafe { &*entries.offset(i as _) };

                ret.push(DqliteLogEntry {
                    term: entry.term,
                    entry_type: entry.type_,
                    data: unsafe { entry.buf.as_bytes() }.to_vec(),
                });
            }
        }
        return Ok(ret);
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_uint;
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;

    use super::bindings::{
        raft_entry, uv_buf_t, uvSegmentBuffer, uvSegmentBufferAppend, uvSegmentBufferFinalize,
        uvSegmentBufferFormat, uvSegmentBufferInit,
    };

    use super::*;

    struct DqliteSegmentBuilder(Vec<Vec<DqliteLogEntry>>);

    impl DqliteSegmentBuilder {
        fn new() -> Self {
            Self(Vec::new())
        }

        /// Adds a single batch containing entries to the segment.
        fn add_batch(mut self, entries: Vec<DqliteLogEntry>) -> Self {
            self.0.push(entries);
            self
        }

        /// Adds entries to the segment, using one batch each.
        fn add_entries(mut self, entries: &[DqliteLogEntry]) -> Self {
            for entry in entries {
                self.0.push(vec![entry.clone()]);
            }
            self
        }

        fn write_to(&self, file: &mut File) -> Result<()> {
            let mut buf = uvSegmentBuffer::default();
            unsafe { uvSegmentBufferInit(&mut buf, 4096) };

            let rc = unsafe { uvSegmentBufferFormat(&mut buf) };
            if rc != raft_result::OK {
                return Err(anyhow!("failed to format segment buffer: {}", rc));
            }

            for batch in &self.0 {
                let entries: Vec<_> = batch
                    .iter()
                    .map(|e| raft_entry {
                        term: e.term,
                        type_: e.entry_type,
                        buf: raft_buffer {
                            // Safety: the buffer is only used within this block and it is only ever read from.
                            base: e.data.as_ptr() as *mut _,
                            len: e.data.len(),
                        },
                        ..Default::default()
                    })
                    .collect();
                let rc = unsafe {
                    uvSegmentBufferAppend(&mut buf, entries.as_ptr(), entries.len() as c_uint)
                };
                if rc != raft_result::OK {
                    return Err(anyhow!("failed to append to segment buffer: {}", rc));
                }
            }

            let mut write_buffer = uv_buf_t::default();
            unsafe { uvSegmentBufferFinalize(&mut buf, &mut write_buffer) };
            file.write_all(unsafe { write_buffer.as_bytes() })?;

            Ok(())
        }
    }

    struct DqliteDirWriter {
        term: u64,
        voted_for: u64,
        first_index: u64,
        closed_segments: Vec<DqliteSegmentBuilder>,
        open_segments: Vec<DqliteSegmentBuilder>,
    }

    impl DqliteDirWriter {
        fn new(term: u64, voted_for: u64, first_index: u64) -> Self {
            Self {
                term,
                voted_for,
                first_index,
                closed_segments: Vec::new(),
                open_segments: Vec::new(),
            }
        }

        fn add_closed_segment(mut self, segment: DqliteSegmentBuilder) -> Self {
            assert!(segment.0.len() > 0);
            self.closed_segments.push(segment);
            self
        }

        fn add_open_segment(mut self, segment: DqliteSegmentBuilder) -> Self {
            self.open_segments.push(segment);
            self
        }

        fn build(&self, dir: PathBuf) -> Result<()> {
            let mut err = RaftErrorStr::new();

            let rc = unsafe {
                bindings::uvMetadataStore(
                    CString::new(dir.as_os_str().as_bytes()).unwrap().as_ptr(),
                    &bindings::uvMetadata {
                        version: 1,
                        term: self.term,
                        voted_for: self.voted_for,
                    },
                    err.as_mut_ptr(),
                )
            };
            if rc != raft_result::OK {
                return Err(anyhow!("failed to store metadata: {}", err));
            }

            let mut path = dir;
            let mut index = self.first_index;
            for closed_segment in self.closed_segments.iter() {
                let last_index = index + closed_segment.0.len() as u64 - 1;
                path.push(format!("{index:0>16}-{last_index:0>16}"));

                let mut file = File::create(path.as_path())?;
                closed_segment.write_to(&mut file)?;

                path.pop();
                index = last_index + 1;
            }

            let mut index = 0;
            for open_segment in self.open_segments.iter() {
                path.push(format!("open-{}", index));

                let mut file = File::create(path.as_path())?;
                open_segment.write_to(&mut file)?;

                path.pop();
                index += 1;
            }

            Ok(())
        }
    }

    #[test]
    fn test_non_dqlite_folder() {
        let dir = tempfile::tempdir().unwrap();
        let err = DqliteDir::new(dir.path()).unwrap_err();

        assert!(err.to_string().contains("not ad dqlite folder"));
    }

    #[test]
    fn test_metadata_only() {
        let dir = tempfile::tempdir().unwrap();
        DqliteDirWriter::new(1, 0, 1)
            .build(dir.path().to_path_buf())
            .unwrap();

        let state = DqliteDir::new(dir.path()).unwrap();
        assert_eq!(state.term(), 1);
        assert_eq!(state.voted_for(), 0);
        assert_eq!(state.first_index(), 1);
        assert_eq!(state.snapshots().len(), 0);
        assert_eq!(state.segments().len(), 0);

        drop(dir)
    }

    #[test]
    fn test_single_closed_segment() {
        let dir = tempfile::tempdir().unwrap();
        let entries = [
            DqliteLogEntry {
                term: 1,
                entry_type: 1,
                data: vec![0u8; 1000],
            },
            DqliteLogEntry {
                term: 1,
                entry_type: 2,
                data: vec![1u8; 40],
            },
            DqliteLogEntry {
                term: 2,
                entry_type: 3,
                data: vec![2u8; 128],
            },
        ];
        DqliteDirWriter::new(3, 1, 1000)
            .add_closed_segment(DqliteSegmentBuilder::new().add_entries(&entries))
            .build(dir.path().to_path_buf())
            .unwrap();

        let state = DqliteDir::new(dir.path()).unwrap();
        assert_eq!(state.term(), 3);
        assert_eq!(state.voted_for(), 1);
        assert_eq!(state.first_index(), 1000);
        assert_eq!(state.snapshots().len(), 0);
        assert_eq!(state.segments().len(), 1);

        let segment = &state.segments()[0];

        if let DqliteSegment::Closed { indexes, .. } = segment {
            assert_eq!(*indexes.start(), 1000);
            assert_eq!(indexes.clone().count(), entries.len());
        } else {
            panic!("expected closed segment");
        }

        assert_eq!(entries, segment.entries().unwrap());

        drop(dir)
    }

    #[test]
    fn test_single_open_segment() {
        let dir = tempfile::tempdir().unwrap();
        let entries = [
            DqliteLogEntry {
                term: 1,
                entry_type: 1,
                data: vec![0u8; 1000],
            },
            DqliteLogEntry {
                term: 1,
                entry_type: 2,
                data: vec![1u8; 40],
            },
            DqliteLogEntry {
                term: 2,
                entry_type: 3,
                data: vec![2u8; 128],
            },
        ];
        DqliteDirWriter::new(3, 1, 1)
            .add_open_segment(DqliteSegmentBuilder::new().add_entries(&entries))
            .build(dir.path().to_path_buf())
            .unwrap();

        let state = DqliteDir::new(dir.path()).unwrap();
        assert_eq!(state.term(), 3);
        assert_eq!(state.voted_for(), 1);
        assert_eq!(state.first_index(), 1);
        assert_eq!(state.snapshots().len(), 0);
        assert_eq!(state.segments().len(), 1);

        let segment = &state.segments()[0];

        if let DqliteSegment::Open { counter, .. } = segment {
            assert_eq!(*counter, 0);
        } else {
            panic!("expected closed segment");
        }

        assert_eq!(entries, segment.entries().unwrap());

        drop(dir)
    }
}
