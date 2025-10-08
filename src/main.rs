use std::fs::File;
use std::io::stdin;
use std::iter::once;
use std::process::exit;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rustyline::Editor;
use rustyline::history::DefaultHistory;

#[derive(Parser, Debug)]
#[command(after_help = "")]
struct Args {
    /// The commands to run, separated by semicolons.
    #[arg(short, long, value_delimiter = ';')]
    commands: Vec<String>,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    Quit,
}

#[derive(Parser, Debug)]
struct CommandParser {
    #[clap(subcommand)]
    command: Command,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if !args.commands.is_empty() {
        for command in args.commands {
            let command = command.trim();
            if !command.is_empty() {
                parse_and_run(&command)?;
            }
        }
    } else if atty::is(atty::Stream::Stdin) {
        run_interactive()?;
    } else {
        run_batch()?;
    }

    Ok(())
}

fn run_interactive() -> Result<()> {
    let history_file = home::home_dir().unwrap().join(".dqlite-utils-history");

    let mut rl = Editor::<(), DefaultHistory>::new()?;

    if history_file.exists() {
        rl.load_history(&history_file).ok();
    } else if let Err(err) = File::create(&history_file) {
        eprintln!("Error creating history file: {err}");
    }

    println!("Welcome to dqlite-utils! Type 'help' for commands  or 'quit' to exit.");

    loop {
        let readline = rl.readline("> ");
        match readline {
            Ok(line) => {
                let line = line.trim();
                if !line.is_empty() {
                    rl.add_history_entry(line).ok();
                }
                if line == "quit" {
                    break;
                }
                if let Err(err) = parse_and_run(line) {
                    eprintln!("Error: {err}");
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!(" (Press Ctrl+D or type 'quit' to exit)");
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                eprintln!("Error reading input: {:?}", err);
                break;
            }
        }
    }

    if let Err(err) = rl.save_history(&history_file) {
        eprintln!("Error saving history: {err}");
    }

    Ok(())
}

fn run_batch() -> Result<()> {
    for command in stdin().lines() {
        let command = command?;
        let command = command.trim();
        if !command.is_empty() {
            // When running in batch mode, we bail out on errors
            parse_and_run(command)?;
        }
    }
    Ok(())
}

fn parse_and_run(command: &str) -> Result<()> {
    let args = shell_words::split(command).context("Invalid command")?;
    run(
        CommandParser::try_parse_from(once("dqlite-utils").chain(args.iter().map(|s| s.as_str())))?
            .command,
    )
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Quit => exit(0),
    }
}
