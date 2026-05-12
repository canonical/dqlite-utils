use anyhow::{Context as _, Result};
use indoc::printdoc;

use crate::Context;
use crate::command::help::Help;
use crate::dqlite::{DqliteDir, DqliteLogEntryContent, RaftConfiguration, RaftRole, RaftServer};

use super::UnrecognizedArgumentsError;

#[derive(Debug)]
pub(crate) struct ConfigCommand {
    raw: bool,
}

impl ConfigCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".config")
            .summary("Show the current dqlite configuration")
            .add_flag("--raw", "print config in go-dqlite cluster.yaml format")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let raw = match args {
            [] => false,
            [flag] if flag == "--raw" => true,
            args => {
                return Err(UnrecognizedArgumentsError(args.to_vec()).into());
            }
        };
        Ok(Self { raw })
    }

    pub(crate) fn run(self, ctx: &Context) -> Result<()> {
        let Self { raw } = self;
        let dqlite = ctx.dqlite()?;
        let configuration = Self::current_configuration(dqlite)?;

        for server in &configuration.servers {
            let RaftServer { id, address, role } = server;
            let role = if raw {
                (*role as u8).to_string()
            } else {
                Self::role_name(role).to_string()
            };
            printdoc!(
                "
                    - ID: {id}
                      Address: {address}
                      Role: {role}
                "
            );
        }

        Ok(())
    }

    fn current_configuration(dqlite: &DqliteDir) -> Result<RaftConfiguration> {
        for segment in dqlite.segments().iter().rev() {
            let entries = segment.entries()?;
            for entry in entries.iter().rev() {
                if let DqliteLogEntryContent::Change(configuration) = &entry.content {
                    return Ok(configuration.clone());
                }
            }
        }

        dqlite
            .snapshots()
            .last()
            .map(|snapshot| snapshot.configuration.clone())
            .with_context(|| "cannot find configuration")
    }

    fn role_name(role: &RaftRole) -> &'static str {
        match role {
            RaftRole::Standby => "standby",
            RaftRole::Voter => "voter",
            RaftRole::Spare => "spare",
        }
    }
}
