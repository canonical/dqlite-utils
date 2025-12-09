use std::ffi::CString;
use std::time::SystemTime;

use anyhow::anyhow;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::command::snapshot::{ShellSnapshotContext, SnapshotShell};
use crate::dqlite::DqliteDir;
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
        let configuration = configuration.clone().unwrap_or_default();

        // DqliteDir::creator(path)
        //     .with_snapshot(move |s| {
        //         s.with_term(*term)
        //             .with_index(*index)
        //             .with_timestamp(timestamp)
        //             .with_configuration(configuration.to_owned().into())
        //     })
        //     .create()?;

        ctx.shell = Shell::default();
        ctx.prompt = Prompt::default();

        Ok(())
    }
}
