mod args;
mod command;
mod dqlite;
mod interactive_reader;
mod prompt;
mod utils;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, anyhow};
use clap::Parser;
use owo_colors::Style;
use rustyline::error::ReadlineError;

use self::args::Args;
use self::command::{Command, Help, RootShell, SnapshotShell};
use self::dqlite::{DqliteDir, NoMetadataError};
use self::interactive_reader::CommandHelper;
use self::interactive_reader::InteractiveCommandReader;
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
        run_interactive(InteractiveCommandReader::<ShellKind>::new()?, ctx)
    } else {
        run_batch(stdin_commands(), ctx)
    }
}

fn run_interactive(
    mut command_reader: InteractiveCommandReader<ShellKind>,
    mut ctx: Context,
) -> Result<()> {
    const ERROR_STYLE: Style = Style::new().bright_red();

    println!("{}", command_reader.banner());
    loop {
        command_reader.helper_mut().command_helper = ctx.shell.kind();
        // command_reader.set_shell(ctx.shell.kind());
        let command = match command_reader.read(&ctx) {
            Ok(Some(command)) => command,
            Ok(None) => continue,
            Err(err) => match err.downcast_ref() {
                Some(ReadlineError::Interrupted) => {
                    eprintln!(
                        "{}",
                        "(Press Ctrl+D or type 'quit' to exit)".terminal_style(ERROR_STYLE)
                    );
                    continue;
                }
                Some(ReadlineError::Eof) => break,
                _ => {
                    eprintln!("{}", err.terminal_style(ERROR_STYLE));
                    continue;
                }
            },
        };
        if let Err(err) = command.run(&mut ctx) {
            println!("{:?}", err.terminal_style(ERROR_STYLE));
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
    fn kind(&self) -> ShellKind {
        match self {
            Self::Root(_) => ShellKind::Root,
            Self::Snapshot(_) => ShellKind::Snapshot,
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

#[derive(Copy, Clone, Debug, Default)]
enum ShellKind {
    #[default]
    Root,
    Snapshot,
}

impl ShellKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Snapshot => "snapshot",
        }
    }

    fn help(&self) -> Help {
        match self {
            Self::Root => RootShell::help(),
            Self::Snapshot => SnapshotShell::help(),
        }
    }
}

impl CommandHelper for ShellKind {
    fn known_commands(&self) -> impl Iterator<Item = &'static str> {
        self.help()
            .commands()
            .to_owned() // Ew
            .into_iter()
            .map(|kind| kind.name())
    }
}
