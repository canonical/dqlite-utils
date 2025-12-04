use anyhow::anyhow;

use crate::{
    Context, Result,
    command::{MissingArgumentError, UnrecognizedArgumentsError},
    dqlite::{RaftRole, RaftServer},
};

pub(crate) struct AddServerCommand {
    role: RaftRole,
    id: u64,
    address: String,
}

impl AddServerCommand {
    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let (role, id, address) = match args {
            [] => return Err(MissingArgumentError("role").into()),
            [_] => return Err(MissingArgumentError("id").into()),
            [_, _] => return Err(MissingArgumentError("address").into()),
            [role, id, address] => (role, id, address),
            [_, _, _, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let role = role.parse()?;
        let id = id.parse()?;
        let address = address.to_owned();
        Ok(Self { role, id, address })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { role, id, address } = self;
        let shell = ctx.shell.snapshot_mut().ok_or_else(|| {
            anyhow!("internal error: add-server command not called in snapshot shell")
        })?;

        let server = RaftServer { id, address, role };
        shell.builder.set(shell.builder.take().add_server(server));
        Ok(())
    }
}
