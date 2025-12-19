use std::io::{self, ErrorKind, Write};

use anyhow::Result;
use indoc::writedoc;
use owo_colors::Style;

use crate::Context;
use crate::command::help::Help;
use crate::dqlite::{DqliteDir, DqliteLogEntry, DqliteLogEntryContent, DqliteSegment, RaftServer};
use crate::utils::{Pager, TerminalStylizeExt};

use super::UnrecognizedArgumentsError;

#[derive(Debug)]
pub(crate) struct LogCommand {
    compact: bool,
    pager: Pager,
    prev_term: Option<u64>,
}

impl LogCommand {
    const TERM_STYLE: Style = Style::new().bold().green();
    const INDEX_STYLE: Style = Style::new().yellow();
    const ENTRY_TYPE_STYLE: Style = Style::new().cyan();
    const TAG_STYLE: Style = Style::new().bright_magenta();

    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".log")
            .summary("Show a list of all commands applied to the dqlite state machine")
            .add_flag("--compact", "output compactly")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let compact = match args {
            [] => false,
            [flag] if flag == "--compact" => true,
            args => {
                return Err(UnrecognizedArgumentsError(args.to_vec()).into());
            }
        };
        Ok(LogCommand {
            compact,
            pager: Pager::new()?,
            prev_term: None,
        })
    }

    pub(crate) fn run(mut self, ctx: &mut Context) -> Result<()> {
        let dqlite = ctx.dqlite()?;
        // In order to properly get the index of the last entry, we need to read
        // all open entries first.
        let open_segments = dqlite.open_segments();
        let mut index = dqlite
            .closed_segments()
            .last()
            .map_or(dqlite.first_index(), |s| match s {
                DqliteSegment::Closed { indexes, .. } => *indexes.end(),
                DqliteSegment::Open { .. } => unreachable!(),
            });
        for segment in open_segments {
            index += segment.entries()?.len() as u64;
        }

        let mut entry_written = false;
        for segment in dqlite.segments().iter().rev() {
            if let DqliteSegment::Closed { indexes, .. } = segment {
                assert!(index == *indexes.end());
            }

            let entries = segment.entries()?;
            for (i, entry) in entries.iter().rev().enumerate() {
                let entry_index = index - i as u64;
                match self.write_entry(dqlite, entry_index, entry) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::BrokenPipe => return Ok(()),
                    Err(e) => return Err(e.into()),
                }
                entry_written = true;
            }
            index -= entries.len() as u64;
        }
        if !entry_written {
            writeln!(self.pager, "(no entries)")?;
        }

        Ok(())
    }

    fn write_entry(
        &mut self,
        dqlite_dir: &DqliteDir,
        index: u64,
        entry: &DqliteLogEntry,
    ) -> io::Result<()> {
        self.write_term(entry.term)?;

        let snapshot_tag = if dqlite_dir.snapshots().iter().any(|s| s.index == index) {
            "[SNAPSHOT]"
        } else {
            ""
        };
        self.write_entry_header(index, entry, snapshot_tag)?;

        if !self.compact {
            self.write_entry_content(&entry.content)?;
        }

        Ok(())
    }

    fn write_term(&mut self, term: u64) -> io::Result<()> {
        let term_changed = match self.prev_term {
            Some(t) => t != term,
            None => true,
        };
        let marker = if self.prev_term.is_none() {
            "╭"
        } else {
            "├"
        };
        if term_changed {
            let term_tag = "TERM".terminal_style(LogCommand::TERM_STYLE);
            let term = term.terminal_style(LogCommand::TERM_STYLE);
            writeln!(self.pager, "{marker} {term_tag} {term}")?;
        }
        self.prev_term = Some(term);

        Ok(())
    }

    fn write_entry_header(
        &mut self,
        index: u64,
        entry: &DqliteLogEntry,
        tag: &str,
    ) -> io::Result<()> {
        use DqliteLogEntryContent as Dlec;

        let index = index.terminal_style(LogCommand::INDEX_STYLE);
        let tag = tag.terminal_style(LogCommand::TAG_STYLE);

        match &entry.content {
            Dlec::Barrier => {
                let command = "BARRIER".terminal_style(LogCommand::ENTRY_TYPE_STYLE);
                writeln!(self.pager, "│ {index} {command} {tag}")?;
            }
            Dlec::Change(..) => {
                let command = "CONFIG".terminal_style(LogCommand::ENTRY_TYPE_STYLE);
                writeln!(self.pager, "│ {index} {command} {tag}")?;
            }
            Dlec::CommandOpen { filename } => {
                let command = "OPEN".terminal_style(LogCommand::ENTRY_TYPE_STYLE);
                writeln!(
                    self.pager,
                    "│ {index} {command} {} {tag}",
                    filename.display()
                )?;
            }
            Dlec::CommandFrames {
                filename,
                is_commit,
                ..
            } => {
                let command = if *is_commit { "COMMIT" } else { "FRAMES" };
                let command = command.terminal_style(LogCommand::ENTRY_TYPE_STYLE);
                writeln!(
                    self.pager,
                    "│ {index} {command} {} {tag}",
                    filename.display()
                )?;
            }
            Dlec::CommandUndo { .. } => {
                let command = "ROLLBACK".terminal_style(LogCommand::ENTRY_TYPE_STYLE);
                writeln!(self.pager, "│ {index} {command} {tag}")?;
            }
            Dlec::CommandCheckpoint { filename } => {
                let command = "CHECKPOINT".terminal_style(LogCommand::ENTRY_TYPE_STYLE);
                writeln!(
                    self.pager,
                    "│ {index} {command} {} {tag}",
                    filename.display()
                )?;
            }
        };

        Ok(())
    }

    fn write_entry_content(&mut self, content: &DqliteLogEntryContent) -> io::Result<()> {
        use DqliteLogEntryContent as Dlec;

        match content {
            Dlec::Change(config) => {
                writeln!(self.pager, "│   servers:")?;
                for server in &config.servers {
                    let RaftServer {
                        id, address, role, ..
                    } = server;
                    writedoc!(
                        self.pager,
                        "
                            │     {id}:
                            │       address: {address}
                            │       role: {role:?}
                        "
                    )?;
                }
            }
            Dlec::CommandFrames {
                tx_id,
                frames,
                truncate,
                ..
            } => {
                let pages = frames
                    .iter()
                    .map(|f| f.page_number.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");

                writedoc!(
                    self.pager,
                    "
                        │   tx_id: {tx_id}
                        │   truncate: {truncate}
                        │   pages: {pages}
                    "
                )?;
            }
            Dlec::CommandUndo { tx_id } => {
                writeln!(self.pager, "│   tx_id: {tx_id}")?;
            }
            Dlec::Barrier | Dlec::CommandOpen { .. } | Dlec::CommandCheckpoint { .. } => {}
        };

        Ok(())
    }
}
