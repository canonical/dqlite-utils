use anyhow::{Context as _, anyhow};

use crate::{
    Context, Result,
    command::{MissingArgumentError, UnrecognizedArgumentsError},
};

pub(crate) struct SetTermCommand {
    term: u64,
}

impl SetTermCommand {
    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let term = match args {
            [] => return Err(MissingArgumentError("term").into()),
            [term] => term,
            [_, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let term = term
            .parse()
            .with_context(|| anyhow!("cannot parse term {term}"))?;
        Ok(Self { term })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { term } = self;
        let shell = ctx.shell.snapshot_mut().ok_or_else(|| {
            anyhow!("internal error: finish command not called in snapshot shell")
        })?;
        shell.builder.update(|builder| builder.with_term(term));
        Ok(())
    }
}
