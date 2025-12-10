use time::UtcDateTime;

use crate::command::help::Help;
use crate::command::{UnknownCommand, UnrecognizedArgumentsError};
use crate::dqlite::RaftServer;
use crate::prompt::Prompt;
use crate::{Context, Result, Shell};

#[derive(Debug)]
pub(crate) struct SnapshotCommand;

impl SnapshotCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot")
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
        ctx.shell = Shell::Snapshot(SnapshotShell::new());
        Ok(())
    }
}

#[derive(Debug)]
pub struct SnapshotShell {
    #[allow(unused)]
    snapshot: ShellSnapshotContext,
    prompt: Prompt,
}

impl SnapshotShell {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("snapshot shell")
            .summary("incrementally create a snapshot")
            .skip_usage()
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn new() -> Self {
        let snapshot = ShellSnapshotContext::new();
        let prompt = Prompt::new("snapshot");
        Self { snapshot, prompt }
    }

    pub(crate) fn prompt(&self) -> &Prompt {
        &self.prompt
    }
}

#[derive(Debug)]
struct ShellSnapshotContext {
    #[allow(unused)]
    term: u64,
    #[allow(unused)]
    index: u64,
    #[allow(unused)]
    timestamp: UtcDateTime,
    #[allow(unused)]
    configuration: ShellSnapshotRaftConfiguration,
}

impl ShellSnapshotContext {
    fn new() -> Self {
        Self {
            term: 1,
            index: 1,
            timestamp: UtcDateTime::now(),
            configuration: ShellSnapshotRaftConfiguration::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ShellSnapshotRaftConfiguration {
    #[allow(unused)]
    servers: Vec<RaftServer>,
}

impl ShellSnapshotRaftConfiguration {
    fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug)]
pub(crate) enum SnapshotShellCommand {}

impl SnapshotShellCommand {
    pub(crate) fn kind(&self) -> SnapshotShellCommandKind {
        unimplemented!();
    }

    pub(crate) fn try_from_input(_command: &str, _args: &[String]) -> Result<Self> {
        Err(UnknownCommand.into())
    }

    pub(crate) fn run(self, _ctx: &mut Context) -> Result<()> {
        unimplemented!();
    }
}

#[derive(Debug)]
pub(crate) enum SnapshotShellCommandKind {}

impl SnapshotShellCommandKind {
    pub(crate) fn help(&self) -> Help {
        unimplemented!()
    }

    pub(crate) fn name(&self) -> &'static str {
        unimplemented!();
    }
}
