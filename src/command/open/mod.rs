mod close;
mod databases;
mod index;
mod vfs;

use std::{cell::OnceCell, fmt::Debug, str::FromStr};

use anyhow::{Context as _, Error, Result, anyhow};
use rusqlite::{
    Connection,
    hooks::{AuthContext, Authorization},
};

use self::vfs::DqliteVfs;
use crate::{
    Context, Shell,
    command::{
        Help, UnknownCommand, UnrecognizedArgumentsError,
        open::{close::CloseCommand, databases::DatabasesCommand, index::IndexCommand},
    },
    dqlite::DqliteDir,
    prompt::Prompt,
    rusqlite_ext::{
        config::ConnectionConfigExt,
        vfs::{VfsRegistration, VfsRegistrationGuard},
    },
};

#[derive(Default)]
pub struct OpenState {
    vfs_registration_guard: OnceCell<VfsRegistrationGuard<DqliteVfs>>,
}

impl OpenState {
    fn load(&self, dqlite: &DqliteDir, page_size: usize) -> Result<()> {
        // TODO: use `get_mut_or_init` when stabilized. See https://github.com/rust-lang/rust/issues/121641
        if self.vfs_registration_guard.get().is_some() {
            return Ok(());
        }

        let vfs = DqliteVfs::from_dir(dqlite, page_size)?;
        let guard = VfsRegistration::new(vfs)
            .make_default()
            .max_pathlen(256)
            .register("dqlite")?;
        self.vfs_registration_guard
            .set(guard)
            .map_err(|_| anyhow!("internal error: vfs already registered"))?;
        Ok(())
    }

    fn vfs(&self) -> Option<&DqliteVfs> {
        self.vfs_registration_guard.get().map(|r| r.vfs())
    }
}

impl Debug for OpenState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let vfs = self.vfs();
        f.debug_struct("OpenState").field("vfs", &vfs).finish()
    }
}

#[derive(Debug)]
pub(crate) struct OpenCommand {
    index: Option<u64>,
}

impl OpenCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name(".open")
            .summary("open a shell on a point-in-time dqlite state")
            .add_optional_arg(
                "index",
                "the index of the point-in-time state to open, or 'latest' (default)",
            )
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let index = match args {
            [] => None,
            [arg] if arg == "latest" => None,
            [arg] => {
                let index = arg
                    .parse::<u64>()
                    .map_err(|e| anyhow!("invalid index '{}': {}", arg, e))?;
                Some(index)
            }
            _ => {
                return Err(UnrecognizedArgumentsError(args.to_vec()).into());
            }
        };
        Ok(Self { index })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let shell = OpenShell::new(self.index)?;

        {
            let state = ctx.open_state();
            state.load(ctx.dqlite()?, 4096)?; // TODO get the page size from the snapshot
            if let Some(index) = self.index {
                state
                    .vfs()
                    .expect("internal error: unregistered VFS")
                    .set_current_index(index)?;
            }
            shell.attach_databases(state.vfs().unwrap().databases()?.into_iter())?;
        }

        ctx.shell = Shell::Open(shell);
        Ok(())
    }
}

#[derive(Debug)]
pub struct OpenShell {
    connection: Connection,
    prompt: Prompt,
}

impl OpenShell {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("open shell")
            .summary("query a point-in-time dqlite state")
            .add_command(CloseCommand::help())
            .add_command(DatabasesCommand::help())
            .add_command(IndexCommand::help())
            .skip_usage()
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn new(index: Option<u64>) -> Result<Self> {
        let connection = Self::open_connection()?;
        let prompt = if let Some(index) = index {
            Prompt::new(format!("open@{index}"))
        } else {
            Prompt::new("open@latest")
        };
        Ok(Self { connection, prompt })
    }

    pub(crate) fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    fn open_connection() -> Result<Connection> {
        let ret = Connection::open_in_memory()
            .context("internal error: cannot open connection to in-memory database")?;
        ret.set_main_name(c"raft");
        ret.authorizer(Some(Self::authorizer))?;
        // TODO: use a virtual table to access the snapshot/index metadata? Or just copy that here? Not sure...
        Ok(ret)
    }

    fn authorizer(_ctx: AuthContext<'_>) -> Authorization {
        Authorization::Allow // Allow everything for now
    }

    fn databases(&self) -> Result<Vec<String>> {
        let mut stmt = self.connection.prepare_cached(
            "
                SELECT name
                FROM pragma_database_list()
            ",
        )?;
        let mut result = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            if name == "temp" || name == "raft" {
                continue;
            }
            result.push(name);
        }
        Ok(result)
    }

    fn attach_databases(&self, databases: impl Iterator<Item = String>) -> Result<()> {
        for db in databases {
            self.connection.execute_batch(&format!(
                "
                    ATTACH DATABASE 'file:{db}?vfs=dqlite' AS '{db}'
                "
            ))?;
        }
        Ok(())
    }

    fn detach_databases(&self) -> Result<()> {
        for db in self.databases()? {
            if db == "raft" {
                continue;
            }
            self.connection.execute_batch(&format!(
                "
                    DETACH DATABASE '{db}'
                "
            ))?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum OpenShellCommand {
    Close(CloseCommand),
    Index(IndexCommand),
    Databases(DatabasesCommand),
}

impl OpenShellCommand {
    pub(crate) fn try_from_input(kind: OpenShellCommandKind, args: &[String]) -> Result<Self> {
        use OpenShellCommandKind as Osck;
        match kind {
            Osck::Close => Ok(Self::Close(CloseCommand::try_from_args(args)?)),
            Osck::Index => Ok(Self::Index(IndexCommand::try_from_args(args)?)),
            Osck::Databases => Ok(Self::Databases(DatabasesCommand::try_from_args(args)?)),
        }
    }

    pub(crate) fn kind(&self) -> OpenShellCommandKind {
        use OpenShellCommandKind as Osck;
        match self {
            Self::Close(_) => Osck::Close,
            Self::Index(_) => Osck::Index,
            Self::Databases(_) => Osck::Databases,
        }
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Close(cmd) => cmd.run(ctx),
            Self::Index(cmd) => cmd.run(ctx),
            Self::Databases(cmd) => cmd.run(ctx),
        }
    }
}

#[derive(Debug)]
pub(crate) enum OpenShellCommandKind {
    Close,
    Index,
    Databases,
}

impl OpenShellCommandKind {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Close => ".close",
            Self::Index => ".index",
            Self::Databases => ".databases",
        }
    }

    pub(crate) fn help(&self) -> Help {
        match self {
            Self::Close => CloseCommand::help(),
            Self::Index => IndexCommand::help(),
            Self::Databases => DatabasesCommand::help(),
        }
    }
}

impl FromStr for OpenShellCommandKind {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            ".close" => Ok(Self::Close),
            ".index" => Ok(Self::Index),
            ".databases" => Ok(Self::Databases),
            _ => Err(UnknownCommand.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        ffi::{CString, OsStr},
        io::Write,
        ops::{Range, RangeFrom, RangeTo},
        time::{Duration, SystemTime},
    };

    use anyhow::Result;
    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::{
        command::open::OpenState,
        dqlite::{
            DqliteDatabaseWriter, DqliteDir, DqliteFrame, DqliteLogEntry, DqliteLogEntryContent,
            DqliteSegmentBuilder, DqliteSnapshotBuilder, Empty, RaftConfiguration, RaftRole,
            RaftServer,
        },
        rusqlite_ext::files::{ConnectionFile, ConnectionFilesExt},
    };

    struct ConnectionWriter<'a> {
        main: RefCell<ConnectionFile<'a>>,
        page_size: usize,
    }

    impl<'a> ConnectionWriter<'a> {
        fn new(conn: &'a Connection, db: &str) -> Result<Self> {
            let main = conn.main_file(Some(OsStr::new(db)))?;
            let page_size: i64 = conn.pragma_query_value(Some(db), "page_size", |v| v.get(0))?;
            Ok(ConnectionWriter {
                main: RefCell::new(main),
                page_size: page_size as usize,
            })
        }
    }

    impl<'a> DqliteDatabaseWriter for ConnectionWriter<'a> {
        fn main_size(&self) -> Result<u64> {
            Ok(self.main.borrow_mut().len()? as u64)
        }

        fn wal_size(&self) -> Result<u64> {
            Ok(0) // No WAL.
        }

        fn write_main(&mut self, out: &mut impl Write) -> Result<()> {
            let mut main = self.main.borrow_mut();
            let main_size = main.len()? as usize;
            let mut buf = vec![0u8; self.page_size];

            for offset in (0..main_size).step_by(self.page_size) {
                main.read_at(&mut buf, offset as u64)?;
                out.write_all(&buf)?;
            }

            Ok(())
        }

        fn write_wal(&mut self, _out: &mut impl Write) -> Result<()> {
            Ok(())
        }
    }

    trait AddConnectionExt<'a> {
        fn add_connection(
            self,
            conn: &'a Connection,
            db: &'a str,
        ) -> DqliteSnapshotBuilder<ConnectionWriter<'a>>;
    }

    impl<'a> AddConnectionExt<'a> for DqliteSnapshotBuilder<Empty> {
        fn add_connection(
            self,
            conn: &'a Connection,
            db: &'a str,
        ) -> DqliteSnapshotBuilder<ConnectionWriter<'a>> {
            self.add_database(
                CString::new(db.as_bytes()).unwrap(),
                ConnectionWriter::new(conn, db).unwrap(),
            )
        }
    }

    impl<'a> AddConnectionExt<'a> for DqliteSnapshotBuilder<ConnectionWriter<'a>> {
        fn add_connection(
            self,
            conn: &'a Connection,
            db: &'a str,
        ) -> DqliteSnapshotBuilder<ConnectionWriter<'a>> {
            self.add_database(
                CString::new(db.as_bytes()).unwrap(),
                ConnectionWriter::new(conn, db).unwrap(),
            )
        }
    }

    trait WalSegment<R> {
        fn add_wal_segment(self, term: u64, conn: &Connection, db: &str, range: R) -> Self;
    }

    impl WalSegment<Range<u64>> for DqliteSegmentBuilder {
        fn add_wal_segment(
            self,
            term: u64,
            conn: &Connection,
            db: &str,
            range: Range<u64>,
        ) -> Self {
            let page_size: i64 = conn
                .pragma_query_value(Some(db), "page_size", |v| v.get(0))
                .unwrap();
            let mut wal = conn.journal_file(Some(db.as_ref())).unwrap().unwrap();
            let mut entries = Vec::new();
            let mut frames = Vec::new();
            for i in range {
                let offset = i * (page_size as u64 + 24) + 32;
                let mut buf = vec![0u8; page_size as usize];

                // Read the header first
                wal.read_at(&mut buf[..24], offset).unwrap();
                // Get the page number
                let page_number = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as u64;
                debug_assert!(page_number > 0);

                // Then check if this is a commit frame
                let is_commit = u32::from_be_bytes(buf[4..8].try_into().unwrap()) != 0;

                // Read the page
                wal.read_at(&mut buf, offset).unwrap();
                frames.push(DqliteFrame {
                    page_number,
                    data: buf.clone(),
                });

                if is_commit {
                    entries.push(DqliteLogEntry {
                        term,
                        content: DqliteLogEntryContent::CommandFrames {
                            filename: db.into(),
                            tx_id: 0,
                            truncate: 0,
                            is_commit: true,
                            frames: std::mem::take(&mut frames),
                        },
                    })
                }
            }
            self.add_batch(&entries)
        }
    }

    impl WalSegment<RangeFrom<u64>> for DqliteSegmentBuilder {
        fn add_wal_segment(
            self,
            term: u64,
            conn: &Connection,
            db: &str,
            range: RangeFrom<u64>,
        ) -> Self {
            let wal_size = conn
                .journal_file(Some(OsStr::new(db)))
                .unwrap()
                .expect("internal error: WAL file does not exist")
                .len()
                .unwrap();
            if wal_size <= 32 {
                return self; // Empty WAL
            }
            let page_size: i64 = conn
                .pragma_query_value(Some(db), "page_size", |v| v.get(0))
                .unwrap();
            let total_frames = (wal_size - 32) / (page_size as u64 + 24);
            self.add_wal_segment(term, conn, db, range.start..total_frames)
        }
    }

    impl WalSegment<RangeTo<u64>> for DqliteSegmentBuilder {
        fn add_wal_segment(
            self,
            term: u64,
            conn: &Connection,
            db: &str,
            range: RangeTo<u64>,
        ) -> Self {
            self.add_wal_segment(term, conn, db, 0..range.end)
        }
    }

    #[test]
    fn test_read_snapshot() -> Result<()> {
        const PAGE_SIZE: usize = 4096;
        let tempdir = tempdir()?;
        let tempfile = tempdir.path().join("test.sqlite");
        let mut conn = Connection::open(tempfile)?;

        conn.pragma_update(None, "page_size", PAGE_SIZE as i64)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        {
            let tx = conn.transaction()?;
            tx.execute_batch(
                "
                    CREATE TABLE IF NOT EXISTS users (
                        id INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        email TEXT NOT NULL UNIQUE,
                        created_at TEXT DEFAULT CURRENT_TIMESTAMP
                    );
                    CREATE TABLE IF NOT EXISTS products (
                        id INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        price INTEGER NOT NULL -- Storing cents to avoid floating point issues
                    );
                    CREATE TABLE IF NOT EXISTS orders (
                        id INTEGER PRIMARY KEY,
                        user_id INTEGER NOT NULL REFERENCES users(id),
                        created_at TEXT DEFAULT CURRENT_TIMESTAMP
                    );
                    CREATE TABLE IF NOT EXISTS order_items (
                        id INTEGER PRIMARY KEY,
                        order_id INTEGER NOT NULL REFERENCES orders(id),
                        product_id INTEGER NOT NULL REFERENCES products(id),
                        quantity INTEGER DEFAULT 1,
                        UNIQUE(order_id, product_id)
                    );

                    WITH RECURSIVE cnt(x) AS (
                        SELECT 1
                        UNION ALL
                        SELECT x+1 FROM cnt WHERE x < 10
                    )
                    INSERT INTO users (name, email)
                    SELECT 
                        'User ' || x, 
                        'user' || x || '@test.com' 
                    FROM cnt;

                    WITH RECURSIVE cnt(x) AS (
                        SELECT 1
                        UNION ALL
                        SELECT x+1 FROM cnt WHERE x < 5
                    )
                    INSERT INTO products (name, price)
                    SELECT 
                        'Widget ' || CHAR(64+x),
                        x * 1000
                    FROM cnt
                ",
            )?;

            tx.commit()?;
        }
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        conn.execute_batch(
            "
                DELETE FROM products;
                DELETE FROM users;
                VACUUM;
            ",
        )?;

        DqliteDir::creator(tempdir.path())
            .with_page_size(PAGE_SIZE as u64)
            .with_snapshot(|s| {
                s.with_compression(true)
                    .with_configuration(RaftConfiguration {
                        servers: vec![RaftServer {
                            id: 1,
                            address: "192.168.1.2".to_owned(),
                            role: RaftRole::Voter,
                        }],
                    })
                    .with_term(1)
                    .with_index(100)
                    .with_timestamp(SystemTime::now() - Duration::from_hours(1))
                    .add_connection(&conn, "main")
            })
            .with_first_index(101)
            .with_closed_segment(|s| {
                s.add_entries(&[DqliteLogEntry {
                    term: 1,
                    content: DqliteLogEntryContent::Change(RaftConfiguration {
                        servers: vec![
                            RaftServer {
                                id: 1,
                                address: "192.168.1.2".to_owned(),
                                role: RaftRole::Voter,
                            },
                            RaftServer {
                                id: 2,
                                address: "192.168.1.3".to_owned(),
                                role: RaftRole::Standby,
                            },
                        ],
                    }),
                }])
                .add_wal_segment(1, &conn, "main", 0..)
            })
            .create()?;

        let dqlite_dir = DqliteDir::open(tempdir.path())?;
        let open_state = OpenState::default();
        open_state.load(&dqlite_dir, PAGE_SIZE)?;

        Ok(())
    }
}
