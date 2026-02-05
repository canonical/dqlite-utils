use anyhow::{Result, anyhow};

use crate::{Context, Shell, command::{Help, UnrecognizedArgumentsError}};


#[derive(Debug)]
pub struct DatabasesCommand {}

impl DatabasesCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".databases")
            .summary("list databases available in the open shell")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        if !args.is_empty() {
            return Err(UnrecognizedArgumentsError(args.to_vec()).into());
        }
        Ok(Self {})
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let shell = match &ctx.shell {
            Shell::Open(shell) => shell,
            _ => return Err(anyhow!(".databases command can only be used in open shell")),
        };

        println!("name  raft_last_update");
        println!("---");
        let connection = shell.connection();
        for db in shell.databases()? {
            let last_raft_index: String =
                connection
                    .pragma_query_value(Some(db.as_str()), "raft_last_update", |v| v.get(0))?;
            println!("{db}  {last_raft_index}");
        }

        Ok(())
    }
}
