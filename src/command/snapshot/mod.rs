mod abort;
mod add_server;
mod finish;
mod info;
mod set_index;
mod set_term;
mod set_timestamp;

use std::str::FromStr;

use anyhow::{Context as _, anyhow};
use indoc::indoc;
use rusqlite::{Connection, named_params};
use strum::EnumIter;
use time::UtcDateTime;
use time::format_description::well_known::Iso8601;

use crate::command::help::Help;
use crate::command::{UnknownCommand, UnrecognizedArgumentsError};
use crate::prompt::Prompt;
use crate::{Context, Error, Result, Shell};

use self::abort::AbortCommand;
use self::add_server::AddServerCommand;
use self::finish::FinishCommand;
use self::info::InfoCommand;
use self::set_index::SetIndexCommand;
use self::set_term::SetTermCommand;
use self::set_timestamp::SetTimestampCommand;

#[derive(Debug)]
pub(crate) struct SnapshotCommand;

impl SnapshotCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".snapshot")
            .summary("Enter snapshot-creation shell")
            .add_arg("dir", "the directory to save the snapshot into")
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
        let Self = self;
        ctx.shell = Shell::Snapshot(SnapshotShell::new()?);
        Ok(())
    }
}

#[derive(Debug)]
pub struct SnapshotShell {
    connection: Connection,
    prompt: Prompt,
}

impl SnapshotShell {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot shell")
            .summary("incrementally create a snapshot")
            .skip_usage()
            .add_command(AbortCommand::help())
            .add_command(AddServerCommand::help())
            .add_command(FinishCommand::help())
            .add_command(InfoCommand::help())
            .add_command(SetIndexCommand::help())
            .add_command(SetTermCommand::help())
            .add_command(SetTimestampCommand::help())
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn new() -> Result<Self> {
        let prompt = Prompt::new("snapshot");
        let connection = Self::prepare_connection()?;
        Ok(Self { prompt, connection })
    }

    fn prepare_connection() -> Result<Connection> {
        let ret = Connection::open_in_memory()
            .context("internal error: cannot open connection to in-memory database")?;

        ret.execute(
            indoc! {"
                CREATE TABLE raft_data (
                    term int,
                    idx int,
                    timestamp TEXT NOT NULL,
                    CHECK (rowid = 1)
                ) STRICT;
            "},
            (),
        )
        .context("internal error: cannot create raft_data table")?;

        ret.execute(
            indoc! {"
                CREATE TABLE raft_servers (
                    id int,
                    address TEXT NOT NULL,
                    role TEXT CHECK (role IN ('standby', 'voter', 'spare')),
                    UNIQUE (id, address, role)
                ) STRICT;
            "},
            (),
        )
        .context("internal error: cannot create raft_data table")?;

        let rows_affected = ret
            .execute(
                indoc! {"
                    INSERT INTO raft_data (term, idx, timestamp)
                    VALUES (:term, :idx, :timestamp);
                "},
                named_params! {
                    ":term": 1,
                    ":idx": 1,
                    ":timestamp": UtcDateTime::now().format(&Iso8601::DEFAULT)?,
                },
            )
            .context("internal error: cannot insert into raft_data")?;
        if rows_affected != 1 {
            return Err(anyhow!(
                "internal error: raft_data insertion affected {rows_affected} rows"
            ));
        }

        Ok(ret)
    }

    pub(crate) fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[derive(Debug)]
pub(crate) enum SnapshotShellCommand {
    Abort(AbortCommand),
    AddServer(AddServerCommand),
    Finish(FinishCommand),
    Info(InfoCommand),
    SetIndex(SetIndexCommand),
    SetTerm(SetTermCommand),
    SetTimestamp(SetTimestampCommand),
}

impl SnapshotShellCommand {
    pub(crate) fn kind(&self) -> SnapshotShellCommandKind {
        use SnapshotShellCommandKind as Ssck;
        match self {
            Self::Abort(_) => Ssck::Abort,
            Self::AddServer(_) => Ssck::AddServer,
            Self::Finish(_) => Ssck::Finish,
            Self::Info(_) => Ssck::Info,
            Self::SetIndex(_) => Ssck::SetIndex,
            Self::SetTerm(_) => Ssck::SetTerm,
            Self::SetTimestamp(_) => Ssck::SetTimestamp,
        }
    }

    pub(crate) fn try_from_input(command: &str, args: &[String]) -> Result<Self> {
        use SnapshotShellCommandKind as Ssck;
        match command.parse()? {
            Ssck::Abort => Ok(Self::Abort(AbortCommand::try_from_args(args)?)),
            Ssck::AddServer => Ok(Self::AddServer(AddServerCommand::try_from_args(args)?)),
            Ssck::Finish => Ok(Self::Finish(FinishCommand::try_from_args(args)?)),
            Ssck::Info => Ok(Self::Info(InfoCommand::try_from_args(args)?)),
            Ssck::SetIndex => Ok(Self::SetIndex(SetIndexCommand::try_from_args(args)?)),
            Ssck::SetTerm => Ok(Self::SetTerm(SetTermCommand::try_from_args(args)?)),
            Ssck::SetTimestamp => Ok(Self::SetTimestamp(SetTimestampCommand::try_from_args(
                args,
            )?)),
        }
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Abort(cmd) => cmd.run(ctx),
            Self::AddServer(cmd) => cmd.run(ctx),
            Self::Finish(cmd) => cmd.run(ctx),
            Self::Info(cmd) => cmd.run(ctx),
            Self::SetIndex(cmd) => cmd.run(ctx),
            Self::SetTerm(cmd) => cmd.run(ctx),
            Self::SetTimestamp(cmd) => cmd.run(ctx),
        }
    }
}

#[derive(Debug, EnumIter)]
pub(crate) enum SnapshotShellCommandKind {
    Abort,
    AddServer,
    Finish,
    Info,
    SetIndex,
    SetTerm,
    SetTimestamp,
}

impl SnapshotShellCommandKind {
    pub(crate) fn help(&self) -> Help {
        match self {
            Self::Abort => AbortCommand::help(),
            Self::AddServer => AddServerCommand::help(),
            Self::Finish => FinishCommand::help(),
            Self::Info => InfoCommand::help(),
            Self::SetIndex => SetIndexCommand::help(),
            Self::SetTerm => SetTermCommand::help(),
            Self::SetTimestamp => SetTimestampCommand::help(),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Abort => ".abort",
            Self::AddServer => ".abort",
            Self::Finish => ".finish",
            Self::Info => ".info",
            Self::SetIndex => ".set-index",
            Self::SetTerm => ".set-term",
            Self::SetTimestamp => ".set-timestamp",
        }
    }
}

impl FromStr for SnapshotShellCommandKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw {
            ".abort" => Ok(Self::Abort),
            ".add-server" => Ok(Self::AddServer),
            ".finish" => Ok(Self::Finish),
            ".info" => Ok(Self::Info),
            ".set-index" => Ok(Self::SetIndex),
            ".set-term" => Ok(Self::SetTerm),
            ".set-timestamp" => Ok(Self::SetTimestamp),
            _ => Err(UnknownCommand.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use googletest::expect_that;
    use googletest::matchers::contains_substring;
    use strum::IntoEnumIterator;

    use super::*;

    #[googletest::test]
    fn test_all_commands_listed_in_help() {
        let help_output = {
            let mut help_output = Cursor::new(Vec::new());
            SnapshotShell::help().write_to(&mut help_output).unwrap();
            String::try_from(help_output.into_inner()).unwrap()
        };
        for command_kind in SnapshotShellCommandKind::iter() {
            expect_that!(help_output, contains_substring(command_kind.name()));
        }
    }
}
