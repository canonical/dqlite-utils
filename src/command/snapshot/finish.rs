use anyhow::anyhow;

use crate::command::UnrecognizedArgumentsError;
use crate::command::snapshot::SnapshotShell;
use crate::prompt::Prompt;
use crate::{Context, Result, Shell};

pub(crate) struct FinishCommand;

impl FinishCommand {
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
        let SnapshotShell { path, builder: _ } = shell;
        println!("writing snapshot to {}...", path.display());

        ctx.shell = Shell::default();
        ctx.prompt = Prompt::default();

        Ok(())
    }
}
