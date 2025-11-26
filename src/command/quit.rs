use std::process;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::{Context, Result};

#[derive(Debug, Default)]
pub(crate) struct QuitCommand;

impl QuitCommand {
    pub(crate) const SUMMARY: &'static str = "Exit";

    pub(crate) fn help() -> Help {
        Help::builder().name("quit").summary(Self::SUMMARY).build()
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(&self, _ctx: &Context) -> ! {
        process::exit(0);
    }
}
