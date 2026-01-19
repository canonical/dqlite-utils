use std::fmt;

use anyhow::anyhow;
use indoc::printdoc;
use time::format_description::well_known::Iso8601;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::command::snapshot::finish::RaftMetadata;
use crate::dqlite::RaftServer;
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
        } = RaftMetadata::read_from(&conn)?;
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

        let servers = {
            let mut servers = vec![];
            let mut stmt = conn.prepare("
                SELECT id, address, role
                FROM raft.servers;
            ")?;
            let mut rows = stmt.query(())?;
            while let Some(row) = rows.next()? {
                let server = RaftServer {
                    id: row.get("id")?,
                    address: row.get("address")?,
                    role: row.get("role")?,
                };
                servers.push(server);
            }
            servers
        };
        if servers.is_empty() {
            println!("configuration: -");
        } else {
            println!("configuration:");
            for server in servers {
                let RaftServer { id, address, role } = server;
                printdoc!(
                    "
                        - id: {id}
                          address: {address}
                          role: {role}
                    "
                );
            }
        }
        Ok(())
    }
}
