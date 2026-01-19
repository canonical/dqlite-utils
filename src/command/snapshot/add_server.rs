use anyhow::anyhow;
use libsqlite3_sys as sqlite3;
use rusqlite::{ErrorCode, named_params};

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
        let res = shell.connection().execute(
            "
                INSERT INTO servers (id, address, role)
                VALUES (:id, :address, :role);
            ",
            named_params! {
                ":id": id,
                ":address": address,
                ":role": role,
            },
        );
        match res {
            Ok(1) => Ok(()),
            Ok(rows_affected) => Err(anyhow!(
                "internal error: servers insertion affected {rows_affected} rows"
            )),
            Err(rusqlite::Error::SqliteFailure(
                sqlite3::Error {
                    code: ErrorCode::ConstraintViolation,
                    extended_code: sqlite3::SQLITE_CONSTRAINT_PRIMARYKEY,
                },
                _,
            )) => {
                // Assumes `id` is the only primary key.
                Err(anyhow!("id already in use"))
            }
            Err(rusqlite::Error::SqliteFailure(
                sqlite3::Error {
                    code: ErrorCode::ConstraintViolation,
                    extended_code: sqlite3::SQLITE_CONSTRAINT_UNIQUE,
                },
                _,
            )) => {
                // Assumes `address` is the only unique, non-primary key.
                Err(anyhow!("address already in use"))
            }
            Err(err) => Err(err.into()),
        }
    }
}
