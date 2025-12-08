use anyhow::anyhow;
use indoc::printdoc;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::command::snapshot::SnapshotShell;
use crate::{Context, Result};

pub(crate) struct InfoCommand;

impl InfoCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("info")
            .summary("show info about the current snapshot")
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
        let shell = ctx.shell.snapshot_mut().ok_or_else(|| {
            anyhow!("internal error: finish command not called in snapshot shell")
        })?;
        let SnapshotShell { path, snapshot } = shell;
        let dir_path = path.display();
        printdoc! {r#"
            dir: {dir_path}
            {snapshot}"#
        };
        Ok(())
    }
}
