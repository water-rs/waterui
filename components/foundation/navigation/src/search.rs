use nami::Binding;
use waterui_core::Str;
use waterui_text::{IntoText, Text};

/// Search field configuration rendered inside navigation chrome.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NavigationSearch {
    /// Caller-owned query binding.
    text: Binding<Str>,
    /// Prompt shown when the query is empty.
    prompt: Text,
}

impl NavigationSearch {
    /// Caller-owned query binding.
    #[must_use]
    pub const fn text(&self) -> &Binding<Str> {
        &self.text
    }

    /// Prompt shown when the query is empty.
    #[must_use]
    pub const fn prompt(&self) -> &Text {
        &self.prompt
    }

    /// Sets the `prompt`.
    pub fn set_prompt(&mut self, prompt: Text) {
        self.prompt = prompt;
    }
}

impl NavigationSearch {
    /// Creates a new navigation search model.
    #[must_use]
    pub fn new(text: &Binding<Str>, prompt: impl IntoText) -> Self {
        Self {
            text: text.clone(),
            prompt: prompt.into_text(),
        }
    }
}
