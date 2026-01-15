use std::borrow::Cow;
use std::fmt::Display;

use owo_colors::Style;

#[derive(Debug)]
pub struct Prompt {
    content: Cow<'static, str>,
}

impl Prompt {
    pub(crate) const STYLE: Style = Style::new().bright_green().bold();

    pub(crate) fn new(text: impl Display) -> Self {
        let content = Cow::from(format!("{text}> "));
        Self { content }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.content
    }
}

impl Default for Prompt {
    fn default() -> Self {
        Self {
            content: Cow::from("> "),
        }
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { content, .. } = self;
        write!(f, "{content}")
    }
}
