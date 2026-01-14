mod args;
mod command;
mod dqlite;
mod prompt;
mod rusqlite_ext;
mod utils;

use std::fmt::Display;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, anyhow};
use clap::Parser;
use owo_colors::Style;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;

use self::args::Args;
use self::command::{Command, Help, RootShell, SnapshotShell};
use self::dqlite::{DqliteDir, NoMetadataError};
use self::prompt::Prompt;
use self::utils::TerminalStylizeExt;

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
    let Args {
        raw_commands,
        dir_path,
    } = args;

    let mut ctx = Context::new();
    if let Some(dir_path) = dir_path {
        ctx.open(&dir_path)
            .with_context(|| anyhow!("cannot open {}", dir_path.display()))?;
    } else if let Err(err) = ctx.open(PathBuf::from(".")) {
        if !err.is::<NoMetadataError>() {
            return Err(err).with_context(|| anyhow!("cannot open current directory"));
        }
        eprintln!(
            "{}: {err}",
            "warning".terminal_style(Style::new().yellow().bold())
        );
    }

    if !raw_commands.is_empty() {
        let commands: Vec<_> = raw_commands
            .into_iter()
            .map(|command| command.parse())
            .collect::<Result<_>>()?;
        run_batch(commands, ctx)
    } else if io::stdin().is_terminal() {
        run_interactive(InteractiveCommandReader::new()?, ctx)
    } else {
        run_batch(stdin_commands(), ctx)
    }
}

fn run_interactive(mut command_reader: InteractiveCommandReader, mut ctx: Context) -> Result<()> {
    println!("{}", command_reader.banner());
    loop {
        let command = match command_reader.read(&ctx) {
            Ok(Some(command)) => command,
            Ok(None) => continue,
            Err(err) => match err.downcast_ref() {
                Some(ReadlineError::Interrupted) => {
                    eprintln!(
                        "{}",
                        "(Press Ctrl+D or type 'quit' to exit)"
                            .terminal_style(InteractiveCommandReader::ERROR_STYLE)
                    );
                    continue;
                }
                Some(ReadlineError::Eof) => break,
                _ => {
                    eprintln!(
                        "{}",
                        err.terminal_style(InteractiveCommandReader::ERROR_STYLE)
                    );
                    continue;
                }
            },
        };
        if let Err(err) = command.run(&mut ctx) {
            println!(
                "{:?}",
                err.terminal_style(InteractiveCommandReader::ERROR_STYLE)
            );
        }
    }
    Ok(())
}

fn run_batch(commands: impl IntoIterator<Item = Command>, mut ctx: Context) -> Result<()> {
    for command in commands {
        command.run(&mut ctx)?;
    }
    Ok(())
}

struct InteractiveCommandReader {
    history_path: Option<PathBuf>,

    // TODO(kcza): improve completion.
    line_editor: Editor<(), DefaultHistory>,
}

impl InteractiveCommandReader {
    const ERROR_STYLE: Style = Style::new().bright_red();

    fn new() -> Result<Self> {
        const HISTORY_FILE: &str = ".dqlite-utils-history";

        let mut line_editor = Editor::new()?;
        let history_path = home::home_dir().map(|home| home.join(HISTORY_FILE));
        if let Some(history_path) = &history_path {
            line_editor.load_history(&history_path).ok();
        } else {
            eprintln!("cannot load history");
        }
        Ok(Self {
            history_path,
            line_editor,
        })
    }

    fn banner(&self) -> impl Display {
        r#"enter ".help" for usage hints"#
    }

    fn read(&mut self, ctx: &Context) -> Result<Option<Command>> {
        let line = self.line_editor.readline(ctx.shell.prompt().as_str())?;
        let trimmed_line = line.trim();
        let ret = trimmed_line.parse().map(Some);
        self.line_editor.add_history_entry(line)?;
        ret
    }
}

impl Drop for InteractiveCommandReader {
    fn drop(&mut self) {
        if let Some(history_path) = &self.history_path
            && let Err(err) = self.line_editor.save_history(history_path)
        {
            eprintln!("cannot save history: {err}");
        }
    }
}

fn stdin_commands() -> impl Iterator<Item = Command> {
    io::stdin()
        .lines() // Assumes 1-line commands only
        .enumerate()
        .map(|(line_num, line)| {
            line?
                .parse()
                .with_context(|| anyhow!("cannot parse line {}", line_num + 1))
        })
        .scan(false, |error_seen, command| match (*error_seen, command) {
            (true, _) => None, // Stop after first error.
            (_, Ok(command)) => Some(command),
            (_, Err(err)) => {
                eprintln!("{err}");
                *error_seen = true;
                None
            }
        })
}

#[derive(Debug, Default)]
pub struct Context {
    pub dqlite: Option<DqliteDir>,
    pub shell: Shell,
}

impl Context {
    fn new() -> Self {
        Self::default()
    }

    fn open(&mut self, dir_path: impl Into<PathBuf>) -> Result<&DqliteDir> {
        let dir = DqliteDir::open(dir_path)?;
        let ret = self.dqlite.insert(dir);
        Ok(ret)
    }

    fn dqlite(&self) -> Result<&DqliteDir> {
        Ok(self.dqlite.as_ref().ok_or(NoOpenDqliteDir)?)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("no open dqlite directory")]
struct NoOpenDqliteDir;

#[derive(Debug)]
pub enum Shell {
    Root(RootShell),
    Snapshot(SnapshotShell),
}

impl Shell {
    fn name(&self) -> &'static str {
        match self {
            Self::Root(_) => "root",
            Self::Snapshot(_) => "snapshot",
        }
    }

    fn help(&self) -> Help {
        match self {
            Shell::Root(_) => RootShell::help(),
            Shell::Snapshot(_) => SnapshotShell::help(),
        }
    }

    fn prompt(&self) -> &Prompt {
        match self {
            Self::Root(shell) => shell.prompt(),
            Self::Snapshot(shell) => shell.prompt(),
        }
    }

    fn snapshot(&self) -> Option<&SnapshotShell> {
        match self {
            Self::Snapshot(shell) => Some(shell),
            _ => None,
        }
    }

    fn snapshot_mut(&mut self) -> Option<&mut SnapshotShell> {
        match self {
            Self::Snapshot(shell) => Some(shell),
            _ => None,
        }
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::Root(RootShell::default())
    }
}
