use anyhow::{Result, anyhow};
use owo_colors::{OwoColorize, Stream, Style};
use std::io::{self, IsTerminal, Write};
use std::process::{self, Child};

use crate::Context;
use crate::dqlite::{DqliteLogEntry, DqliteLogEntryContent, DqliteSegment};

enum Pager {
    Stdout(io::StdoutLock<'static>),
    Less(Child),
}

impl Pager {
    fn new() -> Result<Pager> {
        if !io::stdout().is_terminal() {
            return Ok(Pager::Stdout(io::stdout().lock()));
        }

        let proc = process::Command::new("less")
            .arg("-R") // Allow raw control characters (for colors).
            .stdin(process::Stdio::piped())
            .spawn()?;

        Ok(Pager::Less(proc))
    }

    fn pipe(&mut self) -> &mut dyn Write {
        match self {
            Pager::Stdout(stdout) => stdout,
            Pager::Less(child) => child.stdin.as_mut().expect("pager pipe already taken"),
        }
    }
}

impl Drop for Pager {
    fn drop(&mut self) {
        match self {
            Pager::Stdout(_) => {}
            Pager::Less(child) => {
                let _ = child.wait();
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Command {
    compact: bool,
}

impl Command {
    const TERM_STYLE: Style = Style::new().underline().red();
    const INDEX_STYLE: Style = Style::new().yellow();
    const ENTRY_TYPE_STYLE: Style = Style::new().cyan();
    const SNAPSHOT_TAG_STYLE: Style = Style::new().bright_magenta();

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let compact = match args {
            [] => false,
            [flag] if flag == "--compact" => true,
            args => {
                return Err(anyhow!("unrecognised arguments {args:?} for 'log' command"));
            }
        };
        Ok(Command { compact })
    }

    pub(crate) fn run(&self, ctx: &mut Context) -> Result<()> {
        // Spawn a `less` process to page through the log output.
        let mut pager = Pager::new()?;
        let out = pager.pipe();

        let mut term = None;
        let mut log_entry =
            |index: u64, entry: &DqliteLogEntry| -> core::result::Result<(), std::io::Error> {
                term = Some(match term {
                    Some(t) => {
                        if t != entry.term {
                            writeln!(
                                out,
                                "├ {} {}",
                                "TERM".if_supports_color(Stream::Stdout, |t| t
                                    .style(Command::TERM_STYLE)),
                                entry.term.if_supports_color(Stream::Stdout, |t| t
                                    .style(Command::TERM_STYLE))
                            )?;
                        }
                        entry.term
                    }
                    None => {
                        writeln!(
                            out,
                            "┌ {} {}",
                            "TERM".if_supports_color(Stream::Stdout, |t| t
                                .style(Command::TERM_STYLE)),
                            entry.term.if_supports_color(Stream::Stdout, |t| t
                                .style(Command::TERM_STYLE))
                        )?;
                        entry.term
                    }
                });

                let snapshotted = if ctx.dqlite.snapshots().iter().any(|s| s.index == index) {
                    "[SNAPSHOTTED]"
                } else {
                    ""
                };
                let snapshotted = snapshotted
                    .if_supports_color(Stream::Stdout, |t| t.style(Command::SNAPSHOT_TAG_STYLE));

                write!(
                    out,
                    "| {} ",
                    index.if_supports_color(Stream::Stdout, |i| i.style(Command::INDEX_STYLE))
                )?;
                use DqliteLogEntryContent as Dlec;
                match &entry.content {
                    Dlec::Barrier => {
                        writeln!(
                            out,
                            "{} {}",
                            "BARRIER".if_supports_color(Stream::Stdout, |t| t
                                .style(Command::ENTRY_TYPE_STYLE)),
                            snapshotted
                        )?;
                    }
                    Dlec::Change(config) => {
                        writeln!(
                            out,
                            "{} {}",
                            "CONFIG".if_supports_color(Stream::Stdout, |t| t
                                .style(Command::ENTRY_TYPE_STYLE)),
                            snapshotted
                        )?;
                        if !self.compact {
                            writeln!(out, "|\tservers:")?;
                            for server in &config.servers {
                                writeln!(out, "|\t  {}:", server.id)?;
                                writeln!(out, "|\t    address: {}", server.address)?;
                                writeln!(out, "|\t    role: {:?}", server.role)?;
                            }
                        }
                    }
                    Dlec::CommandOpen { filename } => {
                        writeln!(
                            out,
                            "{} {} {}",
                            "OPEN".if_supports_color(Stream::Stdout, |t| t
                                .style(Command::ENTRY_TYPE_STYLE)),
                            filename.display(),
                            snapshotted
                        )?;
                    }
                    Dlec::CommandFrames {
                        filename,
                        is_commit,
                        tx_id,
                        frames,
                        truncate,
                    } => {
                        let action = if *is_commit { "COMMIT" } else { "FRAMES" };
                        writeln!(
                            out,
                            "{} {} {}",
                            action.if_supports_color(Stream::Stdout, |t| t
                                .style(Command::ENTRY_TYPE_STYLE)),
                            filename.display(),
                            snapshotted
                        )?;

                        if !self.compact {
                            writeln!(out, "|\ttx_id: {tx_id}",)?;
                            writeln!(out, "|\ttruncate: {truncate}")?;
                            // TODO add other fields like the type of the page or the header (with the database size)
                            // particularly the database size makes sense as 0 size means "database deleted"
                            writeln!(
                                out,
                                "|\tpages: {}",
                                frames
                                    .iter()
                                    .map(|f| f.page_number.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )?;
                            writeln!(out, "|")?;
                        }
                    }
                    Dlec::CommandUndo { tx_id } => {
                        writeln!(
                            out,
                            "{} {}",
                            "ROLLBACK".if_supports_color(Stream::Stdout, |t| t
                                .style(Command::ENTRY_TYPE_STYLE)),
                            snapshotted
                        )?;
                        if !self.compact {
                            writeln!(out, "|\ttx_id: {tx_id}",)?;
                        }
                    }
                    Dlec::CommandCheckpoint { filename } => {
                        writeln!(
                            out,
                            "{} {} {}",
                            "CHECKPOINT".if_supports_color(Stream::Stdout, |t| t
                                .style(Command::ENTRY_TYPE_STYLE)),
                            filename.display(),
                            snapshotted
                        )?;
                    }
                };

                Ok(())
            };

        let mut log_entries = |index, entries: &[_]| -> core::result::Result<(), std::io::Error> {
            for (i, entry) in entries.iter().rev().enumerate() {
                log_entry(index - i as u64, entry)?;
            }
            Ok(())
        };

        // In order to properly get the index of the last entry, we need to read
        // all open entries first.
        let open_segments = ctx.dqlite.open_segments();
        let mut index = ctx
            .dqlite
            .closed_segments()
            .last()
            .map_or(ctx.dqlite.first_index(), |s| match s {
                DqliteSegment::Closed { indexes, .. } => *indexes.end(),
                DqliteSegment::Open { .. } => unreachable!(),
            });

        if !open_segments.is_empty() {
            for segment in open_segments {
                let entries = segment.entries()?;
                index += entries.len() as u64;
            }
        }

        for segment in ctx.dqlite.segments().iter().rev() {
            if let DqliteSegment::Closed { indexes, .. } = segment {
                assert!(index == *indexes.end());
            }

            let entries = segment.entries()?;
            match log_entries(index, entries) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
                Err(e) => return Err(anyhow!("{}", e)),
            }
            index -= entries.len() as u64;
        }

        Ok(())
    }
}
