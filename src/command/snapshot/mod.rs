use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::prompt::Prompt;
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct SnapshotCommand;

impl SnapshotCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot")
            .summary("Enter snapshot-creation shell")
            .add_arg("dir", "the directory to save the snapshot into")
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
        let Self = self;
        ctx.prompt = Prompt::new("snapshot");
        Ok(())
    }
}
