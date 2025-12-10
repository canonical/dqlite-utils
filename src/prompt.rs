use std::fmt::Display;

use owo_colors::Style;

use crate::utils::TerminalStylizeExt;

#[derive(Debug)]
pub struct Prompt {
    content: String,
}

impl Prompt {
    const STYLE: Style = Style::new().bright_green().bold();

    #[allow(unused)]
    pub(crate) fn new(text: impl Display) -> Self {
        let content = format!("{text}> ").terminal_style(Self::STYLE).to_string();
        Self { content }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.content
    }
}

impl Default for Prompt {
    fn default() -> Self {
        let content = "> ".terminal_style(Self::STYLE).to_string();
        Self { content }
    }
}
