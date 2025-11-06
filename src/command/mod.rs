use std::ffi::OsString;
use std::io::Write;
use std::process::{self, Child, ChildStdin};
use std::str::FromStr;

use anyhow::anyhow;
use owo_colors::{OwoColorize, Stream, Style};

use crate::dqlite::{
    DqliteLogEntry, DqliteLogEntryContent, DqliteSegment, RaftConfiguration, RaftRole, RaftServer,
};
use crate::{Context, Error, Result};

struct Pager {
    proc: Child,
}

impl Pager {
    fn new() -> Result<Pager> {
        let proc = process::Command::new("less")
            .arg("-R") // Allow raw control characters (for colors).
            .stdin(process::Stdio::piped())
            .spawn()?;

        Ok(Pager { proc })
    }

    fn pipe(&mut self) -> &mut ChildStdin {
        self.proc.stdin.as_mut().expect("pager pipe already taken")
    }
}

impl Drop for Pager {
    fn drop(&mut self) {
        let _ = self.proc.wait();
    }
}

#[derive(Debug)]
pub enum Command {
    Noop,
    Quit,
    Log { verbose: bool },
}

impl Command {
    const TERM_STYLE: Style = Style::new().underline().red();
    const INDEX_STYLE: Style = Style::new().yellow();
    const ENTRY_TYPE_STYLE: Style = Style::new().cyan();
    const SNAPSHOT_TAG_STYLE: Style = Style::new().bright_magenta();

    pub fn run(&self, _ctx: &mut Context) -> Result<()> {
        match self {
            Self::Noop => Ok(()),
            Self::Quit => {
                process::exit(0);
            }
            Self::Log { verbose } => self.log(*verbose, _ctx),
        }
    }

    fn log(&self, verbose: bool, ctx: &mut Context) -> Result<()> {
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
                let snapshotted = snapshotted.if_supports_color(Stream::Stdout, |t| t.style(Command::SNAPSHOT_TAG_STYLE));

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
                        if verbose {
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

                        if verbose {
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
                        if verbose {
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

impl FromStr for Command {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let words = shell_words::split(raw)?;
        let (command, args) = match words.split_first() {
            Some((command, args)) => (command, args),
            None => return Ok(Self::Noop),
        };
        match (command.as_str(), args) {
            ("quit", []) => Ok(Self::Quit),
            ("log", args) => {
                let verbose = match args {
                    [] => true,
                    [flag] if flag == "--oneline" => false,
                    args => {
                        return Err(anyhow!("unrecognised arguments {args:?} for 'log' command"));
                    }
                };
                Ok(Self::Log { verbose })
            }
            (unknown, []) => Err(anyhow!("unknown command '{unknown}'")),
            (_, tail) => Err(anyhow!("unrecognised arguments {tail:?}")),
        }
    }
}
