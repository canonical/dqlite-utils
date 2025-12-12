use anyhow::{Context as _, anyhow};

use crate::{
    Context, Result,
    command::{MissingArgumentError, UnrecognizedArgumentsError, help::Help},
};

#[derive(Debug)]
pub(crate) struct SetIndexCommand {
    index: u64,
}

impl SetIndexCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".set-index")
            .summary("set the index of the snapshot")
            .add_arg("index", "the new index")
            .build()
            .expect("internal error: help invalid")
    }

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
            anyhow!("internal error: .set_index command not called in snapshot shell")
        })?;
        shell.snapshot.index = index;
        Ok(())
    }
}
