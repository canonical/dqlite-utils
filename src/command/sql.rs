use anyhow::{anyhow, Context as _};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;

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
        let mut parser = Parser::new(&SQLiteDialect {})
            .with_recursion_limit(100)
            .try_with_sql(raw)?;
        parser.try_parse(|parser| parser.parse_statements())?;

        let raw = raw.to_owned();
        Ok(Self { raw })
    }

    pub(crate) fn run(self, ctx: &Context) -> Result<()> {
        let Self { raw } = self;
        let conn = ctx
            .shell
            .connection()
            .ok_or_else(|| anyhow!("sql execution not available in {} shell", ctx.shell.name()))?;
        let rows: Vec<_> = conn.prepare(&raw)?
            .query_map((), |row| Ok(format!("{row:?}")))? // Placeholder repr.
            .map(|res| res.context("cannot execute query"))
            .collect::<Result<_>>()?;
        // TODO(kcza): replace this row output with something prettier.
        for row in rows {
            println!("{row}");
        }
        // TODO(kcza): only output rows affected on queries containing CRUD.
        println!("{} rows affected", conn.changes());
        Ok(())
    }
}
