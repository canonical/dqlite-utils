use std::borrow::Cow;
use std::fmt::Display;

use owo_colors::Style;

use crate::utils::TerminalStylizeExt;

#[derive(Debug)]
pub struct Prompt {
    content: Cow<'static, str>,
}

impl Prompt {
    const STYLE: Style = Style::new().bright_green().bold();

    #[allow(unused)]
    pub(crate) fn new(text: impl Display) -> Self {
        let content = Cow::from(format!("{text}> "));
        Self { content }
    }
}

impl Default for Prompt {
    fn default() -> Self {
        let content = Cow::from("> ");
        Self { content }
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { content } = self;
        write!(f, "{}", content.terminal_style(Self::STYLE))
    }
}
