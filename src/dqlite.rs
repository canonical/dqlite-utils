use std::{
    cmp::Ordering,
    fs::File,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    time::{self, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::Serialize;

pub struct DqliteState {
    pub folder: PathBuf,
    pub term: u64,
    pub index: u64,
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

pub enum RaftSegment {
    Open { counter: u64 },
    Closed { index_range: Range<u64> },
}

pub type RaftConfiguration = Vec<RaftServer>;

pub struct RaftServer {
    pub id: u64,
    pub address: String,
    pub role: RaftRole,
}

pub enum RaftRole {
    Standby,
    Voter,
    Spare,
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

        return Ok(DqliteState {
            folder: folder.clone(),
            term: metadata.term,
            index: 1,
            voted_for: metadata.voted_for,
            snapshots: snapshots,
            segments: segments,
        });
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

    pub fn data_path(&self) -> PathBuf {
        PathBuf::from(format!(
            "snapshot-{}-{}-{}",
            self.term,
            self.index,
            self.created.duration_since(UNIX_EPOCH).unwrap().as_millis(),
        ))
    }

    pub fn meta_path(&self) -> PathBuf {
        PathBuf::from(format!(
            "snapshot-{}-{}-{}.meta",
            self.term,
            self.index,
            self.created.duration_since(UNIX_EPOCH).unwrap().as_millis(),
        ))
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
