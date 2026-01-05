use anyhow::anyhow;
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;

use crate::command::help::Help;
use crate::{Context, Result, Shell};

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
        let mut parser = Parser::new(&SQLiteDialect {})
            .with_recursion_limit(100)
            .try_with_sql(raw)?;
        parser.try_parse(|parser| parser.parse_statements())?;

        let raw = raw.to_owned();
        Ok(Self { raw })
    }

    pub(crate) fn run(self, ctx: &Context) -> Result<()> {
        let Self { raw } = self;
        if !matches!(ctx.shell, Shell::Snapshot(_)) {
            return Err(anyhow!("sql not available in {} shell", ctx.shell.name()));
        }

        let conn = ctx.connection()?;
        match conn.execute(&raw, ()) {
            Ok(updated) => println!("{updated} rows affected"),
            Err(err) => return Err(err.into()),
        }
        Ok(())
    }
}
