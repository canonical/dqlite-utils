use anyhow::Result;

use crate::{
    Context, Shell,
    command::{Help, UnrecognizedArgumentsError},
};

#[derive(Debug)]
pub struct CloseCommand;

impl CloseCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".close")
            .summary("exit the open shell and close the connection")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        Ok(Self)
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        ctx.shell = Shell::default();
        Ok(())
    }
}
