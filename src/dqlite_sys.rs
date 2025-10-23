use std::{
    error::Error,
    ffi::{CStr, CString, OsStr, OsString, c_int, c_void},
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
    RAFT_ERRMSG_BUF_SIZE, command__decode, configurationAdd, configurationDecode,
    configurationInit, raft_buffer, raft_command_type, raft_configuration, raft_entry_type,
    raft_free, raft_result, raft_role, raft_server, raft_snapshot, uvMetadata, uvSegmentInfo,
    uvSnapshotInfo, uvSnapshotLoadMeta,
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
    const EMPTY: Self = Self(ptr::null_mut());

    unsafe fn new(ptr: *mut T) -> Self {
        Self(ptr)
    }

    fn as_ptr(&self) -> *const T {
        self.0
    }

    fn as_mut_ptr(&mut self) -> *mut T {
        self.0
    }

    unsafe fn as_mut_ref(&mut self) -> &mut *mut T {
        &mut self.0
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
    pub fn open(dir: &Path) -> Result<Self> {
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
            .map(|s| DqliteSnapshot::load_internal(&cdir, s))
            .collect::<Result<_>>()?;

        let segments: Vec<_> = unsafe { segments.as_slice(n_segments) }
            .iter()
            .map(|s| DqliteSegment::open(&dir, s))
            .collect::<Result<_>>()?;

        let start_index = segments
            .first()
            .and_then(|s| match &s {
                DqliteSegment::Closed { indexes, .. } => Some(*indexes.start()),
                DqliteSegment::Open { .. } => snapshots.first().map(|s| s.index + 1),
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
    pub term: u64,
    pub index: u64,
    pub timestamp: SystemTime,
    pub configuration: RaftConfiguration,

    file: File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RaftConfiguration {
    pub servers: Vec<RaftServer>,
}

impl RaftConfiguration {
    fn new(configuration: &raft_configuration) -> Result<Self> {
        let mut servers = Vec::with_capacity(configuration.n as usize);
        let raw_servers =
            unsafe { std::slice::from_raw_parts(configuration.servers, configuration.n as usize) };
        for server in raw_servers {
            servers.push(RaftServer::new(server)?);
        }
        Ok(Self { servers })
    }
}

impl TryInto<raft_configuration> for &RaftConfiguration {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<raft_configuration, Self::Error> {
        let mut c = raft_configuration::default();
        unsafe { configurationInit(&mut c) };

        for server in self.servers.iter() {
            let rc = unsafe {
                configurationAdd(
                    &mut c,
                    server.id,
                    CString::new(server.address.as_str()).unwrap().as_ptr(),
                    match server.role {
                        RaftRole::Standby => raft_role::RAFT_STANDBY,
                        RaftRole::Voter => raft_role::RAFT_VOTER,
                        RaftRole::Spare => raft_role::RAFT_SPARE,
                    } as _,
                )
            };
            if rc != raft_result::OK {
                return Err(anyhow!("failed to add server to configuration"));
            }
        }
        Ok(c)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RaftServer {
    pub id: u64,
    pub address: String,
    pub role: RaftRole,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RaftRole {
    Standby,
    Voter,
    Spare,
}

impl RaftServer {
    fn new(server: &raft_server) -> Result<Self> {
        let role = match server.role as _ {
            raft_role::RAFT_STANDBY => RaftRole::Standby,
            raft_role::RAFT_VOTER => RaftRole::Voter,
            raft_role::RAFT_SPARE => RaftRole::Spare,
            _ => return Err(anyhow!("invalid role")),
        };
        Ok(Self {
            id: server.id,
            address: unsafe { CStr::from_ptr(server.address).to_str()?.to_owned() },
            role,
        })
    }
}

type LazyCell<T> = std::cell::LazyCell<T, Box<dyn std::ops::FnOnce() -> T>>;

impl DqliteSnapshot {
    pub fn load(dir: impl AsRef<Path>, snapshot: &uvSnapshotInfo) -> Result<Self> {
        let dir = CString::new(dir.as_ref().as_os_str().as_bytes())
            .map_err(|e| anyhow!("failed to convert dir to C string: {e}"))?;
        Self::load_internal(&dir, snapshot)
    }

    fn load_internal(dir: &CStr, snapshot: &uvSnapshotInfo) -> Result<Self> {
        let mut metadata = raft_snapshot::default();
        let mut err = RaftErrorStr::new();
        let rc =
            unsafe { uvSnapshotLoadMeta(dir.as_ptr(), snapshot, &mut metadata, err.as_mut_ptr()) };
        if rc != raft_result::OK {
            return Err(anyhow!("failed to load metadata: {err}"));
        }

        let mut path = PathBuf::from(OsStr::from_bytes(dir.to_bytes())).join(snapshot.filename());
        path.set_extension("");
        let file = File::open(path)?;

        let configuration = RaftConfiguration::new(&metadata.configuration)?;
        let timestamp = UNIX_EPOCH + Duration::from_millis(snapshot.timestamp);
        Ok(Self {
            term: snapshot.term,
            index: snapshot.index,
            timestamp,
            configuration,
            file,
        })
    }
}

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
    pub term: u64,
    pub content: DqliteLogEntryContent,
}

impl DqliteLogEntry {
    pub fn entry_type(&self) -> u16 {
        match &self.content {
            DqliteLogEntryContent::Barrier => raft_entry_type::RAFT_BARRIER as u16,
            DqliteLogEntryContent::Change(_) => raft_entry_type::RAFT_CHANGE as u16,
            _ => raft_entry_type::RAFT_COMMAND as u16,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DqliteLogEntryContent {
    Barrier,
    Change(RaftConfiguration),
    CommandOpen {
        filename: OsString,
    },
    CommandFrames {
        filename: OsString,
        tx_id: u64,
        truncate: u32,
        is_commit: bool,
        frames: Vec<DqliteFrame>,
    },
    CommandUndo {
        tx_id: u64,
    },
    CommandCheckpoint {
        filename: OsString,
    },
}

impl DqliteLogEntryContent {
    fn from(entry_type: u16, data: &[u8]) -> Result<Self> {
        match entry_type as _ {
            raft_entry_type::RAFT_BARRIER => {
                assert!(data.len() == 8 && data.iter().all(|b| *b == 0));
                Ok(Self::Barrier)
            }
            raft_entry_type::RAFT_CHANGE => {
                let mut configuration = raft_configuration::default();
                let rv = unsafe {
                    configurationDecode(
                        &raft_buffer {
                            base: data.as_ptr() as *mut _,
                            len: data.len(),
                        },
                        &mut configuration,
                    )
                };
                if rv != raft_result::OK {
                    return Err(anyhow!("failed to decode change entry: {rv}"));
                }
                Ok(Self::Change(RaftConfiguration::new(&configuration)?))
            }
            raft_entry_type::RAFT_COMMAND => {
                let mut type_: c_int = 0;

                let mut command = RaftPtr::EMPTY;
                let rv = unsafe {
                    command__decode(
                        &raft_buffer {
                            base: data.as_ptr() as *mut _,
                            len: data.len(),
                        },
                        &mut type_,
                        command.as_mut_ref(),
                    )
                };
                if rv != raft_result::OK {
                    return Err(anyhow!("failed to decode command: {rv}"));
                }

                match type_ as _ {
                    raft_command_type::COMMAND_OPEN => {
                        let command = command.as_mut_ptr() as *mut bindings::command_open;
                        let filename = OsStr::from_bytes(
                            unsafe { CStr::from_ptr((*command).filename) }.to_bytes(),
                        );
                        Ok(DqliteLogEntryContent::CommandOpen {
                            filename: filename.to_owned(),
                        })
                    }
                    raft_command_type::COMMAND_UNDO => {
                        let command = command.as_mut_ptr() as *mut bindings::command_undo;
                        Ok(DqliteLogEntryContent::CommandUndo {
                            tx_id: unsafe { (*command).tx_id },
                        })
                    }
                    raft_command_type::COMMAND_CHECKPOINT => {
                        let command = command.as_mut_ptr() as *mut bindings::command_checkpoint;
                        let filename = OsStr::from_bytes(
                            unsafe { CStr::from_ptr((*command).filename) }.to_bytes(),
                        );
                        Ok(DqliteLogEntryContent::CommandCheckpoint {
                            filename: filename.to_owned(),
                        })
                    }
                    raft_command_type::COMMAND_FRAMES => {
                        let command = command.as_mut_ptr() as *mut bindings::command_frames;
                        assert!(unsafe { (*command).frames.n_pages > 0 });
                        assert!(unsafe { (*command).__unused1__ == 0 });
                        assert!(unsafe { (*command).__unused2__ == 0 });
                        let filename = OsStr::from_bytes(
                            unsafe { CStr::from_ptr((*command).filename) }.to_bytes(),
                        );

                        let page_size = unsafe { (*command).frames.page_size } as usize;
                        let pages_count = unsafe { (*command).frames.n_pages } as isize;

                        let mut frames = Vec::with_capacity(pages_count as usize);
                        for i in 0..pages_count {
                            let page_number = unsafe { *(*command).frames.page_numbers.offset(i) };
                            let page = unsafe { *(*command).frames.pages.offset(i) } as *const u8;
                            frames.push(DqliteFrame {
                                page_number: page_number,
                                data: unsafe { std::slice::from_raw_parts(page, page_size) }
                                    .to_vec(),
                            });
                        }

                        Ok(DqliteLogEntryContent::CommandFrames {
                            filename: filename.to_owned(),
                            tx_id: unsafe { (*command).tx_id },
                            truncate: unsafe { (*command).truncate },
                            is_commit: unsafe { (*command).is_commit > 0 },
                            frames,
                        })
                    }
                    _ => panic!("unknown command type: {type_}"),
                }
            }
            _ => panic!("unknown entry type: {entry_type}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DqliteFrame {
    page_number: u64,
    data: Vec<u8>,
}

impl DqliteSegment {
    pub fn open(dir: &Path, segment: &uvSegmentInfo) -> Result<Self> {
        let path = dir.join(segment.filename());
        // It is important to open the file here, as soon as possible,
        // so that in case dqlite is running and decides to remove or
        // rename a segment file then we can still load the entries.
        let file = File::open(path)?;
        if segment.is_open {
            Ok(Self::new_open(file, unsafe { segment.info.open.counter }))
        } else {
            let closed = unsafe { segment.info.closed };
            Ok(Self::new_closed(
                file,
                closed.first_index..=closed.end_index,
            ))
        }
    }

    pub fn new_open(file: File, counter: u64) -> Self {
        let content = LazyCell::new(Box::new(move || Self::load_segment_file(file)));
        Self::Open { counter, content }
    }

    pub fn new_closed(file: File, indexes: RangeInclusive<u64>) -> Self {
        let content = LazyCell::new(Box::new(move || Self::load_segment_file(file)));
        Self::Closed { indexes, content }
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
                    content: DqliteLogEntryContent::from(entry.type_, unsafe {
                        entry.buf.as_bytes()
                    })?,
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
    use std::str::FromStr;
    use std::time;

    use super::bindings::{
        command__encode, command_checkpoint, command_frames, command_open, command_undo,
        configurationAdd, configurationEncode, configurationInit, encodeSnapshotHeader,
        formatSnapshotMetaHeader, frames_t, raft_entry, uv_buf_t, uvSegmentBuffer,
        uvSegmentBufferAppend, uvSegmentBufferFinalize, uvSegmentBufferFormat, uvSegmentBufferInit,
    };

    use super::*;

    struct DqliteSegmentBuilderEntry {
        term: u64,
        entry_type: u16,
        data: Vec<u8>,
    }

    impl TryFrom<&DqliteLogEntry> for DqliteSegmentBuilderEntry {
        type Error = anyhow::Error;

        fn try_from(entry: &DqliteLogEntry) -> Result<Self> {
            unsafe fn encode_command(
                command_type: c_int,
                command: *const c_void,
            ) -> Result<Vec<u8>> {
                let mut buf = raft_buffer::default();
                let rc = unsafe { command__encode(command_type, command, &mut buf) };
                if rc != raft_result::OK {
                    return Err(anyhow!("failed to encode command: {rc}"));
                }
                let data = unsafe { buf.as_bytes().to_vec() };
                unsafe { raft_free(buf.base) };
                Ok(data)
            }

            let data: Vec<_> = match &entry.content {
                DqliteLogEntryContent::Barrier => vec![0u8; 8],
                DqliteLogEntryContent::Change(configuration) => {
                    let configuration = configuration.try_into()?;
                    let mut buf = raft_buffer::default();
                    let rc = unsafe { configurationEncode(&configuration, &mut buf) };
                    if rc != raft_result::OK {
                        return Err(anyhow!("failed to encode configuration: {rc}"));
                    }
                    let data = unsafe { buf.as_bytes().to_vec() };
                    unsafe { raft_free(buf.base) };
                    data
                }
                DqliteLogEntryContent::CommandOpen { filename } => unsafe {
                    encode_command(
                        raft_command_type::COMMAND_OPEN as _,
                        &command_open {
                            filename: CString::new(filename.as_bytes()).unwrap().as_ptr()
                                as *const _,
                        } as *const command_open as *const _,
                    )?
                },
                DqliteLogEntryContent::CommandUndo { tx_id } => unsafe {
                    encode_command(
                        raft_command_type::COMMAND_UNDO as _,
                        &command_undo { tx_id: *tx_id } as *const command_undo as *const _,
                    )?
                },
                DqliteLogEntryContent::CommandCheckpoint { filename } => unsafe {
                    encode_command(
                        raft_command_type::COMMAND_CHECKPOINT as _,
                        &command_checkpoint {
                            filename: CString::new(filename.as_bytes()).unwrap().as_ptr()
                                as *const _,
                        } as *const command_checkpoint as *const _,
                    )?
                },
                DqliteLogEntryContent::CommandFrames {
                    filename,
                    tx_id,
                    truncate,
                    is_commit,
                    frames,
                } => {
                    assert!(frames.len() > 0);

                    let page_size = frames[0].data.len();
                    assert!(frames.iter().all(|f| f.data.len() == page_size));

                    let mut page_numbers = Vec::with_capacity(frames.len());
                    let mut pages = Vec::with_capacity(frames.len());

                    for frame in frames {
                        page_numbers.push(frame.page_number);
                        pages.push(frame.data.as_ptr() as *const c_void);
                    }

                    unsafe {
                        encode_command(
                            raft_command_type::COMMAND_FRAMES as _,
                            &command_frames {
                                filename: CString::new(filename.as_bytes()).unwrap().as_ptr()
                                    as *const _,
                                tx_id: *tx_id,
                                truncate: *truncate,
                                is_commit: if *is_commit { 1 } else { 0 },
                                __unused1__: 0,
                                __unused2__: 0,
                                frames: frames_t {
                                    n_pages: frames.len() as u32,
                                    page_size: page_size as u16,
                                    __unused__: 0,
                                    page_numbers: page_numbers.as_ptr() as *mut u64,
                                    pages: pages.as_ptr() as *mut *mut c_void,
                                },
                            } as *const command_frames as *const _,
                        )?
                    }
                }
            };
            Ok(Self {
                term: entry.term,
                entry_type: entry.entry_type(),
                data,
            })
        }
    }

    struct DqliteSegmentBuilder(Vec<Vec<DqliteSegmentBuilderEntry>>);

    impl DqliteSegmentBuilder {
        fn new() -> Self {
            Self(Vec::new())
        }

        /// Adds a single batch containing entries to the segment.
        fn add_batch(mut self, entries: &[DqliteLogEntry]) -> Self {
            self.0.push(
                entries
                    .iter()
                    .map(|e| e.try_into())
                    .collect::<Result<_>>()
                    .expect("cannot serialize log entry"),
            );
            self
        }

        /// Adds entries to the segment, using one batch each.
        fn add_entries(mut self, entries: &[DqliteLogEntry]) -> Self {
            for entry in entries {
                self.0
                    .push(vec![entry.try_into().expect("cannot serialize log entry")]);
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
                        // FIXME how to do this? Probably need to raft_malloc here.
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

    struct DqliteSnapshotBuilder {
        term: u64,
        index: u64,
        timestamp: SystemTime,
        configuration: RaftConfiguration,
    }

    impl DqliteSnapshotBuilder {
        fn new(
            term: u64,
            index: u64,
            timestamp: SystemTime,
            configuration: RaftConfiguration,
        ) -> Self {
            Self {
                term,
                index,
                timestamp,
                configuration,
            }
        }

        // TODO: add method to add databases in the snapshot. For now the snapshot will be empty (data-wise).
        fn write_to(&self, folder: &Path) -> Result<()> {
            let mut path = {
                let term = self.term;
                let index = self.index;
                let timestamp = self.timestamp.duration_since(UNIX_EPOCH)?.as_millis();
                folder.join(format!("snapshot-{term}-{index}-{timestamp}"))
            };

            {
                let mut data = File::create(&path)?;
                let mut header_buf = raft_buffer::default();
                let rc = unsafe { encodeSnapshotHeader(0, &mut header_buf) };
                if rc != raft_result::OK {
                    return Err(anyhow!("failed to encode snapshot header"));
                }
                let result = data.write_all(unsafe { header_buf.as_bytes() });
                unsafe { raft_free(header_buf.base) };
                result?; // Avoid leaking `header_buf` content.
            }

            {
                path.set_extension("meta");
                let mut meta = File::create(path)?;

                let mut config = raft_configuration::default();
                unsafe { configurationInit(&mut config) };

                for server in &self.configuration.servers {
                    let rc = unsafe {
                        configurationAdd(
                            &mut config,
                            server.id,
                            CString::new(server.address.as_str()).unwrap().as_ptr(),
                            match server.role {
                                RaftRole::Standby => raft_role::RAFT_STANDBY,
                                RaftRole::Voter => raft_role::RAFT_VOTER,
                                RaftRole::Spare => raft_role::RAFT_SPARE,
                            } as _,
                        )
                    };
                    if rc != raft_result::OK {
                        return Err(anyhow!("failed to add server to configuration"));
                    }
                }

                let mut config_buf = raft_buffer::default();
                let rc = unsafe { configurationEncode(&config, &mut config_buf) };
                if rc != raft_result::OK {
                    return Err(anyhow!("failed to encode configuration"));
                }

                let mut header = [0u8; 32];
                unsafe {
                    formatSnapshotMetaHeader(header.as_mut_ptr() as *mut _, self.index, &config_buf)
                };
                let result = meta
                    .write_all(header.as_slice())
                    .and_then(|_| meta.write_all(unsafe { config_buf.as_bytes() }));
                unsafe { raft_free(config_buf.base) };
                result?;
            }

            Ok(())
        }
    }

    struct DqliteDirWriter {
        dir: PathBuf,
        term: u64,
        voted_for: u64,
        first_index: u64,
        closed_segments: Vec<DqliteSegmentBuilder>,
        open_segments: Vec<DqliteSegmentBuilder>,
        snapshots: Vec<DqliteSnapshotBuilder>,
    }

    impl DqliteDirWriter {
        fn new(dir: PathBuf, term: u64, voted_for: u64, first_index: u64) -> Self {
            Self {
                dir,
                term,
                voted_for,
                first_index,
                closed_segments: Vec::new(),
                open_segments: Vec::new(),
                snapshots: Vec::new(),
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

        fn add_snapshot(mut self, snapshot: DqliteSnapshotBuilder) -> Self {
            self.snapshots.push(snapshot);
            self
        }

        fn write(&self) -> Result<()> {
            let mut err = RaftErrorStr::new();

            let rc = unsafe {
                bindings::uvMetadataStore(
                    CString::new(self.dir.as_os_str().as_bytes())
                        .unwrap()
                        .as_ptr(),
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

            let mut path = self.dir.clone();
            let mut index = self.first_index;
            for closed_segment in &self.closed_segments {
                let last_index = index + closed_segment.0.len() as u64 - 1;
                path.push(format!("{index:0>16}-{last_index:0>16}"));

                let mut file = File::create(path.as_path())?;
                closed_segment.write_to(&mut file)?;

                path.pop();
                index = last_index + 1;
            }

            let mut index = 0;
            for open_segment in &self.open_segments {
                path.push(format!("open-{}", index));

                let mut file = File::create(path.as_path())?;
                open_segment.write_to(&mut file)?;

                path.pop();
                index += 1;
            }

            for snapshot in &self.snapshots {
                snapshot.write_to(self.dir.as_path())?;
            }

            Ok(())
        }
    }

    #[test]
    fn test_non_dqlite_folder() {
        let dir = tempfile::tempdir().unwrap();
        let err = DqliteDir::open(dir.path()).unwrap_err();

        assert!(err.to_string().contains("not ad dqlite folder"));
    }

    #[test]
    fn test_metadata_only() {
        let dir = tempfile::tempdir().unwrap();
        DqliteDirWriter::new(dir.path().to_path_buf(), 1, 0, 1)
            .write()
            .unwrap();

        let state = DqliteDir::open(dir.path()).unwrap();
        assert_eq!(state.term(), 1);
        assert_eq!(state.voted_for(), 0);
        assert_eq!(state.first_index(), 1);
        assert_eq!(state.snapshots().len(), 0);
        assert_eq!(state.segments().len(), 0);

        drop(dir)
    }

    #[test]
    fn test_segment() {
        let dir = tempfile::tempdir().unwrap();
        let entries = [
            DqliteLogEntry {
                term: 1,
                content: DqliteLogEntryContent::Barrier,
            },
            DqliteLogEntry {
                term: 1,
                content: DqliteLogEntryContent::Change(RaftConfiguration {
                    servers: vec![RaftServer {
                        id: 1,
                        address: "127.0.0.1:8080".to_owned(),
                        role: RaftRole::Voter,
                    }],
                }),
            },
            DqliteLogEntry {
                term: 3,
                content: DqliteLogEntryContent::CommandUndo { tx_id: 1 },
            },
            DqliteLogEntry {
                term: 3,
                content: DqliteLogEntryContent::CommandOpen {
                    filename: OsStr::new("bar").to_owned(),
                },
            },
            DqliteLogEntry {
                term: 3,
                content: DqliteLogEntryContent::CommandFrames {
                    filename: OsStr::new("foo").to_owned(),
                    tx_id: 1,
                    truncate: 0,
                    is_commit: false,
                    frames: vec![
                        DqliteFrame {
                            page_number: 0,
                            data: vec![0u8; 4096],
                        },
                        DqliteFrame {
                            page_number: 1,
                            data: vec![1u8; 4096],
                        },
                    ],
                },
            },
            DqliteLogEntry {
                term: 3,
                content: DqliteLogEntryContent::CommandCheckpoint {
                    filename: OsStr::new("baz").to_owned(),
                },
            },
        ];

        let open_segment_path = dir.path().join("test");
        DqliteSegmentBuilder::new()
            .add_batch(&entries)
            .write_to(File::create(&open_segment_path).as_mut().unwrap())
            .unwrap();

        let open_segment = DqliteSegment::new_open(File::open(&open_segment_path).unwrap(), 0);
        assert!(matches!(open_segment, DqliteSegment::Open { counter, .. } if counter == 0));
        assert_eq!(open_segment.entries().unwrap(), entries);
    }

    #[test]
    fn test_single_closed_segment() {
        let dir = tempfile::tempdir().unwrap();
        let entries = [
            DqliteLogEntry {
                term: 1,
                content: DqliteLogEntryContent::Barrier,
            },
            DqliteLogEntry {
                term: 1,
                content: DqliteLogEntryContent::CommandOpen {
                    filename: OsStr::new("bar").to_owned(),
                },
            },
            DqliteLogEntry {
                term: 2,
                content: DqliteLogEntryContent::CommandCheckpoint {
                    filename: OsStr::new("baz").to_owned(),
                },
            },
            DqliteLogEntry {
                term: 3,
                content: DqliteLogEntryContent::CommandUndo { tx_id: 1 },
            },
        ];
        DqliteDirWriter::new(dir.path().to_path_buf(), 3, 1, 1000)
            .add_closed_segment(DqliteSegmentBuilder::new().add_entries(&entries))
            .write()
            .unwrap();

        let state = DqliteDir::open(dir.path()).unwrap();
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
                content: DqliteLogEntryContent::Barrier,
            },
            DqliteLogEntry {
                term: 1,
                content: DqliteLogEntryContent::CommandOpen {
                    filename: OsStr::new("bar").to_owned(),
                },
            },
            DqliteLogEntry {
                term: 2,
                content: DqliteLogEntryContent::CommandCheckpoint {
                    filename: OsStr::new("baz").to_owned(),
                },
            },
            DqliteLogEntry {
                term: 3,
                content: DqliteLogEntryContent::CommandUndo { tx_id: 1 },
            },
        ];
        DqliteDirWriter::new(dir.path().to_path_buf(), 3, 1, 1)
            .add_open_segment(DqliteSegmentBuilder::new().add_entries(&entries))
            .write()
            .unwrap();

        let state = DqliteDir::open(dir.path()).unwrap();
        assert_eq!(state.term(), 3);
        assert_eq!(state.voted_for(), 1);
        assert_eq!(state.first_index(), 1);
        assert_eq!(state.snapshots().len(), 0);
        assert_eq!(state.segments().len(), 1);

        let segment = &state.segments()[0];

        if let DqliteSegment::Open { counter, .. } = segment {
            assert_eq!(*counter, 0);
        } else {
            panic!("expected open segment");
        }

        assert_eq!(entries, segment.entries().unwrap());

        drop(dir)
    }

    #[test]
    fn test_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let configuration = RaftConfiguration {
            servers: vec![RaftServer {
                id: 1,
                address: "127.0.0.1:8080".to_owned(),
                role: RaftRole::Voter,
            }],
        };
        let timestamp = UNIX_EPOCH
            + Duration::from_millis(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            );
        DqliteDirWriter::new(dir.path().to_path_buf(), 3, 1, 1)
            .add_snapshot(DqliteSnapshotBuilder::new(
                3,
                1,
                timestamp,
                configuration.clone(),
            ))
            .write()
            .unwrap();

        let state = DqliteDir::open(dir.path()).unwrap();

        let snapshots = state.snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].term, 3);
        assert_eq!(snapshots[0].index, 1);
        assert_eq!(snapshots[0].timestamp, timestamp);
        assert_eq!(snapshots[0].configuration, configuration);
    }
}
