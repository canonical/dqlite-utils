use std::fmt::Display;

use owo_colors::Style;

use crate::utils::TerminalStylizeExt;

pub(crate) struct Prompt {
    content: String,
}

impl Prompt {
    const SEPARATOR_STYLE: Style = Style::new().bright_green();

    #[allow(unused)]
    pub(crate) fn new(text: impl Display) -> Self {
        let separator = "> ".terminal_style(Self::SEPARATOR_STYLE).to_string();
        let content = format!("{text}{separator}");
        Self { content }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.content
    }
}

impl Default for Prompt {
    fn default() -> Self {
        let content = "> ".terminal_style(Self::SEPARATOR_STYLE).to_string();
        Self { content }
    }
}
