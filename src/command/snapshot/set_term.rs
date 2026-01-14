use anyhow::{Context as _, anyhow};
use indoc::indoc;

use crate::{
    Context, Result,
    command::{MissingArgumentError, UnrecognizedArgumentsError, help::Help},
};

#[derive(Debug)]
pub(crate) struct SetTermCommand {
    term: u64,
}

impl SetTermCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".set-term")
            .summary("set the term of the snapshot")
            .build()
            .expect("internal error: help invalid")
    }

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
        let shell = ctx.shell.snapshot().ok_or_else(|| {
            anyhow!("internal error: .set-term command not called in snapshot shell")
        })?;
        shell.connection().execute(
            indoc! {"
                UPDATE raft_data
                SET term = ?
            "},
            (term,),
        )?;
        Ok(())
    }
}
