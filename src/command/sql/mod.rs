use anyhow::anyhow;
use owo_colors::Style;
use rusqlite::types::ValueRef;

use crate::utils::TerminalStylizeExt;
use crate::{Context, Result};

#[derive(Debug)]
pub(crate) struct SqlCommand {
    raw: String,
}

impl SqlCommand {
    pub(crate) fn try_from_raw(raw: &str) -> Result<Self> {
        let raw = raw.to_owned();
        Ok(Self { raw })
    }

    pub(crate) fn run(self, ctx: &Context) -> Result<()> {
        let Self { raw } = self;
        let conn = ctx.shell.connection().ok_or_else(|| {
            anyhow!(
                "sql execution not available in {} shell",
                ctx.shell.kind().name()
            )
        })?;
        let mut stmt = conn.prepare(&raw)?;
        {
            let column_count = stmt.column_count();

            // Print header
            if column_count > 0 {
                for i in 0..column_count {
                    print!("{}  ", stmt.column_name(i)?);
                }
                println!("\n---");
            }

            // Print content
            let mut rows = stmt.query(())?;
            while let Some(row) = rows.next()? {
                for i in 0..column_count {
                    match row.get_ref(i)? {
                        ValueRef::Blob(blob) => print!("<blob:({}B)>  ", blob.len()),
                        ValueRef::Null => print!("NULL  "),
                        ValueRef::Integer(value) => print!("{}  ", value),
                        ValueRef::Real(value) => print!("{}  ", value),
                        ValueRef::Text(text) => {
                            print!("{}  ", String::from_utf8_lossy(text));
                        }
                    }
                }
                println!();
            }
        }

        if !stmt.readonly() {
            const ROWS_AFFECTED_STYLE: Style = Style::new().dimmed();
            println!(
                "{} {}",
                conn.changes().terminal_style(ROWS_AFFECTED_STYLE),
                "rows affected".terminal_style(ROWS_AFFECTED_STYLE)
            );
        }
        Ok(())
    }
}
