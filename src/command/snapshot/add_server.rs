use anyhow::anyhow;
use libsqlite3_sys as sqlite3;
use rusqlite::{ErrorCode, named_params};

use crate::command::help::Help;
use crate::command::{MissingArgumentError, UnrecognizedArgumentsError};
use crate::dqlite::RaftRole;
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct AddServerCommand {
    address: String,
    role: RaftRole,
    id: Option<u64>,
}

impl AddServerCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".add-server")
            .summary("add a server to the snapshot")
            .add_arg("address", "the server's address")
            .add_optional_arg(
                "role",
                "the role of the new server (standby, voter or spare, default: voter)",
            )
            .add_optional_arg("id", "the raft ID of the server, generated if unspecified")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let (address, role, id) = match args {
            [] => return Err(MissingArgumentError("address").into()),
            [address] => (address, None, None),
            [address, role] => (address, Some(role.to_lowercase()), None),
            [address, role, id] => (address, Some(role.to_lowercase()), Some(id)),
            [_, _, _, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let address = address.to_owned();
        let role = role
            .as_deref()
            .map(|role| match role {
                "standby" => Ok(RaftRole::Standby),
                "voter" => Ok(RaftRole::Voter),
                "spare" => Ok(RaftRole::Spare),
                _ => Err(anyhow!("cannot parse {role} as raft role")),
            })
            .transpose()?
            .unwrap_or(RaftRole::Voter);
        let id = id.map(|id| id.parse()).transpose()?;
        Ok(Self { address, role, id })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { address, role, id } = self;
        let shell = ctx.shell.snapshot().ok_or_else(|| {
            anyhow!("internal error: .add-server command not called in snapshot shell")
        })?;
        let res = shell.connection().execute(
            "
                INSERT INTO servers (id, address, role)
                VALUES (:id, :address, :role);
            ",
            named_params! {
                ":id": id.map(|id| id as i64),
                ":address": address,
                ":role": role as u8,
            },
        );
        match res {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(
                sqlite3::Error {
                    code: ErrorCode::ConstraintViolation,
                    extended_code: sqlite3::SQLITE_CONSTRAINT_PRIMARYKEY,
                },
                _,
            )) => Err(anyhow!("id already in use")),
            Err(rusqlite::Error::SqliteFailure(
                sqlite3::Error {
                    code: ErrorCode::ConstraintViolation,
                    extended_code: sqlite3::SQLITE_CONSTRAINT_UNIQUE,
                },
                _,
            )) => {
                // Assumes `address` is the only UNIQUE non-PRIMARY key.
                Err(anyhow!("address already in use"))
            }
            Err(err) => Err(err.into()),
        }
    }
}
