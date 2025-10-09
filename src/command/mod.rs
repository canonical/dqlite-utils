use std::str::FromStr;

use anyhow::anyhow;

use crate::{Error, Result};

#[derive(Debug)]
pub enum Command {
    Quit,
}

impl FromStr for Command {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let words = shell_words::split(&raw)?;
        let (command, args) = match words.split_first() {
            Some((command, args)) => (command, args),
            None => return Err(anyhow!("unexpected end of input")),
        };
        match (command.as_str(), args) {
            ("quit", []) => Ok(Self::Quit),
            (unknown, []) => Err(anyhow!("unknown command {unknown}")),
            (_, tail) => Err(anyhow!("unrecognised arguments: {}", tail.join(" "))),
        }
    }
}
