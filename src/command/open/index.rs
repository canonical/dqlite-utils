use anyhow::{Result, anyhow};

use crate::{
    Context,
    command::{Help, UnrecognizedArgumentsError},
    prompt::Prompt,
};

#[derive(Debug)]
pub struct IndexCommand {
    index: u64,
}

impl IndexCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".index")
            .summary("set the raft index to query for all databases")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if args.len() != 1 {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        let index = args[0]
            .parse::<u64>()
            .map_err(|e| anyhow!("invalid index '{}': {}", args[0], e))?;
        Ok(Self { index })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        if ctx.shell.open().is_none() {
            return Err(anyhow!(".index command can only be used in open shell"));
        }

        let databases = {
            let state = ctx.open_state();
            let vfs = state.vfs().expect("internal error: unregistered VFS");
            vfs.set_current_index(self.index)?;
            vfs.databases()?
        };
        let shell = ctx.shell.open_mut().unwrap();
        shell.detach_databases()?;
        shell.attach_databases(databases.into_iter())?;
        shell.prompt = Prompt::new(format!("open@{}", self.index));

        Ok(())
    }
}
