mod args;
mod command;
mod dqlite_sys;
mod runner;

use std::fs::File;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Context};
use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::Editor;

use self::args::Args;
use self::command::Command;
use self::runner::Runner;

pub type Error = anyhow::Error;
pub type Result<T> = anyhow::Result<T>;

fn main() -> ExitCode {
    match exec(Args::parse()) {
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

fn exec(args: Args) -> Result<()> {
    let Args { raw_commands, dir } = args;

    let runner = Runner::new(dir);
    if !raw_commands.is_empty() {
        let commands: Vec<_> = raw_commands
            .into_iter()
            .map(|command| command.parse())
            .collect::<Result<_>>()?;
        runner.run_batch(commands)
    } else if io::stdin().is_terminal() {
        runner.run_interactive(InteractiveCommandReader::new()?)
    } else {
        runner.run_batch(stdin_commands()?)
    }
}

struct InteractiveCommandReader {
    history_path: PathBuf,

    // TODO(kcza): improve completion.
    line_editor: Editor<(), DefaultHistory>,
}

impl InteractiveCommandReader {
    fn new() -> Result<Self> {
        const HISTORY_FILE: &str = ".dqlite-utils-history";
        let home_dir = home::home_dir().with_context(|| anyhow!("cannot get home directory"))?;
        let history_path = home_dir.join(HISTORY_FILE);

        let mut line_editor = Editor::new()?;
        let loaded_history = line_editor.load_history(&history_path).is_ok();
        if !loaded_history {
            File::create(&history_path)
                .with_context(|| anyhow!("cannot create {}", history_path.display()))?;
        }

        Ok(Self {
            history_path,
            line_editor,
        })
    }

    fn next_command(&mut self) -> Result<Option<Command>> {
        let line = self.line_editor.readline("> ")?;
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            return Ok(None);
        }
        let ret = trimmed_line.parse().map(Some);
        self.line_editor.add_history_entry(line)?;
        ret
    }
}

impl Drop for InteractiveCommandReader {
    fn drop(&mut self) {
        if let Err(err) = self.line_editor.save_history(&self.history_path) {
            eprintln!("cannot save history: {err}");
        }
    }
}

impl Iterator for InteractiveCommandReader {
    type Item = Command;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.next_command() {
                Ok(Some(command)) => return Some(command),
                Ok(None) => continue,
                Err(err) => match err.downcast_ref() {
                    Some(ReadlineError::Interrupted) => {
                        eprintln!("(Press Ctrl+D or type 'quit' to exit)")
                    }
                    Some(ReadlineError::Eof) => return Some(Command::Quit),
                    _ => eprintln!("{err}"),
                },
            }
        }
    }
}

fn stdin_commands() -> Result<Vec<Command>> {
    io::stdin()
        .lines() // Assumes 1-line commands only
        .enumerate()
        .filter(|(_, line)| line.as_ref().is_ok_and(|line| !line.trim().is_empty()))
        .map(|(line_num, line)| {
            line?
                .parse()
                .with_context(|| anyhow!("cannot parse line {line_num}"))
        })
        .collect()
}
