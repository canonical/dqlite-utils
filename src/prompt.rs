use std::borrow::Cow;
use std::fmt::Display;

use owo_colors::Style;

use crate::utils::TerminalStylizeExt;

#[derive(Debug)]
pub struct Prompt {
    _content: Cow<'static, str>,
    pretty_content: String,
}

impl Prompt {
    const STYLE: Style = Style::new().bright_green().bold();

    #[allow(unused)]
    pub(crate) fn new(text: impl Display) -> Self {
        let content = Cow::from(format!("{text}> "));
        let pretty_content = content.terminal_style(Self::STYLE).to_string();
        Self {
            _content: content,
            pretty_content,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.pretty_content
    }
}

impl Default for Prompt {
    fn default() -> Self {
        let content = Cow::from("> ");
        let pretty_content = content.terminal_style(Self::STYLE).to_string();
        Self {
            _content: content,
            pretty_content,
        }
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { pretty_content, .. } = self;
        write!(f, "{pretty_content}")
    }
}
