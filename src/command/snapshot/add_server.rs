use anyhow::anyhow;

use crate::{
    Context, Result,
    command::{MissingArgumentError, UnrecognizedArgumentsError, help::Help},
    dqlite::{RaftRole, RaftServer},
};

pub(crate) struct AddServerCommand {
    role: RaftRole,
    id: u64,
    address: String,
}

impl AddServerCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".add-server")
            .summary("add a server to the snapshot")
            .add_arg(
                "role",
                "the role of the new server (standby, voter or spare)",
            )
            .add_arg("id", "the raft ID of the server")
            .add_arg("address", "the server's address")
            .build()
            .expect("internal error: help invalid")
    }

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

        if let Some(configuration) = &shell.snapshot.configuration {
            if configuration.servers.iter().any(|s| s.id == id) {
                return Err(anyhow!("cannot add server with id {id}: already used"));
            }
            if configuration.servers.iter().any(|s| s.address == address) {
                return Err(anyhow!(
                    "cannot add server with address '{address}': already used"
                ));
            }
        }

        let server = RaftServer { id, address, role };
        shell
            .snapshot
            .configuration
            .get_or_insert_default()
            .servers
            .push(server);
        Ok(())
    }
}
