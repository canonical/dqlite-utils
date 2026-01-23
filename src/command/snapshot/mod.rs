mod abort;
mod add_server;
mod finish;
mod info;
mod set_index;
mod set_term;
mod set_timestamp;

use std::str::FromStr;

use anyhow::Context as _;
use fallible_iterator::FallibleIterator;
use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use strum::EnumIter;
use time::UtcDateTime;

use crate::command::help::{Help, HelpCommand};
use crate::command::{UnknownCommand, UnrecognizedArgumentsError};
use crate::dqlite::{RaftRole, RaftServer};
use crate::prompt::Prompt;
use crate::rusqlite_ext::config::ConnectionConfigExt;
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

const SCHEMA: &str = "
    CREATE TABLE metadata (
        raft_term INTEGER NOT NULL,
        raft_index INTEGER NOT NULL,
        timestamp INTEGER NOT NULL,
        pretty_timestamp TEXT AS (strftime('%FT%T', timestamp, 'unixepoch')),
        CHECK (rowid = 1)
    ) STRICT;

    CREATE TABLE servers (
        id INTEGER NOT NULL PRIMARY KEY,
        address TEXT NOT NULL UNIQUE,
        role INTEGER NOT NULL CHECK (role IN (0, 1, 2)),
        role_name TEXT AS (CASE
            WHEN role = 0 THEN 'Standby'
            WHEN role = 1 THEN 'Voter'
            WHEN role = 2 THEN 'Spare'
            ELSE 'Unknown'
        END)
    ) STRICT;

    INSERT INTO metadata (raft_term, raft_index, timestamp)
    VALUES (1, 1, unixepoch('now'));
";

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
            .add_command(HelpCommand::help())
            .add_command(InfoCommand::help())
            .add_command(SetIndexCommand::help())
            .add_command(SetTermCommand::help())
            .add_command(SetTimestampCommand::help())
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn new() -> Result<Self> {
        let prompt = Prompt::new("snapshot");
        let connection = Self::open_connection()?;
        Ok(Self { prompt, connection })
    }

    fn open_connection() -> Result<Connection> {
        let ret = Connection::open_in_memory()
            .context("internal error: cannot open connection to in-memory database")?;
        ret.set_main_name(c"raft");
        ret.execute_batch(SCHEMA)
            .context("internal error: cannot create raft_data table")?;
        ret.authorizer(Some(Self::authorizer))?;
        Ok(ret)
    }

    fn authorizer(ctx: AuthContext<'_>) -> Authorization {
        use AuthAction as Aa;

        let AuthContext {
            action,
            database_name,
            accessor: _,
        } = ctx;

        // All-schema rules
        match action {
            Aa::Attach { filename } => {
                let filename = filename.strip_prefix("file:").unwrap_or(filename);
                let filename = filename
                    .split_once('?')
                    .map(|(filename, _)| filename)
                    .unwrap_or(filename);
                if matches!(filename, "" | ":memory:") {
                    return Authorization::Deny;
                }
            }
            Aa::Unknown { .. } => return Authorization::Deny,
            _ => {}
        }

        // Per-schema rules
        if database_name.is_none_or(|name| name == "raft") {
            match action {
                Aa::AlterTable { .. }
                | Aa::CreateIndex { .. }
                | Aa::CreateTable { .. }
                | Aa::CreateTrigger { .. }
                | Aa::CreateView { .. }
                | Aa::CreateVtable { .. }
                | Aa::Delete {
                    table_name: "metadata",
                }
                | Aa::DropIndex { .. }
                | Aa::DropTable { .. }
                | Aa::DropTrigger { .. }
                | Aa::DropView { .. }
                | Aa::DropVtable { .. }
                | Aa::Insert {
                    table_name: "metadata",
                }
                | Aa::Pragma {
                    pragma_name: "writable_schema",
                    pragma_value: Some(_),
                }
                | Aa::Reindex { .. } => return Authorization::Deny,
                _ => {}
            }
        }

        Authorization::Allow
    }

    pub(crate) fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
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

    pub(crate) fn try_from_input(kind: SnapshotShellCommandKind, args: &[String]) -> Result<Self> {
        use SnapshotShellCommandKind as Ssck;
        match kind {
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

pub(crate) struct RaftMetadata {
    pub(crate) term: u64,
    pub(crate) index: u64,
    pub(crate) timestamp: UtcDateTime,
}

impl RaftMetadata {
    pub(crate) fn read_from(conn: &Connection) -> Result<Self> {
        let ret = conn.query_one(
            "
                SELECT raft_term, raft_index, timestamp
                FROM metadata
            ",
            (),
            |row| {
                Ok(Self {
                    term: row.get::<_, i64>("raft_term")? as u64,
                    index: row.get::<_, i64>("raft_index")? as u64,
                    timestamp: UtcDateTime::from_unix_timestamp(row.get("timestamp")?)
                        .map_err(|err| rusqlite::Error::UserFunctionError(err.into()))?,
                })
            },
        )?;
        Ok(ret)
    }
}

pub(crate) struct RaftServers {
    pub(crate) servers: Vec<RaftServer>,
}

impl RaftServers {
    pub(crate) fn read_from(conn: &Connection) -> Result<Self> {
        let mut stmt = conn.prepare(
            "
                SELECT id, address, role
                FROM servers;
            ",
        )?;
        let servers: Vec<_> = stmt
            .query(())?
            .map(|row| {
                Ok(RaftServer {
                    id: row.get::<_, i64>("id")? as u64,
                    address: row.get("address")?,
                    role: RaftRole::try_from(row.get::<_, u8>("role")?)
                        .map_err(|err| rusqlite::Error::UserFunctionError(err.into()))?,
                })
            })
            .collect()?;
        Ok(Self { servers })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use googletest::expect_that;
    use googletest::matchers::{anything, contains_substring, displays_as, err, ok};
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

    #[googletest::test]
    fn test_authorizer() {
        let tempdir = tempfile::tempdir().unwrap();
        let dummy_db_name = tempdir.path().join("some.db");

        // All-schema rule tests.
        Test::new("attach").allow(format!(
            "ATTACH DATABASE {:?} AS foo;",
            dummy_db_name.display()
        ));
        Test::new("create-table").allow(format!(
            "
                ATTACH DATABASE {:?} AS foo;
                CREATE TABLE foo.bar (name TEXT);
            ",
            dummy_db_name.display(),
        ));
        Test::new("attach-temp").deny("ATTACH DATABASE '' AS tmp;");
        Test::new("attach-temp-with-param").deny("ATTACH DATABASE '?k=v' AS tmp;");
        Test::new("attach-temp-with-prefix").deny("ATTACH DATABASE 'file:' AS tmp;");
        Test::new("attach-temp-with-prefix-and-param").deny("ATTACH DATABASE 'file:?k=v' AS tmp;");
        Test::new("attach-in-memory").deny("ATTACH DATABASE ':memory:' AS mem;");
        Test::new("attach-in-memory-param").deny("ATTACH DATABASE ':memory:?k=v' AS mem;");
        Test::new("attach-in-memory-with-prefix").deny("ATTACH DATABASE 'file::memory:' AS mem;");
        Test::new("attach-in-memory-with-prefix-and-param")
            .deny("ATTACH DATABASE 'file::memory:?k=v' AS mem;");

        // Raft schema tests.
        Test::new("select").allow("SELECT * FROM metadata;");
        Test::new("alter-table").deny(
            "
                ALTER TABLE metadata
                ADD COLUMN interloper TEXT;
            ",
        );
        Test::new("create-index").deny("CREATE INDEX interloper ON metadata (timestamp);");
        Test::new("create-table").deny("CREATE TABLE interloper (name TEXT);");
        Test::new("create-trigger").deny(
            "
                CREATE TRIGGER interloper
                BEFORE UPDATE ON metadata
                BEGIN
                    SELECT * FROM metadata;
                END;
            ",
        );
        Test::new("create-view").deny(
            "
                CREATE VIEW interloper
                AS SELECT * FROM metadata;
            ",
        );
        Test::new("create-virtual-table").deny(
            "
                CREATE VIRTUAL TABLE interloper
                USING dbstat;
            ",
        );
        Test::new("delete").deny("DELETE FROM metadata;");
        Test::new("drop-index")
            .setup("CREATE INDEX interloper ON metadata (timestamp);")
            .deny("DROP INDEX interloper;");
        Test::new("drop-table")
            .setup("CREATE TABLE interloper (name TEXT);")
            .deny("DROP TABLE interloper;");
        Test::new("drop-trigger")
            .setup(
                "
                    CREATE TRIGGER interloper
                    BEFORE UPDATE ON metadata
                    BEGIN
                        SELECT * FROM metadata;
                    END;
                ",
            )
            .deny("DROP TRIGGER interloper;");
        Test::new("drop-view")
            .setup(
                "
                    CREATE VIEW interloper
                    AS SELECT * FROM metadata;
                ",
            )
            .deny("DROP VIEW interloper;");
        Test::new("drop-vtable")
            .setup(
                "
                    CREATE VIRTUAL TABLE interloper
                    USING dbstat;
                ",
            )
            .deny("DROP TABLE interloper;");
        Test::new("insert").deny("INSERT INTO metadata (timestamp) VALUES (1);");
        Test::new("pragma-writabl_schema").deny("PRAGMA writable_schema = 1;");
        Test::new("reindex")
            .setup("CREATE INDEX interloper ON metadata (timestamp);")
            .deny("REINDEX interloper;");

        // Test types.
        struct Test {
            name: &'static str,
            setup: Option<&'static str>,
            conn: Connection,
        }

        impl Test {
            fn new(name: &'static str) -> Self {
                let setup = None;
                let conn = SnapshotShell::open_connection().unwrap();
                Self { name, setup, conn }
            }

            fn setup(mut self, sql: &'static str) -> Self {
                self.setup = Some(sql);
                self
            }

            fn allow(self, sql: impl AsRef<str>) {
                let sql = sql.as_ref();
                let txn = self.conn.unchecked_transaction().unwrap();
                expect_that!(self.run(sql), ok(anything()));
                txn.rollback().unwrap();
            }

            fn deny(self, sql: impl AsRef<str>) {
                let sql = sql.as_ref();
                let txn = self.conn.unchecked_transaction().unwrap();
                expect_that!(
                    self.run(sql),
                    err(displays_as(contains_substring("not authorized")))
                );
                txn.rollback().unwrap();
            }

            fn run(&self, sql: &str) -> Result<()> {
                let Self { name, setup, conn } = self;
                println!("Summary: {name}");

                if let Some(setup) = setup {
                    SnapshotShell::execute_authorizer(conn, setup).unwrap();
                }

                conn.execute_batch(sql)?;
                Ok(())
            }
        }
    }

    impl SnapshotShell {
        fn execute_authorizer(conn: &Connection, sql: &str) -> Result<()> {
            conn.authorizer::<fn(AuthContext<'_>) -> _>(None).unwrap();
            let ret = conn.execute_batch(sql);
            conn.authorizer(Some(Self::authorizer)).unwrap();
            ret?;
            Ok(())
        }
    }
}
