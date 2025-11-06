mod sys;

use std::{
    error::Error,
    ffi::{CStr, CString, OsStr, OsString, c_int, c_uint, c_void},
    fmt::{Debug, Display},
    fs::File,
    io::{Read, Seek, Write},
    ops::RangeInclusive,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use self::sys::{
    RAFT_ERRMSG_BUF_SIZE, command_checkpoint, command_frames, command_open, command_undo, frames_t,
    raft_buffer, raft_command_type, raft_configuration, raft_entry, raft_entry_type, raft_result,
    raft_role, raft_server, raft_snapshot, uv_buf_t, uvMetadata, uvSegmentBuffer, uvSegmentInfo,
    uvSnapshotInfo,
};

#[derive(thiserror::Error)]
#[error("{}", self.as_str())]
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

impl Debug for RaftErrorStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

struct RaftPtr<T>(*mut T);

impl<T> RaftPtr<T> {
    unsafe fn new(ptr: *mut T) -> Self {
        Self(ptr)
    }

    fn null() -> Self {
        Self(ptr::null_mut())
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
        unsafe { sys::raft_free(self.0 as *mut _) };
    }
}

#[derive(Debug)]
pub struct DqliteDir {
    snapshots: Vec<DqliteSnapshot>,
    segments: Vec<DqliteSegment>,
    num_closed_segments: usize,
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
            sys::uvMetadataLoad(cdir.as_ptr(), &mut metadata as *mut _, err.as_mut_ptr())
        };
        if rc != raft_result::OK {
            return Err(anyhow!("failed to load metadata: {err}"));
        }
        if metadata.version == 0 {
            return Err(anyhow!("not ad dqlite folder"));
        }

        let mut snapshots = ptr::null_mut();
        let mut n_snapshots = 0usize;

        let mut segments = ptr::null_mut();
        let mut n_segments = 0usize;

        let rc = unsafe {
            sys::UvList(
                CString::new(dir.as_os_str().as_bytes()).unwrap().as_ptr(),
                &mut snapshots,
                &mut n_snapshots,
                &mut segments,
                &mut n_segments,
                err.as_mut_ptr(),
            )
        };
        if rc != raft_result::OK {
            return Err(anyhow!("failed to list snapshots and segments: {err}"));
        }

        assert!(n_snapshots == 0 || !snapshots.is_null());
        assert!(n_segments == 0 || !segments.is_null());

        let snapshots = unsafe { RaftPtr::new(snapshots) };
        let segments = unsafe { RaftPtr::new(segments) };

        let snapshots: Vec<_> = unsafe { snapshots.as_slice(n_snapshots) }
            .iter()
            .map(|s| DqliteSnapshot::load_internal(&cdir, s))
            .collect::<Result<_>>()?;

        let segments: Vec<_> = unsafe { segments.as_slice(n_segments) }
            .iter()
            .map(|s| DqliteSegment::open(dir, s))
            .collect::<Result<_>>()?;

        let num_open_segments = segments
            .iter()
            .rev()
            .take_while(|s| matches!(s, DqliteSegment::Open { .. }))
            .count();
        let num_closed_segments = segments.len() - num_open_segments;

        let first_index = segments
            .first()
            .and_then(|s| match &s {
                DqliteSegment::Closed { indexes, .. } => Some(*indexes.start()),
                DqliteSegment::Open { .. } => snapshots.first().map(|s| s.index + 1),
            })
            .unwrap_or(1);

        Ok(Self {
            snapshots,
            segments,
            num_closed_segments,
            term: metadata.term,
            voted_for: metadata.voted_for,
            first_index,
        })
    }

    pub fn creator(dir: impl Into<PathBuf>) -> DqliteDirCreator {
        DqliteDirCreator {
            dir: dir.into(),
            term: 1,
            voted_for: 0,
            first_index: 1,
            closed_segments: Vec::new(),
            open_segments: Vec::new(),
            snapshots: Vec::new(),
        }
    }

    pub fn snapshots(&self) -> &[DqliteSnapshot] {
        &self.snapshots
    }

    pub fn segments(&self) -> &[DqliteSegment] {
        &self.segments
    }

    pub fn closed_segments(&self) -> &[DqliteSegment] {
        &self.segments[..self.num_closed_segments]
    }

    pub fn open_segments(&self) -> &[DqliteSegment] {
        &self.segments[self.num_closed_segments..]
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

    fn to_raw(&self) -> Result<raft_configuration> {
        let mut c = raft_configuration::default();
        unsafe { sys::configurationInit(&mut c) };

        for server in self.servers.iter() {
            let address = CString::new(server.address.as_str()).unwrap();
            let role = match server.role {
                RaftRole::Standby => raft_role::RAFT_STANDBY,
                RaftRole::Voter => raft_role::RAFT_VOTER,
                RaftRole::Spare => raft_role::RAFT_SPARE,
            } as _;
            let rc = unsafe { sys::configurationAdd(&mut c, server.id, address.as_ptr(), role) };
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

type DynLazyCell<T> = std::cell::LazyCell<T, Box<dyn FnOnce() -> T>>;

impl DqliteSnapshot {
    pub fn load(dir: impl AsRef<Path>, snapshot: &uvSnapshotInfo) -> Result<Self> {
        let dir = CString::new(dir.as_ref().as_os_str().as_bytes())
            .map_err(|e| anyhow!("failed to convert dir to C string: {e}"))?;
        Self::load_internal(&dir, snapshot)
    }

    fn load_internal(dir: &CStr, snapshot: &uvSnapshotInfo) -> Result<Self> {
        let mut metadata = raft_snapshot::default();
        let mut err = RaftErrorStr::new();
        let rc = unsafe {
            sys::uvSnapshotLoadMeta(dir.as_ptr(), snapshot, &mut metadata, err.as_mut_ptr())
        };
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
        content: Vec<DqliteLogEntry>,
    },
    Closed {
        indexes: RangeInclusive<u64>,
        file: Mutex<File>,
    },
}

impl DqliteSegment {
    pub fn entries(&self) -> Result<Vec<DqliteLogEntry>> {
        match self {
            DqliteSegment::Closed { file, .. } => {
                let mut file = file.lock().unwrap();
                Ok(Self::load_segment_file(&mut file)?)
            }
            DqliteSegment::Open { content, .. } => Ok(content.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DqliteLogEntry {
    pub term: u64,
    pub content: DqliteLogEntryContent,
}

impl DqliteLogEntry {
    pub fn entry_type(&self) -> u16 {
        use DqliteLogEntryContent as Dlec;
        match &self.content {
            Dlec::Barrier => raft_entry_type::RAFT_BARRIER as u16,
            Dlec::Change(_) => raft_entry_type::RAFT_CHANGE as u16,
            Dlec::CommandOpen { .. }
            | Dlec::CommandFrames { .. }
            | Dlec::CommandUndo { .. }
            | Dlec::CommandCheckpoint { .. } => raft_entry_type::RAFT_COMMAND as u16,
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
    fn parse(entry_type: u16, data: &[u8]) -> Result<Self> {
        match entry_type as _ {
            raft_entry_type::RAFT_BARRIER => {
                if data.len() != 8 || !data.iter().all(|b| *b == 0) {
                    return Err(anyhow!("invalid barrier entry"));
                }
                Ok(Self::Barrier)
            }
            raft_entry_type::RAFT_CHANGE => {
                let mut configuration = raft_configuration::default();
                let rv = unsafe {
                    sys::configurationDecode(
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
                let mut ty: c_int = 0;

                let mut command = RaftPtr::null();
                let rv = unsafe {
                    sys::command__decode(
                        &raft_buffer {
                            base: data.as_ptr() as *mut _,
                            len: data.len(),
                        },
                        &mut ty,
                        command.as_mut_ref(),
                    )
                };
                if rv != raft_result::OK {
                    return Err(anyhow!("failed to decode command: {rv}"));
                }

                match ty as _ {
                    raft_command_type::COMMAND_OPEN => {
                        let command = command.as_mut_ptr() as *mut command_open;
                        let filename = OsStr::from_bytes(
                            unsafe { CStr::from_ptr((*command).filename) }.to_bytes(),
                        );
                        Ok(DqliteLogEntryContent::CommandOpen {
                            filename: filename.to_owned(),
                        })
                    }
                    raft_command_type::COMMAND_UNDO => {
                        let command = command.as_mut_ptr() as *mut command_undo;
                        Ok(DqliteLogEntryContent::CommandUndo {
                            tx_id: unsafe { (*command).tx_id },
                        })
                    }
                    raft_command_type::COMMAND_CHECKPOINT => {
                        let command = command.as_mut_ptr() as *mut command_checkpoint;
                        let filename = OsStr::from_bytes(
                            unsafe { CStr::from_ptr((*command).filename) }.to_bytes(),
                        );
                        Ok(DqliteLogEntryContent::CommandCheckpoint {
                            filename: filename.to_owned(),
                        })
                    }
                    raft_command_type::COMMAND_FRAMES => {
                        let command = command.as_mut_ptr() as *mut command_frames;
                        // TODO: add logging for weird cases like n_pages == 0 or unused fields not zero.
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
                                page_number,
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
                    _ => Err(anyhow!("unknown command type: {ty}")),
                }
            }
            _ => Err(anyhow!("unknown entry type: {entry_type}")),
        }
    }

    fn encode(&self) -> Result<Vec<u8>> {
        unsafe fn encode_command(command_type: c_int, command: *const c_void) -> Result<Vec<u8>> {
            let mut buf = raft_buffer::default();
            let rc = unsafe { sys::command__encode(command_type, command, &mut buf) };
            if rc != raft_result::OK {
                return Err(anyhow!("failed to encode command: {rc}"));
            }
            let data = unsafe { buf.as_bytes() }.to_vec();
            unsafe { sys::raft_free(buf.base) };
            Ok(data)
        }

        match self {
            DqliteLogEntryContent::Barrier => Ok(vec![0u8; 8]),
            DqliteLogEntryContent::Change(configuration) => {
                let configuration = configuration.to_raw()?;
                let mut buf = raft_buffer::default();
                let rc = unsafe { sys::configurationEncode(&configuration, &mut buf) };
                if rc != raft_result::OK {
                    return Err(anyhow!("failed to encode configuration: {rc}"));
                }
                let data = unsafe { buf.as_bytes().to_vec() };
                unsafe { sys::raft_free(buf.base) };
                Ok(data)
            }
            DqliteLogEntryContent::CommandOpen { filename } => unsafe {
                Ok(encode_command(
                    raft_command_type::COMMAND_OPEN as _,
                    &command_open {
                        filename: CString::new(filename.as_bytes()).unwrap().as_ptr() as *const _,
                    } as *const command_open as *const _,
                )?)
            },
            DqliteLogEntryContent::CommandUndo { tx_id } => unsafe {
                Ok(encode_command(
                    raft_command_type::COMMAND_UNDO as _,
                    &command_undo { tx_id: *tx_id } as *const command_undo as *const _,
                )?)
            },
            DqliteLogEntryContent::CommandCheckpoint { filename } => unsafe {
                Ok(encode_command(
                    raft_command_type::COMMAND_CHECKPOINT as _,
                    &command_checkpoint {
                        filename: CString::new(filename.as_bytes()).unwrap().as_ptr() as *const _,
                    } as *const command_checkpoint as *const _,
                )?)
            },
            DqliteLogEntryContent::CommandFrames {
                filename,
                tx_id,
                truncate,
                is_commit,
                frames,
            } => {
                assert!(!frames.is_empty());

                let page_size = frames[0].data.len();
                assert!(frames.iter().all(|f| f.data.len() == page_size));

                let mut page_numbers = Vec::with_capacity(frames.len());
                let mut pages = Vec::with_capacity(frames.len());

                for frame in frames {
                    page_numbers.push(frame.page_number);
                    pages.push(frame.data.as_ptr() as *const c_void);
                }

                Ok(unsafe {
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
                })
            }
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
        let mut file = File::open(path)?;
        if segment.is_open {
            let content = Self::load_segment_file(&mut file)?;
            let counter = unsafe { segment.info.open.counter };
            Ok(Self::Open { counter, content })
        } else {
            let closed = unsafe { segment.info.closed };
            let indexes = closed.first_index..=closed.end_index;
            Ok(Self::Closed {
                indexes,
                file: Mutex::new(file),
            })
        }
    }

    fn load_segment_file(file: &mut File) -> Result<Vec<DqliteLogEntry>> {
        let mut buf = Vec::new();

        file.seek(std::io::SeekFrom::Start(0))?;
        file.read_to_end(&mut buf)?;

        if buf.is_empty() {
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
        } else if format != sys::UV__DISK_FORMAT as u64 {
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
                sys::uvLoadEntriesBatch(
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
                    content: DqliteLogEntryContent::parse(entry.type_, unsafe {
                        entry.buf.as_bytes()
                    })?,
                });
            }
        }
        Ok(ret)
    }
}

pub struct DqliteSegmentBuilder(Vec<Vec<DqliteLogEntry>>);

impl DqliteSegmentBuilder {
    fn new() -> Self {
        Self(Vec::new())
    }

    /// Adds a single batch containing entries to the segment.
    pub fn add_batch(mut self, entries: &[DqliteLogEntry]) -> Self {
        self.0.push(entries.to_vec());
        self
    }

    /// Adds entries to the segment, using one batch each.
    pub fn add_entries(mut self, entries: &[DqliteLogEntry]) -> Self {
        for entry in entries {
            self.0.push(vec![entry.clone()]);
        }
        self
    }
}

pub struct DqliteSnapshotBuilder {
    term: u64,
    index: u64,
    timestamp: SystemTime,
    configuration: Option<RaftConfiguration>,
}

impl DqliteSnapshotBuilder {
    fn new(term: u64, index: u64, timestamp: SystemTime) -> Self {
        Self {
            term,
            index,
            timestamp,
            configuration: None,
        }
    }

    pub fn with_term(mut self, term: u64) -> Self {
        self.term = term;
        self
    }

    pub fn with_index(mut self, index: u64) -> Self {
        self.index = index;
        self
    }

    pub fn with_timestamp(mut self, timestamp: SystemTime) -> Self {
        self.timestamp = timestamp;
        self
    }

    pub fn with_configuration(mut self, configuration: RaftConfiguration) -> Self {
        self.configuration = Some(configuration);
        self
    }
}

pub struct DqliteDirCreator {
    dir: PathBuf,
    term: u64,
    voted_for: u64,
    first_index: u64,
    closed_segments: Vec<DqliteSegmentBuilder>,
    open_segments: Vec<DqliteSegmentBuilder>,
    snapshots: Vec<DqliteSnapshotBuilder>,
}

impl DqliteDirCreator {
    pub fn with_term(mut self, term: u64) -> Self {
        self.term = term;
        self
    }

    pub fn with_voted_for(mut self, voted_for: u64) -> Self {
        self.voted_for = voted_for;
        self
    }

    pub fn with_first_index(mut self, first_index: u64) -> Self {
        self.first_index = first_index;
        self
    }

    pub fn with_closed_segment(
        mut self,
        f: impl FnOnce(DqliteSegmentBuilder) -> DqliteSegmentBuilder,
    ) -> Self {
        let segment = f(DqliteSegmentBuilder::new());
        assert!(!segment.0.is_empty());

        self.closed_segments.push(segment);
        self
    }

    pub fn with_open_segment(
        mut self,
        f: impl FnOnce(DqliteSegmentBuilder) -> DqliteSegmentBuilder,
    ) -> Self {
        let segment = f(DqliteSegmentBuilder::new());
        self.open_segments.push(segment);
        self
    }

    fn with_snapshot(
        mut self,
        f: impl FnOnce(DqliteSnapshotBuilder) -> DqliteSnapshotBuilder,
    ) -> Self {
        let mut index = self.first_index;
        for entry in self.closed_segments.iter().chain(self.open_segments.iter()) {
            index += entry.0.iter().fold(0, |c, b| c + b.len()) as u64;
        }
        let timestamp = SystemTime::now();

        let snapshot = f(DqliteSnapshotBuilder::new(self.term, index, timestamp));
        self.snapshots.push(snapshot);
        self
    }
}

impl DqliteDirCreator {
    fn write_segment(&self, file: &mut File, batches: &Vec<Vec<DqliteLogEntry>>) -> Result<()> {
        let mut buf = uvSegmentBuffer::default();
        unsafe { sys::uvSegmentBufferInit(&mut buf, 4096) };

        let rc = unsafe { sys::uvSegmentBufferFormat(&mut buf) };
        if rc != raft_result::OK {
            return Err(anyhow!("failed to format segment buffer: {rc}"));
        }

        for batch in batches {
            let data: Vec<_> = batch
                .iter()
                .map(|e| e.content.encode())
                .collect::<Result<_>>()?;
            let entries: Vec<_> = batch
                .iter()
                .enumerate()
                .map(|(i, e)| raft_entry {
                    term: e.term,
                    type_: e.entry_type(),
                    buf: raft_buffer {
                        // Safety: the buffer is only used within this block and it is only ever read from.
                        base: data[i].as_ptr() as *mut _,
                        len: data[i].len(),
                    },
                    ..Default::default()
                })
                .collect();
            let rc = unsafe {
                sys::uvSegmentBufferAppend(&mut buf, entries.as_ptr(), entries.len() as c_uint)
            };
            if rc != raft_result::OK {
                return Err(anyhow!("failed to append to segment buffer: {rc}"));
            }
        }

        let mut write_buffer = uv_buf_t::default();
        unsafe { sys::uvSegmentBufferFinalize(&mut buf, &mut write_buffer) };
        file.write_all(unsafe { write_buffer.as_bytes() })?;

        Ok(())
    }

    // TODO: add method to add databases in the snapshot. For now the snapshot will be empty (data-wise).
    fn write_snapshot(&self, s: &DqliteSnapshotBuilder, folder: &Path) -> Result<()> {
        let mut path = {
            let term = s.term;
            let index = s.index;
            let timestamp = s.timestamp.duration_since(UNIX_EPOCH)?.as_millis();
            folder.join(format!("snapshot-{term}-{index}-{timestamp}"))
        };

        {
            let mut data = File::create(&path)?;
            let mut header_buf = raft_buffer::default();
            let rc = unsafe { sys::encodeSnapshotHeader(0, &mut header_buf) };
            if rc != raft_result::OK {
                return Err(anyhow!("failed to encode snapshot header"));
            }
            let result = data.write_all(unsafe { header_buf.as_bytes() });
            unsafe { sys::raft_free(header_buf.base) };
            result?; // Avoid leaking `header_buf` content.
        }

        {
            path.set_extension("meta");
            let mut meta = File::create(path)?;

            let mut config = raft_configuration::default();
            unsafe { sys::configurationInit(&mut config) };

            let configuration = s
                .configuration
                .as_ref()
                .expect("cannot write snapshot without configuration");
            for server in &configuration.servers {
                let id = server.id;
                let address = CString::new(server.address.as_str()).unwrap();
                let role = match server.role {
                    RaftRole::Standby => raft_role::RAFT_STANDBY,
                    RaftRole::Voter => raft_role::RAFT_VOTER,
                    RaftRole::Spare => raft_role::RAFT_SPARE,
                } as _;
                let rc = unsafe { sys::configurationAdd(&mut config, id, address.as_ptr(), role) };
                if rc != raft_result::OK {
                    return Err(anyhow!("failed to add server to configuration"));
                }
            }

            let mut config_buf = raft_buffer::default();
            let rc = unsafe { sys::configurationEncode(&config, &mut config_buf) };
            if rc != raft_result::OK {
                return Err(anyhow!("failed to encode configuration"));
            }

            let mut header = [0u8; 32];
            unsafe {
                sys::formatSnapshotMetaHeader(header.as_mut_ptr() as *mut _, s.index, &config_buf)
            };
            let result = meta
                .write_all(header.as_slice())
                .and_then(|_| meta.write_all(unsafe { config_buf.as_bytes() }));
            unsafe { sys::raft_free(config_buf.base) };
            result?;
        }

        Ok(())
    }

    pub fn create(&self) -> Result<()> {
        let mut err = RaftErrorStr::new();

        let rc = unsafe {
            sys::uvMetadataStore(
                CString::new(self.dir.as_os_str().as_bytes())
                    .unwrap()
                    .as_ptr(),
                &uvMetadata {
                    version: 1,
                    term: self.term,
                    voted_for: self.voted_for,
                },
                err.as_mut_ptr(),
            )
        };
        if rc != raft_result::OK {
            return Err(anyhow!("failed to store metadata: {err}"));
        }

        let mut path = self.dir.clone();
        let mut index = self.first_index;
        for closed_segment in self.closed_segments.iter() {
            let last_index = index + closed_segment.0.len() as u64 - 1;
            path.push(format!("{index:0>16}-{last_index:0>16}"));

            let mut file = File::create(path.as_path())?;
            self.write_segment(&mut file, &closed_segment.0)?;

            path.pop();
            index = last_index + 1;
        }

        for (index, open_segment) in self.open_segments.iter().enumerate() {
            path.push(format!("open-{index}"));

            let mut file = File::create(path.as_path())?;
            self.write_segment(&mut file, &open_segment.0)?;

            path.pop();
        }

        for snapshot in &self.snapshots {
            self.write_snapshot(snapshot, self.dir.as_path())?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_dqlite_folder() {
        let dir = tempfile::tempdir().unwrap();
        let err = DqliteDir::open(dir.path()).unwrap_err();

        assert!(err.to_string().contains("not ad dqlite folder"));
    }

    #[test]
    fn test_metadata_only() {
        let dir = tempfile::tempdir().unwrap();
        DqliteDir::creator(dir.path()).create().unwrap();

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

        DqliteDir::creator(dir.path())
            .with_open_segment(|s| s.add_entries(&entries))
            .create()
            .unwrap();

        let state = DqliteDir::open(dir.path()).unwrap();
        assert_eq!(state.term(), 1);
        assert_eq!(state.voted_for(), 0);
        assert_eq!(state.first_index(), 1);
        assert_eq!(state.snapshots().len(), 0);
        assert_eq!(state.segments().len(), 1);

        let open_segment = state.segments().first().unwrap();
        assert!(matches!(open_segment, DqliteSegment::Open { counter, .. } if *counter == 0));
        assert_eq!(open_segment.entries().unwrap(), entries);
    }

    #[test]
    fn test_single_closed_segment() {
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![
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
        DqliteDir::creator(dir.path())
            .with_term(3)
            .with_voted_for(1)
            .with_first_index(1000)
            .with_closed_segment(|s| s.add_entries(&entries))
            .create()
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
        let entries = vec![
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
        DqliteDir::creator(dir.path())
            .with_term(3)
            .with_voted_for(1)
            .with_first_index(1)
            .with_open_segment(|s| s.add_entries(&entries))
            .create()
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
        DqliteDir::creator(dir.path())
            .with_snapshot(|s| {
                s.with_configuration(configuration.clone())
                    .with_term(3)
                    .with_index(1)
                    .with_timestamp(timestamp)
            })
            .create()
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
