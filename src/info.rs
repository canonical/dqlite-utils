use std::{
    io::{Write, stdout},
    ops::Range,
    path::PathBuf,
};

use crate::dqlite::{DqliteState, RaftSegment, RaftSnapshot};
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
struct RaftInfo<'a> {
    folder: &'a PathBuf,
    running: bool,
    term: u64,
    voted_for: u64,
    index_range: Range<u64>,
}

#[derive(Serialize)]
struct SnapshotInfo {
    index: u64,
    term: u64,
    created: DateTime<Local>,
}

impl InfoCommand {
    pub fn run(&self, dqlite: &DqliteState) -> Result<()> {
        let mut open_entry_count = 0;
        let mut closed_entry_index = 0;
        let mut first_index = dqlite.snapshots.iter().map(|s| s.index).min().unwrap_or(0);

        for segment in dqlite.segments.iter() {
            match segment {
                RaftSegment::Closed { index_range } => {
                    assert!(index_range.start == closed_entry_index + 1 || closed_entry_index == 0);
                    closed_entry_index = index_range.end;
                    if first_index > index_range.start {
                        first_index = index_range.start;
                    }
                }
                RaftSegment::Open { counter: _ } => {
                    let segment_entries: Result<usize, _> = dqlite
                        .read_segment(&segment)?
                        .without_payload()
                        .map(|batch| {
                            batch.map(|b| {
                                if !b.valid {
                                    println!("invalid batch found");
                                }
                                b.entries.len()
                            })
                        })
                        .sum();
                    open_entry_count += segment_entries? as u64;
                }
            }
        }

        stdout().write_all(
            serde_yaml_ng::to_string(&RaftInfo {
                folder: &dqlite.folder,
                running: dqlite.is_running()?,
                term: dqlite.term,
                index_range: Range {
                    start: first_index,
                    end: closed_entry_index + open_entry_count,
                },
                voted_for: dqlite.voted_for,
            })?
            .as_bytes(),
        )?;
        Ok(())
    }
}

impl From<&RaftSnapshot> for Result<SnapshotInfo> {
    fn from(snapshot: &RaftSnapshot) -> Self {
        return Ok(SnapshotInfo {
            index: snapshot.index,
            term: snapshot.term,
            created: snapshot.created.into(),
        });
    }
}
