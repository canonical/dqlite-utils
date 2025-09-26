use std::{
    cmp::Ordering,
    fs::File,
    io::{BufRead, BufReader, Read},
    ops::Range,
    os::fd::AsRawFd,
    path::PathBuf,
    time::{self, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::Serialize;

use crate::crc32::crc32;

pub struct DqliteState {
    pub folder: PathBuf,
    pub term: u64,
    pub voted_for: u64,
    pub snapshots: Vec<RaftSnapshot>,
    pub segments: Vec<RaftSegment>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RaftSnapshot {
    pub term: u64,
    pub index: u64,
    pub created: SystemTime,
    pub has_meta: bool,
    pub has_data: bool,
}

#[derive(Debug)]
pub enum RaftSegment {
    Open { counter: u64 },
    Closed { index_range: Range<u64> },
}

impl DqliteState {
    pub fn from(folder: &PathBuf) -> Result<DqliteState> {
        let metadata = Metadata::from_folder(&folder)?;

        let mut snapshots = Vec::new();
        let mut segments = Vec::new();
        for f in std::fs::read_dir(&folder)? {
            let f = f?;

            let filetype = f.file_type()?;
            if !filetype.is_file() {
                continue;
            }

            let filename = f.file_name();
            let filename = match filename.to_str() {
                Some(str) => str,
                _ => continue,
            };
            if let Some(snapshot) = RaftSnapshot::from_filename(&filename) {
                snapshot.match_or_add(&mut snapshots);
            }
            if let Some(segment) = RaftSegment::from_filename(&filename) {
                segments.push(segment);
            }
        }

        snapshots.sort_unstable_by_key(|s| (s.term, s.index, s.created));
        segments.sort_unstable_by_key(|s| match s {
            RaftSegment::Closed { index_range } => (0, index_range.start, index_range.end),
            RaftSegment::Open { counter } => (1, *counter, 0),
        });

        return Ok(DqliteState {
            folder: folder.clone(),
            term: metadata.term,
            voted_for: metadata.voted_for,
            snapshots: snapshots,
            segments: segments,
        });
    }

    pub fn is_running(&self) -> Result<bool> {
        let lock_file = match File::open(self.folder.join("dqlite-lock")) {
            Ok(file) => file,
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => return Ok(false),
                _ => return Err(err.into()),
            },
        };
        let mut lock = libc::flock {
            l_type: libc::F_WRLCK as i16,
            l_whence: libc::SEEK_SET as i16,
            l_start: 0,
            l_len: 0,
            l_pid: 0,
        };
        if unsafe { libc::fcntl(lock_file.as_raw_fd(), libc::F_GETLK, &mut lock) } == -1 {
            return Err(anyhow::format_err!("fcntl failed"));
        }
        Ok(lock.l_type as i32 == libc::F_LOCK)
    }

    pub fn read_segment(&self, segment: &RaftSegment) -> Result<RaftSegmentIterator> {
        let filename = match segment {
            RaftSegment::Open { counter } => format!("open-{}", counter),
            RaftSegment::Closed { index_range } => {
                format!("{}-{}", index_range.start, index_range.end)
            }
        };
        let mut path = self.folder.clone();
        path.push(filename);

        RaftSegmentIterator::new(File::open(path)?)
    }
}

struct Metadata {
    version: u64,
    term: u64,
    voted_for: u64,
}

impl Metadata {
    fn from_folder(path: &PathBuf) -> Result<Metadata> {
        let mut path = path.clone();
        path.push("metadata1");
        let metadata1 = Metadata::from_file(&path);
        path.pop();

        path.push("metadata2");
        let metadata2 = Metadata::from_file(&path);
        path.pop();

        match (metadata1, metadata2) {
            (Ok(metadata1), Ok(metadata2)) => match metadata1.version.cmp(&metadata2.version) {
                Ordering::Equal => Err(anyhow::format_err!(
                    "corrupted metadata: both at version {}",
                    metadata1.version
                )),
                Ordering::Greater => Ok(metadata1),
                Ordering::Less => Ok(metadata2),
            },
            (Err(_), Ok(metadata)) => Ok(metadata),
            (Ok(metadata), Err(_)) => Ok(metadata),
            (Err(_), Err(_)) => Err(anyhow::format_err!("couldn't read metadata files")),
        }
    }

    fn from_file(path: &PathBuf) -> Result<Metadata> {
        const RAFT_METADATA_DISK_FORMAT: u64 = 1;

        let mut buffer = [0u8; 32];

        let mut file = File::open(path)?;
        file.read_exact(&mut buffer)?;
        if file.read(&mut [0u8; 1])? != 0 {
            return Err(anyhow::format_err!("File is larger than 32 bytes"));
        }

        let format = u64::from_le_bytes(buffer[0..8].try_into().unwrap());
        if format != RAFT_METADATA_DISK_FORMAT {
            return Err(anyhow::format_err!(
                "metadata format mismatch (format: {format})"
            ));
        }
        let version = u64::from_le_bytes(buffer[8..16].try_into().unwrap());
        if version == 0 {
            return Err(anyhow::format_err!("metadata version must not be 0"));
        }

        let term = u64::from_le_bytes(buffer[16..24].try_into().unwrap());
        let voted_for = u64::from_le_bytes(buffer[24..32].try_into().unwrap());
        return Ok(Metadata {
            version,
            term,
            voted_for,
        });
    }
}

impl RaftSnapshot {
    pub fn from_filename(filename: &str) -> Option<RaftSnapshot> {
        let str = filename.strip_prefix("snapshot-")?;

        let mut parts = str.split(['-', '.']);

        let term = parts.next()?.parse::<u64>().ok()?;
        let index = parts.next()?.parse::<u64>().ok()?;
        let created = parts.next()?.parse::<u64>().ok()?;
        let created = UNIX_EPOCH.checked_add(time::Duration::from_millis(created))?;
        let is_meta = match parts.next() {
            Some(postfix) => postfix == "meta",
            _ => false,
        };

        if is_meta && parts.next().is_some() {
            return None;
        }

        Some(RaftSnapshot {
            term,
            index,
            created,
            has_meta: is_meta,
            has_data: !is_meta,
        })
    }

    pub fn match_or_add(&self, vec: &mut Vec<RaftSnapshot>) {
        for other in vec.into_iter() {
            if other.term == self.term && other.index == self.index && other.created == self.created
            {
                other.has_data |= self.has_data;
                other.has_meta |= self.has_meta;
                return;
            }
        }

        vec.push(*self);
    }
}

impl RaftSegment {
    pub fn from_filename(filename: &str) -> Option<Self> {
        if let Some(filename) = filename.strip_prefix("open-") {
            let counter = filename.parse::<u64>().ok()?;
            Some(RaftSegment::Open { counter: counter })
        } else {
            let (start, end) = filename.split_once('-')?;
            if start.len() != 16 || end.len() != 16 {
                return None;
            }

            let start = start.parse::<u64>().ok()?;
            let end = end.parse::<u64>().ok()?;

            Some(RaftSegment::Closed {
                index_range: (start..end),
            })
        }
    }
}

/*
 * The format of a segment is:
 *
 * +---+---+---+---+---+---+---+---+
 * |         FORMAT_VERSION        |
 * +---+---+---+---+---+---+---+---+
 *
 * +=========+=========+     +=========+
 * | BATCH_1 | BATCH_2 | ... | BATCH_N |
 * +=========+=========+     +=========+
 *
 * Each contains many log entries and is composed by a header and a data segment, with the following format:
 *
 * +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
 * |HEADER_CHECKSUM| DATA_CHECKSUM |          ENTRY_COUNT          |
 * +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
 * |          ENTRY_1_TERM         | ENTRY_1_TYPE  | ENTRY_1_SIZE  |
 * +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
 *                                ...
 * +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
 * |          ENTRY_N_TERM         | ENTRY_N_TYPE  | ENTRY_N_SIZE  |
 * +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
 *
 * Data:
 *
 * +===<ENTRY_1_SIZE>===+===<ENTRY_2_SIZE>===+     +===<ENTRY_N_SIZE>===+
 * |    DATA_ENTRY_1    |    DATA_ENTRY_2    | ... |    DATA_ENTRY_N    |
 * +====================+====================+     +====================+
 *
 * All numbers are encoded in little-endian.
 *
 * It is ok for the last batch to be corrupted, but not for other batches.
 */
pub struct RaftSegmentIterator {
    file: BufReader<File>,
    header_only: bool,
}

impl RaftSegmentIterator {
    const MAX_SEGMENT_SIZE: usize = 8 * 1024 * 1024;
    const MIN_ENTRY_SIZE: usize = 16 /* header */ + 8 /* data */;
    const MAX_ENTRIES_PER_SEGMENT: usize = Self::MAX_SEGMENT_SIZE / Self::MIN_ENTRY_SIZE;
    const CAPACITY: usize = Self::MAX_ENTRIES_PER_SEGMENT.next_power_of_two();

    fn new(file: File) -> Result<RaftSegmentIterator> {
        let mut file = BufReader::with_capacity(Self::CAPACITY, file);

        let buf = file.peek(8)?;
        assert!(buf.len() <= 8);
        if buf.len() == 8 {
            let format_version = u64::from_le_bytes(buf[0..8].try_into().unwrap());
            file.consume(8);
            if format_version == 0 {
                /* Should be empty */
                let buf = file.peek(16)?;
                if buf.len() == 0 || buf == [0; 16] {
                    return Ok(RaftSegmentIterator {
                        file,
                        header_only: true,
                    });
                }
            }
            if format_version != 1 {
                return Err(anyhow::format_err!("invalid format version"));
            }
        } else if buf.len() > 0 {
            return Err(anyhow::format_err!("corrupted segment"));
        }

        Ok(RaftSegmentIterator {
            file,
            header_only: false,
        })
    }

    pub fn without_payload(mut self) -> RaftSegmentIterator {
        self.header_only = true;
        self
    }
}

impl Iterator for RaftSegmentIterator {
    type Item = Result<RaftSegmentBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        let buf = match self.file.peek(16) {
            Ok(buf) => buf,
            Err(err) => return Some(Err(err.into())),
        };
        if buf.len() == 0 {
            return None;
        } else if buf.len() < 16 {
            return Some(Err(anyhow::format_err!("corrupted segment")));
        }
        let header_checksum = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let data_checksum = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let entry_count = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let header_crc = crc32(&buf[8..16], 0);
        self.file.consume(16);
        if entry_count == 0 {
            if data_checksum != 0 || header_checksum != 0 {
                return Some(Err(anyhow::format_err!("corrupted segment")));
            }
            return None;
        }

        Some((|| {
            let header_size = entry_count as usize * 16;
            let header = self.file.peek(header_size)?;
            let header_crc = crc32(header, header_crc);
            let header_valid = header.len() == header_size && header_crc == header_checksum;
            if !header_valid {
                return Ok(RaftSegmentBatch {
                    valid: false,
                    entries: Vec::new(),
                });
            }

            let mut data_size = 0;
            let mut entries = Vec::with_capacity(entry_count as usize);
            for entry in header.chunks(16) {
                let term = u64::from_le_bytes(entry[0..8].try_into().unwrap());
                let type_ = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as u8;
                let size = u32::from_le_bytes(entry[12..16].try_into().unwrap());
                data_size += size as i64;
                entries.push(RaftEntry {
                    term,
                    type_,
                    size,
                    data: None,
                });
            }
            self.file.consume(header_size);

            let mut data_valid = true;
            if !self.header_only {
                let mut data_crc = 0;
                for entry in entries.iter_mut() {
                    let data = self.file.peek(entry.size as usize)?;
                    if data.len() != entry.size as usize {
                        data_valid = false;
                        break;
                    }
                    data_crc = crc32(data, data_crc);
                    entry.data = Some(Vec::from(data));
                    self.file.consume(entry.size as usize);
                }
                if data_crc != data_checksum {
                    data_valid = false;
                }
            } else {
                self.file.seek_relative(data_size)?;
            }

            Ok(RaftSegmentBatch {
                valid: header_valid && data_valid,
                entries,
            })
        })())
    }
}

pub struct RaftSegmentBatch {
    pub valid: bool,
    pub entries: Vec<RaftEntry>,
}

pub struct RaftEntry {
    pub term: u64,
    pub type_: u8,
    pub size: u32,
    pub data: Option<Vec<u8>>,
}
