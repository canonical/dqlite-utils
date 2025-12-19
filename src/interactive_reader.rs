use std::borrow::Cow;
use std::fmt::{Display, Write};
use std::path::PathBuf;

use owo_colors::Style;
use rustyline::completion::Completer;
use rustyline::config::BellStyle;
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::history::DefaultHistory;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{CompletionType, Config, Editor, Helper as RustylineHelper};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;

use crate::command::{Command, UnknownCommand};
use crate::prompt::Prompt;
use crate::utils::TerminalStylizeExt;
use crate::{Context, Result};

pub(crate) struct InteractiveCommandReader<T: CommandHelper> {
    history_path: Option<PathBuf>,

    line_editor: Editor<Helper<T>, DefaultHistory>,
}

impl<T: CommandHelper + Default> InteractiveCommandReader<T> {
    pub(crate) fn new() -> Result<Self> {
        const HISTORY_FILE: &str = ".dqlite-utils-history";

        let config = Config::builder()
            .max_history_size(100)?
            .completion_type(CompletionType::List)
            .completion_prompt_limit(20)
            .auto_add_history(true)
            .bell_style(BellStyle::Audible)
            .tab_stop(4)
            .indent_size(4)
            .build();
        let mut line_editor = Editor::with_config(config)?;
        line_editor.set_helper(Some(Helper {
            command_helper: T::default(),
        }));

        let history_path = home::home_dir().map(|home| home.join(HISTORY_FILE));
        if let Some(history_path) = &history_path {
            line_editor.load_history(&history_path).ok();
        } else {
            eprintln!("cannot load history");
        }
        Ok(Self {
            history_path,
            line_editor,
        })
    }

    pub(crate) fn banner(&self) -> impl Display {
        r#"enter ".help" for usage hints"#
    }

    pub(crate) fn read(&mut self, ctx: &Context) -> Result<Option<Command>> {
        let line = self.line_editor.readline(ctx.shell.prompt().as_str())?;
        let trimmed_line = line.trim();
        let ret = trimmed_line.parse().map(Some);
        self.line_editor.add_history_entry(line)?;
        ret
    }

    pub(crate) fn helper_mut(&mut self) -> &mut Helper<T> {
        self.line_editor.helper_mut().expect("internal error: no helper set")
    }
}

impl<T: CommandHelper> Drop for InteractiveCommandReader<T> {
    fn drop(&mut self) {
        if let Some(history_path) = &self.history_path {
            if let Err(err) = self.line_editor.save_history(history_path) {
                eprintln!("cannot save history: {err}");
            }
        }
    }
}

pub(crate) struct Helper<T> {
    pub(crate) command_helper: T,
}

impl<T> Helper<T> {
    const HINT_STYLE: Style = Style::new().yellow();
    const KNOWN_COMMAND_STYLE: Style = Style::new().blue();
    const UNKNOWN_COMMAND_STYLE: Style = Style::new().red();
}

impl<T: CommandHelper> RustylineHelper for Helper<T> {}

impl<T: CommandHelper> Completer for Helper<T> {
    type Candidate = &'static str;
    // TODO(kcza): completions
    fn complete(
        &self, // FIXME should be `&mut self`
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let command_prefix = &line[..pos];
        if command_prefix.contains(' ') {
            return Ok((0, Vec::new()));
        }

        let candidates = self.command_helper
            .known_commands()
            .filter(|name| name.starts_with(command_prefix))
            .collect();
        Ok((0, candidates))
    }
}

impl<T> Hinter for Helper<T> {
    type Hint = <HistoryHinter as Hinter>::Hint;

    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<Self::Hint> {
        (HistoryHinter {}).hint(line, pos, ctx)
    }
}

impl<T: CommandHelper> Highlighter for Helper<T> {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        prompt.terminal_style(Prompt::STYLE).to_string().into()
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        hint.terminal_style(Self::HINT_STYLE).to_string().into()
    }

    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if line.starts_with('.') {
            let (first_word, separator, remainder) = if let Some((first_word, remainder)) = line.split_once(' ') {
                (first_word, " ", remainder)
            } else {
                (line, "", "")
            };

            let mut ret = String::with_capacity(first_word.len() + remainder.len() + 20);
            let command_known = self.command_helper
                .known_commands()
                .any(|command| command == first_word);
            let first_word_style = if command_known {
                Self::KNOWN_COMMAND_STYLE
            } else {
                Self::UNKNOWN_COMMAND_STYLE
            };
            write!(
                &mut ret,
                "{}{separator}{remainder}",
                first_word.terminal_style(first_word_style)
            )
            .expect("internal error: cannot write highlighted line");
            return Cow::from(ret);
        }

        // Unhighlighted.
        Cow::from(line)
    }
}

impl<T: CommandHelper> Validator for Helper<T> {
    fn validate_while_typing(&self) -> bool {
        false
    }

    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();
        if input.is_empty() {
            Ok(ValidationResult::Valid(None))
        } else if input.starts_with('.')
            && let Some(command) = input.split_whitespace().next()
        {
            self.validate_command(command)
        } else {
            self.validate_sql(input)
        }
    }
}

impl<T: CommandHelper> Helper<T> {
    fn validate_command(&self, to_validate: &str) -> rustyline::Result<ValidationResult> {
        let command_name = match to_validate.split_whitespace().next() {
            Some(command) => command,
            None => return Ok(ValidationResult::Valid(None)),
        };

        let command_candidate = self.command_helper.known_commands()
            .filter(|name| name.starts_with(command_name))
            .next();
        if let Some(name) = command_candidate
            && name == command_name
        {
            Ok(ValidationResult::Valid(None))
        } else {
            Ok(ValidationResult::Invalid(Some(UnknownCommand.to_string())))
        }
    }

    fn validate_sql(&self, to_validate: &str) -> rustyline::Result<ValidationResult> {
        let parser = Parser::new(&SQLiteDialect {})
            .with_recursion_limit(100)
            .try_with_sql(to_validate);
        let mut parser = match parser {
            Ok(parser) => parser,
            Err(err) => return Ok(ValidationResult::Invalid(Some(err.to_string()))),
        };

        if let Some(err) = parser.try_parse(|parser| parser.parse_statements()).err() {
            Ok(ValidationResult::Invalid(Some(err.to_string())))
        } else if !to_validate.trim_end().ends_with(';') {
            Ok(ValidationResult::Incomplete)
        } else {
            Ok(ValidationResult::Valid(None))
        }
    }
}

pub(crate) trait CommandHelper {
    fn known_commands(&self) -> impl Iterator<Item = &'static str>;
}
