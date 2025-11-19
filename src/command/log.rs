use anyhow::{Result, anyhow};
use owo_colors::{OwoColorize, Stream, Style, Styled, SupportsColorsDisplay};
use std::io::{self, ErrorKind, IsTerminal, Write};
use std::process::{self, Child};

use crate::Context;
use crate::dqlite::{DqliteLogEntry, DqliteLogEntryContent, DqliteSegment};

trait TerminalStylize {
    fn terminal_style<'a>(
        &'a self,
        style: Style,
    ) -> SupportsColorsDisplay<'a, Self, Styled<&'a Self>, impl Fn(&'a Self) -> Styled<&'a Self>>;
}

impl<T: OwoColorize> TerminalStylize for T {
    fn terminal_style<'a>(
        &'a self,
        style: Style,
    ) -> SupportsColorsDisplay<'a, Self, Styled<&'a Self>, impl Fn(&'a Self) -> Styled<&'a Self>>
    {
        self.if_supports_color(Stream::Stdout, move |t| t.style(style))
    }
}

enum Pager {
    Stdout(io::StdoutLock<'static>),
    Less(Child),
}

impl Pager {
    fn new() -> Result<Pager> {
        let stdout = io::stdout();
        if !stdout.is_terminal() {
            return Ok(Pager::Stdout(stdout.lock()));
        }

        let less = process::Command::new("less")
            .arg("-R") // Allow raw control characters (for colors).
            .stdin(process::Stdio::piped())
            .spawn()?;
        Ok(Pager::Less(less))
    }
}

impl Write for Pager {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Pager::Stdout(stdout) => stdout.write(buf),
            Pager::Less(child) => child.stdin.as_ref().unwrap().write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Pager::Stdout(stdout) => stdout.flush(),
            Pager::Less(child) => child.stdin.as_ref().unwrap().flush(),
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

struct TermWriter(Option<u64>);

impl TermWriter {
    fn new() -> Self {
        TermWriter(None)
    }

    fn write(&mut self, pager: &mut Pager, term: u64) -> io::Result<()> {
        let (term_changed, marker) = match self.0 {
            Some(t) => (t != term, "├"),
            None => (true, "┌"),
        };
        if term_changed {
            let term_tag = "TERM".terminal_style(Command::TERM_STYLE);
            let term = term.terminal_style(Command::TERM_STYLE);
            writeln!(pager, "{marker} {term_tag} {term}")?;
        }
        self.0 = Some(term);

        Ok(())
    }
}

struct EntryWriter;

impl EntryWriter {
    fn new() -> Self {
        EntryWriter
    }

    fn write_header(
        &self,
        pager: &mut Pager,
        index: u64,
        entry: &DqliteLogEntry,
        tag: &str,
    ) -> io::Result<()> {
        use DqliteLogEntryContent as Dlec;

        let index = index.terminal_style(Command::INDEX_STYLE);
        let tag = tag.terminal_style(Command::TAG_STYLE);

        match &entry.content {
            Dlec::Barrier => {
                let command = "BARRIER".terminal_style(Command::ENTRY_TYPE_STYLE);
                writeln!(pager, "| {index} {command} {tag}")?;
            }
            Dlec::Change(..) => {
                let command = "CONFIG".terminal_style(Command::ENTRY_TYPE_STYLE);
                writeln!(pager, "| {index} {command} {tag}")?;
            }
            Dlec::CommandOpen { filename } => {
                let command = "OPEN".terminal_style(Command::ENTRY_TYPE_STYLE);
                writeln!(pager, "| {index} {command} {} {tag}", filename.display())?;
            }
            Dlec::CommandFrames {
                filename,
                is_commit,
                ..
            } => {
                let command = if *is_commit { "COMMIT" } else { "FRAMES" };
                let command = command.terminal_style(Command::ENTRY_TYPE_STYLE);
                writeln!(pager, "| {index} {command} {} {tag}", filename.display())?;
            }
            Dlec::CommandUndo { .. } => {
                let command = "ROLLBACK".terminal_style(Command::ENTRY_TYPE_STYLE);
                writeln!(pager, "| {index} {command} {tag}")?;
            }
            Dlec::CommandCheckpoint { filename } => {
                let command = "CHECKPOINT".terminal_style(Command::ENTRY_TYPE_STYLE);
                writeln!(pager, "| {index} {command} {} {tag}", filename.display())?;
            }
        };

        Ok(())
    }

    fn write_content(&self, pager: &mut Pager, content: &DqliteLogEntryContent) -> io::Result<()> {
        use DqliteLogEntryContent as Dlec;

        match content {
            Dlec::Change(config) => {
                writeln!(pager, "|    servers:")?;
                for server in &config.servers {
                    writeln!(pager, "|      {}:", server.id)?;
                    writeln!(pager, "|        address: {}", server.address)?;
                    writeln!(pager, "|        role: {:?}", server.role)?;
                }
            }
            Dlec::CommandFrames {
                tx_id,
                frames,
                truncate,
                ..
            } => {
                writeln!(pager, "|    tx_id: {tx_id}")?;
                writeln!(pager, "|    truncate: {truncate}")?;
                // TODO add other fields like the type of the page or the header (with the database size)
                // particularly the database size makes sense as 0 size means "database deleted"
                let pages = frames
                    .iter()
                    .map(|f| f.page_number.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");

                writeln!(pager, "|    pages: {pages}")?;
                writeln!(pager, "|")?;
            }
            Dlec::CommandUndo { tx_id } => {
                writeln!(pager, "|    tx_id: {tx_id}")?;
            }
            Dlec::Barrier | Dlec::CommandOpen { .. } | Dlec::CommandCheckpoint { .. } => {}
        };

        Ok(())
    }
}

impl Command {
    const TERM_STYLE: Style = Style::new().bold().red();
    const INDEX_STYLE: Style = Style::new().yellow();
    const ENTRY_TYPE_STYLE: Style = Style::new().cyan();
    const TAG_STYLE: Style = Style::new().bright_magenta();

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

        let mut term_writer = TermWriter::new();
        let entry_writer = EntryWriter::new();

        let mut log_entry = |(index, entry): (u64, &DqliteLogEntry)| -> io::Result<()> {
            term_writer.write(&mut pager, entry.term)?;

            let tag = if ctx.dqlite.snapshots().iter().any(|s| s.index == index) {
                "[SNAPSHOTTED]"
            } else {
                ""
            };
            entry_writer.write_header(&mut pager, index, entry, tag)?;

            if !self.compact {
                entry_writer.write_content(&mut pager, &entry.content)?;
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

        for segment in open_segments {
            let entries = segment.entries()?;
            index += entries.len() as u64;
        }

        for segment in ctx.dqlite.segments().iter().rev() {
            if let DqliteSegment::Closed { indexes, .. } = segment {
                assert!(index == *indexes.end());
            }

            let entries = segment.entries()?;
            let last_index = index;
            index -= entries.len() as u64;
            let entries = entries
                .iter()
                .rev()
                .enumerate()
                .map(move |(i, entry)| (last_index - i as u64, entry));
            for entry in entries {
                match log_entry(entry) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::BrokenPipe => return Ok(()),
                    Err(e) => return Err(e.into()),
                }
            }
        }

        Ok(())
    }
}
