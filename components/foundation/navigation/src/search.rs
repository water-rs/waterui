use nami::Binding;
use waterui_core::Str;
use waterui_core::handler::{Handler, SharedAction};
use waterui_text::{IntoText, Text};

/// Search field configuration rendered inside navigation chrome.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NavigationSearch {
    /// Caller-owned query binding.
    text: Binding<Str>,
    /// Optional prompt shown when the query is empty.
    prompt: Option<Text>,
    /// Optional caller-owned active state binding.
    active: Option<Binding<bool>>,
    /// Optional submit action run when the user commits the search query.
    on_submit: Option<SharedAction>,
}

impl NavigationSearch {
    /// Caller-owned query binding.
    #[must_use]
    pub const fn text(&self) -> &Binding<Str> {
        &self.text
    }

    /// Optional prompt shown when the query is empty.
    #[must_use]
    pub const fn prompt(&self) -> Option<&Text> {
        self.prompt.as_ref()
    }

    /// Sets the `prompt`.
    pub fn set_prompt(&mut self, prompt: impl IntoText) {
        self.prompt = Some(prompt.into_text());
    }

    /// Caller-owned active state binding.
    #[must_use]
    pub const fn active(&self) -> Option<&Binding<bool>> {
        self.active.as_ref()
    }

    /// Sets the `active` binding.
    pub fn set_active(&mut self, active: &Binding<bool>) {
        self.active = Some(active.clone());
    }

    /// Submit action run when the user commits the search query.
    #[must_use]
    pub const fn on_submit(&self) -> Option<&SharedAction> {
        self.on_submit.as_ref()
    }

    /// Sets the submit action.
    pub fn set_on_submit<Args>(&mut self, action: impl Handler<Args, ()>) {
        self.on_submit = Some(SharedAction::new(action));
    }
}

impl NavigationSearch {
    /// Creates a new navigation search model.
    #[must_use]
    pub fn new(text: &Binding<Str>) -> Self {
        Self {
            text: text.clone(),
            prompt: None,
            active: None,
            on_submit: None,
        }
    }
}
