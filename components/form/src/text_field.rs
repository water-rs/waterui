use nami::Binding;
use waterui_core::Str;
use waterui_core::configurable;
use waterui_core::{AnyView, View};

use waterui_text::Text;

configurable!(TextField, TextFieldConfig);

/// Configuration options for a `TextField`.
#[non_exhaustive]
#[derive(Debug)]
pub struct TextFieldConfig {
    /// The label displayed for the text field.
    pub label: AnyView,
    /// The binding to the text value.
    pub value: Binding<Str>,
    /// The placeholder text shown when the field is empty.
    pub prompt: Text,
    /// The type of keyboard to use for input.
    pub keyboard: KeyboardType,
}

#[derive(Debug, Default)]
#[non_exhaustive]
/// Enum representing the type of keyboard to use for text input.
pub enum KeyboardType {
    #[default]
    /// Default keyboard type, typically used for general text input.
    Text,
    /// Keyboard for secure text input, such as passwords.
    Secure,
    /// Keyboard for email input, which may include special characters like `@` and `.`
    Email,
    /// Keyboard for URL input, which may include characters like `:`, `/`, and `.`
    URL,
    /// Keyboard for numeric input, which may include digits and a decimal point.
    Number,
    /// Keyboard for phone number input, which may include digits and special characters like `+`, `-`, and `()`.
    PhoneNumber,
}

impl TextField {
    /// Creates a new `TextField` with the given value binding.
    #[must_use]
    pub fn new(value: &Binding<Str>) -> Self {
        Self(TextFieldConfig {
            label: AnyView::default(),
            value: value.clone(),
            prompt: Text::default(),
            keyboard: KeyboardType::default(),
        })
    }
    /// Sets the label for the text field.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }

    /// Sets the prompt for the text field.
    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<Text>) -> Self {
        self.0.prompt = prompt.into();
        self
    }
}

/// Creates a new [`TextField`] with the specified label and value binding.
pub fn field(label: impl View, value: &Binding<Str>) -> TextField {
    TextField::new(value).label(label)
}
