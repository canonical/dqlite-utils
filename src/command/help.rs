use std::io::{self, ErrorKind, Write};

use anyhow::Context as _;
use owo_colors::Style;
use strum::IntoEnumIterator;

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
            .expect("internal error: help invalid")
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
            help.build().expect("internal error: help invalid")
        };
        help.write_to(w)
    }
}

#[derive(Debug)]
struct HelpEntry<K> {
    kind: K,
    name: &'static str,
    summary: &'static str,
}

#[derive(Debug)]
struct Arg {
    optional: bool,
}

#[derive(Debug)]
struct Flag;

// NOTE: short name to avoid clash with `crate::Command` type.
#[derive(Debug)]
struct Cmd;

#[derive(Debug)]
pub(crate) struct Help {
    name: &'static str,
    summary: &'static str,
    skip_usage: bool,
    args: Vec<HelpEntry<Arg>>,
    flags: Vec<HelpEntry<Flag>>,
    commands: Vec<HelpEntry<Cmd>>,
}

impl Help {
    const HEADING_STYLE: Style = Style::new().bold().green();
    const PARAM_STYLE: Style = Style::new().bold().cyan();
    const USAGE_PARAM_STYLE: Style = Style::new().cyan();

    pub(crate) fn builder() -> HelpBuilder {
        HelpBuilder::default()
    }

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
            flags,
            commands,
        } = self;
        Self::handle_section(&mut w, "Arguments", args)?;
        Self::handle_section(&mut w, "Options", flags)?;
        Self::handle_section(&mut w, "Commands", commands)?;

        Ok(())
    }

    fn handle_usage(&self, mut w: impl Write) -> io::Result<()> {
        let Self {
            name,
            args,
            flags,
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
        for flag in flags {
            let name = flag.name.terminal_style(Self::USAGE_PARAM_STYLE);
            write!(w, " [{name}]")?;
        }
        for arg in args {
            let name = arg.name.terminal_style(Self::USAGE_PARAM_STYLE);
            if arg.kind.optional {
                write!(w, " [{name}]")?;
            } else {
                write!(w, " <{name}>")?;
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

        const PADDING: &str = match str::from_utf8(&[b' '; 80]) {
            Ok(padding) => padding,
            Err(_) => unreachable!(),
        };
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

#[derive(Default)]
pub(crate) struct HelpBuilder {
    name: Option<&'static str>,
    summary: Option<&'static str>,
    skip_usage: bool,
    args: Vec<HelpEntry<Arg>>,
    flags: Vec<HelpEntry<Flag>>,
    commands: Vec<HelpEntry<Cmd>>,
}

impl HelpBuilder {
    pub(crate) fn name(mut self, name: &'static str) -> Self {
        self.name = Some(name);
        self
    }

    pub(crate) fn summary(mut self, summary: &'static str) -> Self {
        self.summary = Some(summary);
        self
    }

    pub(crate) fn skip_usage(mut self) -> Self {
        self.skip_usage = true;
        self
    }

    #[allow(unused)]
    pub(crate) fn add_arg(mut self, name: &'static str, summary: &'static str) -> Self {
        self.args.push(HelpEntry {
            name,
            kind: Arg { optional: false },
            summary,
        });
        self
    }

    pub(crate) fn add_optional_arg(mut self, name: &'static str, summary: &'static str) -> Self {
        self.args.push(HelpEntry {
            name,
            kind: Arg { optional: true },
            summary,
        });
        self
    }

    pub(crate) fn add_flag(mut self, name: &'static str, summary: &'static str) -> Self {
        self.flags.push(HelpEntry {
            name,
            kind: Flag,
            summary,
        });
        self
    }

    pub(crate) fn add_command(mut self, name: &'static str, summary: &'static str) -> Self {
        self.commands.push(HelpEntry {
            name,
            kind: Cmd,
            summary,
        });
        self
    }

    pub(crate) fn build(self) -> Result<Help> {
        let Self {
            name,
            summary,
            skip_usage,
            args,
            flags,
            commands,
        } = self;
        let name = name.context("internal error: help declared without name")?;
        let summary = summary.context("internal error: help declared without summary")?;
        Ok(Help {
            name,
            summary,
            skip_usage,
            args,
            flags,
            commands,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::io::Cursor;

    use googletest::expect_that;

    use googletest::matchers::{anything, contains_substring, displays_as, err};
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
            .test(Help::builder().name(NAME).summary(SUMMARY).build().unwrap());

        const FLAG_1: &str = "--flag-1";
        const FLAG_2: &str = "--flag-2";
        const FLAG_1_HELP: &str = "__FLAG_1_HELP__";
        const FLAG_2_HELP: &str = "__FLAG_2_HELP__";
        Test::new("flags")
            .expect(FLAG_1)
            .expect(FLAG_2)
            .expect(FLAG_1_HELP)
            .expect(FLAG_2_HELP)
            .test(
                Help::builder()
                    .name(NAME)
                    .summary(SUMMARY)
                    .add_flag(FLAG_1, FLAG_1_HELP)
                    .add_flag(FLAG_2, FLAG_2_HELP)
                    .build()
                    .unwrap(),
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
                    .build()
                    .unwrap(),
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
                    .build()
                    .unwrap(),
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

    #[googletest::test]
    fn test_help_validation() {
        let no_name = Help::builder().build();
        expect_that!(no_name, err(anything()));

        let no_summary = Help::builder().name("asdf").build();
        expect_that!(no_summary, err(anything()));
    }
}
