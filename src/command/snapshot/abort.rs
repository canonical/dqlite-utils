use crate::{
    Context, Result, Shell,
    command::{UnrecognizedArgumentsError, help::Help},
    prompt::Prompt,
};

pub(crate) struct AbortCommand;

impl AbortCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".abort")
            .summary("exit the snapshot shell without writing to disk")
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
        ctx.prompt = Prompt::default();
        Ok(())
    }
}
