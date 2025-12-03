use std::fmt::Display;
use std::io::{self, IsTerminal, StdoutLock, Write};
use std::mem::ManuallyDrop;
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;

use owo_colors::{OwoColorize, Stream, Style};

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
    fn terminal_style(&self, style: Style) -> impl Display;
}

impl<T: OwoColorize + Display> TerminalStylizeExt for T {
    fn terminal_style(&self, style: owo_colors::Style) -> impl Display {
        self.if_supports_color(Stream::Stdout, move |t| t.style(style))
    }
}
