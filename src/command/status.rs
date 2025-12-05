use anyhow::Result;
use indoc::eprintdoc;

use crate::Context;
use crate::command::help::Help;
use crate::dqlite::DqliteSegment;

use super::UnrecognizedArgumentsError;

#[derive(Debug)]
pub(crate) struct StatusCommand;

impl StatusCommand {
    pub(crate) const SUMMARY: &'static str = "Show brief summary of the current Raft state";

    pub(crate) fn help() -> Help {
        Help::builder()
            .name("status")
            .summary(Self::SUMMARY)
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

        let last_closed_index = dqlite
            .closed_segments()
            .last()
            .map(|segment| match segment {
                DqliteSegment::Closed { indexes, .. } => indexes.end(),
                _ => unreachable!(),
            })
            .cloned();
        let num_entries_in_open_segments = dqlite
            .open_segments()
            .iter()
            .map(|segment| segment.entries().map(|entries| entries.len()))
            .sum::<Result<usize>>()? as u64;
        let last_index = last_closed_index.unwrap_or(first_index) + num_entries_in_open_segments;

        let dir_path = dqlite.path().display();
        let term = dqlite.term();
        eprintdoc!(
            "
                dir: {dir_path}
                term: {term}
                current_index: {last_index}
                first_index: {first_index}
            "
        );
        Ok(())
    }
}
