use anyhow::Result;
use indoc::eprintdoc;

use crate::Context;
use crate::command::help::Help;
use crate::dqlite::DqliteSegment;

use super::UnrecognizedArgumentsError;

#[derive(Debug)]
pub(crate) struct StatusCommand;

impl StatusCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".status")
            .summary("Show brief summary of the current Raft state")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(self, ctx: &Context) -> Result<()> {
        let dqlite = ctx.dqlite()?;
        let first_index = dqlite.first_index();
        let current_index = dqlite.current_index()?;
        let dir_path = dqlite.path().display();
        let term = dqlite.term();
        eprintdoc!(
            "
                dir: {dir_path}
                term: {term}
                current_index: {current_index}
                first_index: {first_index}
            "
        );
        Ok(())
    }
}
