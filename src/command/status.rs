use anyhow::Result;
use indoc::eprintdoc;

use crate::{Context, dqlite::DqliteSegment};

use super::UnrecognisedArgumentsError;

#[derive(Debug)]
pub(crate) struct StatusCommand;

impl StatusCommand {
    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognisedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(&self, ctx: &Context) -> Result<()> {
        let dqlite = &ctx.dqlite;
        let first_index = dqlite.first_index();

        let last_closed_index = dqlite
            .closed_segments()
            .last()
            .map(|segment| match segment {
                DqliteSegment::Closed { indexes, .. } => indexes.end(),
                _ => unreachable!(),
            })
            .cloned();
        let num_entries_in_open_segments: u64 = dqlite
            .open_segments()
            .iter()
            .map(|segment| match segment {
                DqliteSegment::Open { counter, .. } => counter,
                _ => unreachable!(),
            })
            .sum();
        let last_index = last_closed_index.unwrap_or(first_index) + num_entries_in_open_segments;

        let dir = ctx.dir.display();
        let term = dqlite.term();
        eprintdoc!(
            "
                dir: {dir}
                term: {term}
                current_index: {last_index}
                first_index: {first_index}
            "
        );
        Ok(())
    }
}
