use std::fmt;

use anyhow::anyhow;
use indoc::printdoc;
use time::format_description::well_known::Iso8601;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::command::snapshot::{RaftMetadata, RaftServers};
use crate::dqlite::{RaftRole, RaftServer};
use crate::utils::AttachedSchemasConnectionExt;
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct InfoCommand;

impl InfoCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".info")
            .summary("show info about the current snapshot")
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
        let shell = ctx
            .shell
            .snapshot()
            .ok_or_else(|| anyhow!("internal error: .info command not called in snapshot shell"))?;

        let conn = shell.connection();

        let RaftMetadata {
            term,
            index,
            timestamp,
        } = RaftMetadata::read_from(conn)?;
        let timestamp = timestamp
            .format(&Iso8601::DEFAULT)
            .map_err(|_| fmt::Error)?;
        printdoc!(
            "
                term: {term}
                index: {index}
                timestamp: {timestamp}
            "
        );

        let RaftServers { servers } = RaftServers::read_from(conn)?;
        if servers.is_empty() {
            println!("configuration: -");
        } else {
            println!("configuration:");
            for server in servers {
                let RaftServer { id, address, role } = server;
                let role = match role {
                    RaftRole::Standby => "standby",
                    RaftRole::Voter => "voter",
                    RaftRole::Spare => "spare",
                };
                printdoc!(
                    "
                        - id: {id}
                          address: {address}
                          role: {role}
                    "
                );
            }
        }

        print!("attached_schemas: ");
        let mut schemas = conn.attached_schemas()?;
        let mut schemas_iter = schemas.try_iter()?;
        let mut schema = schemas_iter.next()?;
        let mut schema_printed = false;
        while let Some(curr_schema) = &schema {
            let name = curr_schema.name();
            if name == "raft" || name == "temp" {
                // `raft` only contains metadata, this does not appear as a schema name in the written snapshot. `temp` is ignored as it cannot be used as a schema name.
                schema = schemas_iter.next()?;
                continue;
            }

            let file = match curr_schema.file()? {
                "" => "-",
                file => file,
            };
            printdoc!(
                "

                    - name: {name}
                      path: {file}
                "
            );
            schema_printed = true;
            schema = schemas_iter.next()?;
        }
        if !schema_printed {
            println!("-");
        }

        Ok(())
    }
}
