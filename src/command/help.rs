use std::io::{self, ErrorKind, Write};

use owo_colors::Style;
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use crate::utils::TerminalStylizeExt;
use crate::{Context, Result};

use super::{CommandKind, UnrecognizedArgumentsError};

#[derive(Debug)]
pub(crate) struct HelpCommand {
    command: Option<CommandKind>,
}

impl HelpCommand {
    pub(crate) const SUMMARY: &'static str = "Print help and exit";

    pub(crate) fn help() -> Help {
        Help::builder()
            .name("help")
            .summary(Self::SUMMARY)
            .add_optional_arg("command", "the command to get help for")
            .build()
    }

    pub(crate) fn try_from_args(args: &[String]) -> Result<Self> {
        match args {
            [] => Self::new(),
            [command] => Self::with_command(command.parse()?),
            [_, tail @ ..] => Err(UnrecognizedArgumentsError(tail.to_vec()).into()),
        }
    }

    fn new() -> Result<Self> {
        Ok(Self { command: None })
    }

    fn with_command(command: CommandKind) -> Result<Self> {
        Ok(Self {
            command: Some(command),
        })
    }

    pub(crate) fn run(self, _ctx: &Context) -> Result<()> {
        let Self { command } = self;
        let stdout = io::stdout().lock();
        let res = match command {
            None => Self::write_general_help(stdout),
            Some(command) => command.write_help(stdout),
        };
        match res {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::BrokenPipe => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    fn write_general_help(w: impl Write) -> io::Result<()> {
        let help = {
            let mut help = Help::builder()
                .name("dqlite-utils")
                .summary("an observability tool for inspecting the on-disk state of a dqlite node")
                .skip_usage();
            for command in CommandKind::iter() {
                help = help.add_command(command.name(), command.summary());
            }
            help.build()
        };
        help.write_to(w)
    }
}

pub(crate) struct HelpEntry<K> {
    kind: K,
    name: &'static str,
    summary: &'static str,
}

pub(crate) struct Arg {
    optional: bool,
}
// NOTE: short name to avoid clash with `std::option::Option`.
pub(crate) struct Opt;
// NOTE: short name to avoid clash with `crate::Command` type.
pub(crate) struct Cmd;

#[derive(TypedBuilder)]
#[builder(mutators(
    pub(crate) fn add_arg(&mut self, name: &'static str, summary: &'static str) {
        self.args.push(HelpEntry {
            name,
            kind: Arg {
                optional: false,
            },
            summary,
        });
    }

    pub(crate) fn add_optional_arg(&mut self, name: &'static str, summary: &'static str) {
        self.args.push(HelpEntry {
            name,
            kind: Arg {
                optional: true,
            },
            summary,
        });
    }

    pub(crate) fn add_option(&mut self, name: &'static str, summary: &'static str) {
        self.options.push(HelpEntry {
            name,
            kind: Opt,
            summary,
        });
    }

    pub(crate) fn add_command(&mut self, name: &'static str, summary: &'static str) {
        self.commands.push(HelpEntry {
            name,
            kind: Cmd,
            summary,
        });
    }
))]
pub(crate) struct Help {
    name: &'static str,
    summary: &'static str,

    #[builder(setter(strip_bool))]
    skip_usage: bool,

    #[builder(default, via_mutators)]
    args: Vec<HelpEntry<Arg>>,

    #[builder(default, via_mutators)]
    options: Vec<HelpEntry<Opt>>,

    #[builder(default, via_mutators)]
    commands: Vec<HelpEntry<Cmd>>,
}

impl Help {
    const HEADING_STYLE: Style = Style::new().bold().green();
    const PARAM_STYLE: Style = Style::new().bold().cyan();
    const USAGE_PARAM_STYLE: Style = Style::new().cyan();

    pub(crate) fn write_to(self, mut w: impl Write) -> io::Result<()> {
        let name = self.name.terminal_style(Self::PARAM_STYLE);
        writeln!(w, "{name} - {}", self.summary)?;

        if !self.skip_usage {
            self.handle_usage(&mut w)?;
        }

        let Self {
            name: _,
            summary: _,
            skip_usage: _,
            args,
            options,
            commands,
        } = self;
        Self::handle_section(&mut w, "Arguments", args)?;
        Self::handle_section(&mut w, "Options", options)?;
        Self::handle_section(&mut w, "Commands", commands)?;

        Ok(())
    }

    fn handle_usage(&self, mut w: impl Write) -> io::Result<()> {
        let Self {
            name,
            args,
            options,
            commands,
            ..
        } = self;
        writeln!(w, "\n{}", "Usage".terminal_style(Self::HEADING_STYLE))?;
        let name = name.terminal_style(Self::PARAM_STYLE);

        if !commands.is_empty() {
            writeln!(w, "  {name} <command>")?;
            return Ok(());
        }

        write!(w, "  {name}")?;
        for option in options {
            write!(w, " [{}]", option.name)?;
        }
        for arg in args {
            let name = arg.name.terminal_style(Self::USAGE_PARAM_STYLE);
            if arg.kind.optional {
                write!(w, " [{name}]")?;
            } else {
                write!(w, " {name}")?;
            }
        }
        writeln!(w)?;

        Ok(())
    }

    fn handle_section<K>(
        mut w: impl Write,
        name: &str,
        entries: Vec<HelpEntry<K>>,
    ) -> io::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        writeln!(w, "\n{}", name.terminal_style(Self::HEADING_STYLE))?;
        let max_name_len = entries
            .iter()
            .map(|entry| entry.name.len())
            .max()
            .expect("internal error: no max of non-empty list");

        const PADDING: &str = "                         ";
        let padding_to = |intended_len, word: &str| {
            assert!(intended_len < PADDING.len());
            &PADDING[..intended_len - word.len()]
        };
        for entry in entries {
            let HelpEntry { name, summary, .. } = entry;
            let padding = padding_to(max_name_len, name);
            let name = name.terminal_style(Self::PARAM_STYLE);
            writeln!(w, "  {name}{padding}  {summary}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::io::Cursor;

    use googletest::expect_that;
    use googletest::matchers::contains_substring;
    use strum::IntoEnumIterator;

    use super::*;

    #[googletest::test]
    fn test_all_commands_listed_in_help() {
        let help_output = {
            let mut help_output = Cursor::new(Vec::new());
            HelpCommand::write_general_help(&mut help_output).unwrap();
            String::try_from(help_output.into_inner()).unwrap()
        };
        for command_kind in CommandKind::iter() {
            expect_that!(help_output, contains_substring(command_kind.name()));
        }
    }

    #[googletest::test]
    fn test_help_output() {
        const NAME: &str = "__HELP_NAME__";
        const SUMMARY: &str = "__HELP_NAME__";

        Test::new("basic info")
            .expect(NAME)
            .expect(SUMMARY)
            .test(Help::builder().name(NAME).summary(SUMMARY).build());

        const OPTION_1: &str = "--option-1";
        const OPTION_2: &str = "--option-2";
        const OPTION_1_HELP: &str = "__OPTION_1_HELP__";
        const OPTION_2_HELP: &str = "__OPTION_2_HELP__";
        Test::new("options")
            .expect(OPTION_1)
            .expect(OPTION_2)
            .expect(OPTION_1_HELP)
            .expect(OPTION_2_HELP)
            .test(
                Help::builder()
                    .name(NAME)
                    .summary(SUMMARY)
                    .add_option(OPTION_1, OPTION_1_HELP)
                    .add_option(OPTION_2, OPTION_2_HELP)
                    .build(),
            );

        const ARG_1: &str = "__ARG_1__";
        const ARG_2: &str = "__ARG_2__";
        const ARG_1_HELP: &str = "__ARG_1_HELP__";
        const ARG_2_HELP: &str = "__ARG_2_HELP__";
        Test::new("args")
            .expect(ARG_1)
            .expect(ARG_2)
            .expect(format!("[{ARG_2}]"))
            .expect(ARG_1_HELP)
            .expect(ARG_2_HELP)
            .test(
                Help::builder()
                    .name(NAME)
                    .summary(SUMMARY)
                    .add_arg(ARG_1, ARG_1_HELP)
                    .add_optional_arg(ARG_2, ARG_2_HELP)
                    .build(),
            );

        const COMMAND_1: &str = "__COMMAND_1__";
        const COMMAND_2: &str = "__COMMAND_2__";
        const COMMAND_1_HELP: &str = "__COMMAND_1_HELP__";
        const COMMAND_2_HELP: &str = "__COMMAND_2_HELP__";
        Test::new("commands")
            .expect(COMMAND_1)
            .expect(COMMAND_2)
            .expect(COMMAND_1_HELP)
            .expect(COMMAND_2_HELP)
            .test(
                Help::builder()
                    .name(NAME)
                    .summary(SUMMARY)
                    .add_command(COMMAND_1, COMMAND_1_HELP)
                    .add_command(COMMAND_2, COMMAND_2_HELP)
                    .build(),
            );

        // Test helpers.
        struct Test<'a> {
            name: &'static str,
            expected: Vec<Cow<'a, str>>,
        }

        impl<'a> Test<'a> {
            fn new(name: &'static str) -> Self {
                let expected = Vec::new();
                Self { name, expected }
            }

            fn expect(mut self, expected: impl Into<Cow<'a, str>>) -> Self {
                self.expected.push(expected.into());
                self
            }

            fn test(self, help: Help) {
                let Self { name, expected } = self;
                eprintln!("Test summary: {name}");

                let written_help = {
                    let mut written_help = Cursor::new(Vec::new());
                    help.write_to(&mut written_help).unwrap();
                    String::from_utf8_lossy(&written_help.into_inner()).into_owned()
                };
                for expected in expected {
                    expect_that!(written_help, contains_substring(expected));
                }
            }
        }
    }
}
