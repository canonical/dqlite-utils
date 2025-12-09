use std::ffi::CString;
use std::io::Write;
use std::time::SystemTime;

use anyhow::anyhow;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::command::snapshot::{ShellSnapshotContext, SnapshotShell};
use crate::dqlite::{DqliteDatabaseWriter, DqliteDir, RaftConfiguration};
use crate::prompt::Prompt;
use crate::{Context, Result, Shell};

pub(crate) struct FinishCommand;

impl FinishCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".finish")
            .summary("validate snapshot and write to disk")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let shell = ctx.shell.snapshot().ok_or_else(|| {
            anyhow!("internal error: finish command not called in snapshot shell")
        })?;
        let SnapshotShell { path, snapshot } = shell;
        println!("writing snapshot to {}", path.display());

        let ShellSnapshotContext {
            term,
            index,
            timestamp,
            configuration,
        } = snapshot;
        let timestamp = SystemTime::from(*timestamp);
        let configuration = RaftConfiguration::try_from(configuration.clone().unwrap_or_default())?;

        DqliteDir::creator(path)
            .with_snapshot(move |s| {
                s.with_term(*term)
                    .with_index(*index)
                    .with_timestamp(timestamp)
                    .with_configuration(configuration)
                    .add_database(
                        CString::new("placeholder db".as_bytes().to_owned())
                            .expect("internal error: CString invalid"),
                        PlaceholderDb,
                    )
            })
            .create()?;

        ctx.shell = Shell::default();
        ctx.prompt = Prompt::default();

        Ok(())
    }
}

struct PlaceholderDb;

impl PlaceholderDb {
    const MAIN_CONTENT: &str = "placeholder main";
    const WAL_CONTENT: &str = "placeholder wal";
}

impl DqliteDatabaseWriter for PlaceholderDb {
    fn main_size(&self) -> usize {
        Self::MAIN_CONTENT.len()
    }

    fn wal_size(&self) -> usize {
        Self::WAL_CONTENT.len()
    }

    fn write_main(&self, out: &mut impl Write) -> Result<()> {
        write!(out, "{}", Self::MAIN_CONTENT)?;
        Ok(())
    }

    fn write_wal(&self, out: &mut impl Write) -> Result<()> {
        write!(out, "{}", Self::WAL_CONTENT)?;
        Ok(())
    }
}
