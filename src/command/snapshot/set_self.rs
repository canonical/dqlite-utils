use anyhow::{Context as _, anyhow};
use libsqlite3_sys as sqlite3;
use rusqlite::{ErrorCode, named_params};

use crate::command::help::Help;
use crate::command::{MissingArgumentError, UnrecognizedArgumentsError};
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct SetSelfCommand {
    id: u64,
}

impl SetSelfCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".set-self")
            .summary("set the self server in the snapshot")
            .add_arg("id", "the ID of the server to mark as self")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let id = match args {
            [] => return Err(MissingArgumentError("id").into()),
            [id] => id,
            [_, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let id = id
            .parse()
            .with_context(|| anyhow!("cannot parse id {id}"))?;
        Ok(Self { id })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { id } = self;
        let shell = ctx.shell.snapshot().ok_or_else(|| {
            anyhow!("internal error: .set-self command not called in snapshot shell")
        })?;
        let conn = shell.connection();
        let res = conn.execute(
            "
                UPDATE metadata
                SET self = :id
            ",
            named_params! {
                ":id": id as i64,
            },
        );
        match res {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(
                sqlite3::Error {
                    code: ErrorCode::ConstraintViolation,
                    extended_code: sqlite3::SQLITE_CONSTRAINT_FOREIGNKEY,
                },
                _,
            )) => Err(anyhow!("no server with id {id}")),
            Err(err) => Err(err.into()),
        }
    }
}
