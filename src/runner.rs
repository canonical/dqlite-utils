use std::path::PathBuf;

use crate::command::Command;
use crate::InteractiveCommandReader;
use crate::Result;

pub struct Runner {
    dir: PathBuf,
}

impl Runner {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn run_interactive(self, command_reader: InteractiveCommandReader) -> Result<()> {
        // This function is a placeholder to allow us to mutate the `command_reader` during
        // iteration, e.g. to add context to its prompt. As there is no context just yet, this
        // function is equivalent to the `run_batch`.
        self.run_batch(command_reader)
    }

    pub fn run_batch(self, commands: impl IntoIterator<Item = Command>) -> Result<()> {
        let Self { dir } = self;
        eprintln!("running in '{}'...", dir.display());
        for command in commands {
            match command {
                Command::Quit => break,
            }
        }
        Ok(())
    }
}
