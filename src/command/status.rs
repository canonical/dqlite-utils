use anyhow::Result;
use indoc::eprintdoc;

use crate::dqlite::DqliteSegment;
use crate::{Context, DqliteContext};

use super::UnrecognizedArgumentsError;

#[derive(Debug)]
pub(crate) struct StatusCommand;

impl StatusCommand {
    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(self, ctx: &Context) -> Result<()> {
        let DqliteContext { path, dir, .. } = ctx.dqlite()?;
        let first_index = dir.first_index();

        let last_closed_index = dir
            .closed_segments()
            .last()
            .map(|segment| match segment {
                DqliteSegment::Closed { indexes, .. } => indexes.end(),
                _ => unreachable!(),
            })
            .cloned();
        let num_entries_in_open_segments = dir
            .open_segments()
            .iter()
            .map(|segment| segment.entries().map(|entries| entries.len()))
            .sum::<Result<usize>>()? as u64;
        let last_index = last_closed_index.unwrap_or(first_index) + num_entries_in_open_segments;

        let dir_path = path.display();
        let term = dir.term();
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
