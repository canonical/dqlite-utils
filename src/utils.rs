use std::fmt::{Debug, Display};
use std::io::{self, IsTerminal, StdoutLock, Write};
use std::process::{Child, Command, Stdio};

use owo_colors::{OwoColorize, Stream, Style};
use rusqlite::{Connection, Rows, Statement};

use crate::Result;

#[derive(Debug)]
#[allow(unused)]
pub(crate) enum Pager {
    Stdout(StdoutLock<'static>),
    Less(Child),
}

impl Pager {
    #[allow(unused)]
    pub(crate) fn new() -> Result<Self> {
        let stdout = io::stdout();
        if !stdout.is_terminal() {
            return Ok(Self::Stdout(stdout.lock()));
        }

        let less = Command::new("less")
            .arg("-R") // Allow raw control characters (for colors).
            .stdin(Stdio::piped())
            .spawn()?;
        Ok(Self::Less(less))
    }
}

impl Write for Pager {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Stdout(stdout) => stdout.write(buf),
            Self::Less(child) => child
                .stdin
                .as_ref()
                .expect("cannot use child's stdin: already taken")
                .write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Stdout(stdout) => stdout.flush(),
            Self::Less(child) => child
                .stdin
                .as_ref()
                .expect("cannot use child's stdin: already taken")
                .flush(),
        }
    }
}

impl Drop for Pager {
    fn drop(&mut self) {
        match self {
            Self::Stdout(_) => {}
            Self::Less(child) => {
                let _ = child.wait();
            }
        }
    }
}

/// Reduce boilerplates when applying styles.
pub(crate) trait TerminalStylizeExt {
    fn terminal_style(&self, style: Style) -> impl Debug + Display;
}

impl<T: OwoColorize + Debug + Display> TerminalStylizeExt for T {
    fn terminal_style(&self, style: owo_colors::Style) -> impl Debug + Display {
        self.if_supports_color(Stream::Stdout, move |t| t.style(style))
    }
}

pub(crate) trait AttachedSchemasConnectionExt {
    fn attached_schemas(&self) -> Result<AttachedSchemas<'_>>;
}

impl AttachedSchemasConnectionExt for Connection {
    fn attached_schemas(&self) -> Result<AttachedSchemas<'_>> {
        AttachedSchemas::new(self)
    }
}

pub(crate) struct AttachedSchemas<'conn> {
    query: Statement<'conn>,
}

impl<'conn> AttachedSchemas<'conn> {
    fn new(conn: &'conn Connection) -> Result<Self> {
        let query = conn.prepare("PRAGMA database_list;")?;
        Ok(Self { query })
    }

    pub(crate) fn try_iter(&mut self) -> Result<AttachedSchemasIter<'_>> {
        let Self { query } = self;
        let rows = query.query(())?;
        Ok(AttachedSchemasIter { rows })
    }
}

pub(crate) struct AttachedSchemasIter<'query> {
    rows: Rows<'query>,
}

impl<'query> AttachedSchemasIter<'query> {
    pub(crate) fn next(&mut self) -> Result<Option<&str>> {
        let row = match self.rows.next()? {
            Some(row) => row,
            None => return Ok(None),
        };
        let name = row.get_ref("name")?.as_str()?;
        Ok(Some(name))
    }
}
