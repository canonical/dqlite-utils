use std::io::{self, IsTerminal, StdoutLock, Write};
use std::process::{Child, Command, Stdio};

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
            Self::Less(child) => child.stdin.as_ref().unwrap().write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Stdout(stdout) => stdout.flush(),
            Self::Less(child) => child.stdin.as_ref().unwrap().flush(),
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
