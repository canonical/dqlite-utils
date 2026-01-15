use std::fmt;

use anyhow::{Context as _, anyhow};
use indoc::indoc;
use indoc::printdoc;
use rusqlite::Error as RusqliteError;
use time::UtcDateTime;
use time::format_description::well_known::Iso8601;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
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
        let (term, index, timestamp) = conn.query_one(
            indoc! {"
                SELECT term, idx, timestamp
                FROM raft_data;
            "},
            (),
            |row| {
                let term = row.get_ref("term")?.as_i64()? as u64;
                let index = row.get_ref("idx")?.as_i64()? as u64;
                let timestamp =
                    UtcDateTime::parse(row.get_ref("timestamp")?.as_str()?, &Iso8601::DEFAULT)
                        .context("cannot parse timestamp")
                        .map_err(|err| RusqliteError::UserFunctionError(err.into()))?;
                Ok((term, index, timestamp))
            },
        )?;

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
            let mut stmt = conn.prepare(indoc! {"
                SELECT id, address, role
                FROM raft_servers;
            "})?;
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
