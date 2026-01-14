use std::fmt;

use anyhow::{Context as _, anyhow};
use indoc::indoc;
use indoc::printdoc;
use rusqlite::Error as RusqliteError;
use time::UtcDateTime;
use time::format_description::well_known::Iso8601;

use crate::command::UnrecognizedArgumentsError;
use crate::command::help::Help;
use crate::command::snapshot::{ShellSnapshotContext, ShellSnapshotRaftConfiguration};
use crate::dqlite::{RaftRole, RaftServer};
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

        let ShellSnapshotContext { configuration } = &shell.snapshot;
        let conn = shell.connection();
        let (term, index, timestamp) = conn.query_one(
            indoc! {"
                SELECT *
                FROM raft_data;
            "},
            (),
            |row| {
                let term: u64 = row.get_unwrap("term");
                let index: u64 = row.get_unwrap("idx");
                let timestamp = UtcDateTime::parse(
                    &row.get_unwrap::<_, String>("timestamp"),
                    &Iso8601::DEFAULT,
                )
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

        let ShellSnapshotRaftConfiguration { servers } = configuration;
        if servers.is_empty() {
            println!("configuration: -");
        } else {
            println!("configuration:");
            for server in servers {
                let RaftServer { id, address, role } = server;
                let pretty_role = match role {
                    RaftRole::Standby => "standby",
                    RaftRole::Voter => "voter",
                    RaftRole::Spare => "spare",
                };
                printdoc!(
                    "
                        - id: {id}
                          address: {address}
                          role: {pretty_role}
                    "
                );
            }
        }
        Ok(())
    }
}
