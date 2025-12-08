use anyhow::{Context as _, anyhow};
use time::{UtcDateTime, format_description::well_known::Iso8601};

use crate::{
    Context, Result,
    command::{MissingArgumentError, UnrecognizedArgumentsError, help::Help},
};

pub(crate) struct SetTimestampCommand {
    timestamp: UtcDateTime,
}

impl SetTimestampCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("set-timestamp")
            .summary("set the timestamp of the snapshot")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        let timestamp = match args {
            [] => return Err(MissingArgumentError("timestamp").into()),
            [timestamp] => timestamp,
            [_, tail @ ..] => return Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        };
        let timestamp = UtcDateTime::parse(timestamp, &Iso8601::DEFAULT)
            .context("cannot parse timestamp, expected yyyy-mm-ddThh:MM:ss")?;
        Ok(Self { timestamp })
    }

    pub(crate) fn run(self, ctx: &mut Context) -> Result<()> {
        let Self { timestamp } = self;
        let shell = ctx.shell.snapshot_mut().ok_or_else(|| {
            anyhow!("internal error: finish command not called in snapshot shell")
        })?;
        shell.snapshot.timestamp = timestamp;
        Ok(())
    }
}
