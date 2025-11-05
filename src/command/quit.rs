use std::process;

use crate::command::UnrecognisedArgumentsError;
use crate::{Context, Result};

#[derive(Debug, Default)]
pub(crate) struct QuitCommand;

impl QuitCommand {
    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognisedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(&self, _ctx: &Context) -> Result<()> {
        process::exit(0);
    }
}
