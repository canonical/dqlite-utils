use std::io::{Write, stdout};

use crate::dqlite::{DqliteState, RaftSnapshot};
use anyhow::Result;
use chrono::{DateTime, Local};
use clap::Parser;
use serde::Serialize;

#[derive(Parser, Debug)]
pub struct InfoCommand {
    #[arg(short, long, default_value_t = false)]
    full: bool,
}

#[derive(Serialize)]
struct RaftInfo {
    folder: String,
    running: bool,
    term: u64,
    index: u64,
    voted_for: u64,
    // segment: SegmentsInfo,
    snapshots: Vec<SnapshotsInfo>,
}

#[derive(Serialize)]
struct SnapshotsInfo {
    index: u64,
    term: u64,
    size: u64,
    created: DateTime<Local>,
}

// Segments:
//   Closed:     2
//   Open:       4
//   FirstIndex: 128
//   LastIndex:  300
// Configuration:
//   6:
//     Address: 10.1.2.2
//     Role: Voter
//   10:
//     Address: 10.1.2.3
//     Role: Spare
// Databases:  ["config-db", "db1", "db2"]
// Snapshots:
//   - Index: 128, Term: 1, Size: 19.5 KiB, Created: 2025-09-23 10:43:02
//   - Index: 256, Term: 2, Size: 195.7 KiB, Created: 2025-09-23 10:44:04
// }

impl InfoCommand {
    pub fn run(&self, dqlite: &DqliteState) -> Result<()> {
        stdout().write_all(
            serde_yaml2::to_string(&RaftInfo {
                folder: dqlite.folder.to_string_lossy().to_string(),
                running: false, // dqlite.is_running()?,
                term: dqlite.term,
                index: dqlite.index,
                voted_for: dqlite.voted_for,
                snapshots: dqlite
                    .snapshots
                    .iter()
                    .map(|s| s.into())
                    .collect::<Result<Vec<SnapshotsInfo>>>()?,
            })?
            .as_bytes(),
        )?;
        Ok(())
    }
}

impl From<&RaftSnapshot> for Result<SnapshotsInfo> {
    fn from(snapshot: &RaftSnapshot) -> Self {
        return Ok(SnapshotsInfo {
            index: snapshot.index,
            term: snapshot.term,
            size: std::fs::metadata(snapshot.data_path())?.len(),
            created: snapshot.created.into(),
        });
    }
}
