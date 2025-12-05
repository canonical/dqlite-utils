use anyhow::{Context as _, anyhow};

use crate::{
    Context, Result,
    command::{MissingArgumentError, UnrecognizedArgumentsError},
};

pub(crate) struct SetIndexCommand {
    index: u64,
}

impl SetIndexCommand {
    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let index = match args {
            [] => return Err(MissingArgumentError("index").into()),
            [index] => index,
            [_, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let index = index
            .parse()
            .with_context(|| anyhow!("cannot parse index {index}"))?;
        Ok(Self { index })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { index } = self;
        let shell = ctx.shell.snapshot_mut().ok_or_else(|| {
            anyhow!("internal error: finish command not called in snapshot shell")
        })?;
        shell.snapshot.index = index;
        Ok(())
    }
}
