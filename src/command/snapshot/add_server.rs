use anyhow::anyhow;
use rusqlite::named_params;

use crate::command::help::Help;
use crate::command::{MissingArgumentError, UnrecognizedArgumentsError};
use crate::dqlite::RaftRole;
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct AddServerCommand {
    // TODO(kcza): make this optional.
    id: u64,
    address: String,
    role: RaftRole,
}

impl AddServerCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".add-server")
            .summary("add a server to the snapshot")
            .add_arg("id", "the raft ID of the server")
            .add_arg("address", "the server's address")
            .add_optional_arg(
                "role",
                "the role of the new server (standby, voter or spare, default: voter)",
            )
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let (id, address, role) = match args {
            [] => return Err(MissingArgumentError("id").into()),
            [_] => return Err(MissingArgumentError("address").into()),
            [id, address] => (id, address, None),
            [id, address, role] => (id, address, Some(role.to_lowercase())),
            [_, _, _, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let id = id.parse()?;
        let address = address.to_owned();
        let role = role
            .as_deref()
            .map(|role| role.parse())
            .transpose()?
            .unwrap_or(RaftRole::Voter);
        Ok(Self { id, address, role })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { id, address, role } = self;
        let shell = ctx.shell.snapshot().ok_or_else(|| {
            anyhow!("internal error: .add-server command not called in snapshot shell")
        })?;
        let rows_affected = shell.connection().execute(
            "
                INSERT INTO raft.servers (id, address, role)
                VALUES (:id, :address, :role);
            ",
            named_params! {
                ":id": id,
                ":address": address,
                ":role": role,
            },
        )?;
        if rows_affected != 1 {
            return Err(anyhow!(
                "internal error: raft.servers insertion affected {rows_affected} rows"
            ));
        }

        Ok(())
    }
}
