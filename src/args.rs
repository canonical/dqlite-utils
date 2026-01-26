use std::path::PathBuf;

use clap::Parser;
use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};

#[derive(Debug, Parser)]
#[command(version, about, styles = Self::styles())]
pub struct Args {
    /// Commands to run, separated by ';'. Can be passed multiple times
    #[arg(short = 'c', value_delimiter = ';', value_name = "command")]
    pub raw_commands: Vec<String>,

    /// Dqlite data directory
    #[arg(long = "dir", global = true)]
    pub dir_path: Option<PathBuf>,

    /// Number of times to retry opening the folder.
    #[arg(long = "retry-count", global = true, default_value_t = 2)]
    pub retry_count: u32,
}

impl Args {
    fn styles() -> Styles {
        // Match cargo output style
        Styles::styled()
            .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
            .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
            .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
            .placeholder(AnsiColor::Cyan.on_default())
            .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
            .valid(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
            .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    }
}
