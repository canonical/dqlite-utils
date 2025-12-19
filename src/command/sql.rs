use crate::command::help::Help;
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct SqlCommand {
    raw: String,
}

impl SqlCommand {
    pub(crate) fn help() -> Help {
        Help::builder()
            .name("<sql>")
            .summary("run a sql command")
            .build()
            .expect("internal error: help invalid")
    }

    pub(crate) fn try_from_raw(raw: &str) -> Result<Self> {
        let raw = raw.to_owned();
        Ok(Self { raw })
    }

    pub(crate) fn run(&self, _ctx: &Context) -> Result<()> {
        let Self { raw } = self;
        todo!("execute {raw}");
    }
}
