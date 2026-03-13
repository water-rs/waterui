use nami::Binding;
use waterui_core::Str;

/// Search field configuration rendered inside navigation chrome.
#[derive(Debug, Clone)]
pub struct NavigationSearch {
    /// Caller-owned query binding.
    pub text: Binding<Str>,
    /// Prompt shown when the query is empty.
    pub prompt: Str,
}

impl NavigationSearch {
    /// Creates a new navigation search model.
    #[must_use]
    pub fn new(text: &Binding<Str>, prompt: impl Into<Str>) -> Self {
        Self {
            text: text.clone(),
            prompt: prompt.into(),
        }
    }
}
