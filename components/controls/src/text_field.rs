//! A text input component wired to a reactive string binding.
use core::num::NonZeroUsize;

use nami::Binding;
use waterui_core::Str;
use waterui_core::configurable;
use waterui_core::{AnyView, View, layout::StretchAxis};

use waterui_text::Text;

configurable!(
    /// A single-line text input field.
    ///
    /// TextField lets users enter and edit text.
    ///
    /// # Layout Behavior
    ///
    /// TextField **expands horizontally** to fill available space, but has a fixed height.
    /// In an `HStack`, it will take up all remaining width after other views are sized.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Basic text field
    /// TextField::new(&name)
    ///
    /// // With label and placeholder
    /// field("Username", &username)
    ///     .prompt("Enter your name")
    ///
    /// // In a form (TextField fills remaining space)
    /// hstack((
    ///     text("Email:"),
    ///     TextField::new(&email),
    /// ))
    /// ```
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //

    // Height: Fixed intrinsic (platform-determined)
    // Width: Reports minimum usable width, expands during layout phase
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    TextField,
    TextFieldConfig,
    StretchAxis::Horizontal
);

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
    /// The maximum number of lines to show.
    /// If `None`, the text field will show as many lines as needed.
    pub line_limit: Option<NonZeroUsize>,
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
            line_limit: NonZeroUsize::new(1),
        })
    }
    /// Sets the label for the text field.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }

    /// Sets the maximum number of lines to show.
    ///
    /// By default, the line limit is 1.
    #[must_use]
    pub fn line_limit(mut self, line_limit: usize) -> Self {
        assert!(line_limit > 0, "Line limit must be greater than 0");
        self.0.line_limit = NonZeroUsize::new(line_limit);
        self
    }

    /// Disables the line limit.
    #[must_use]
    pub fn disable_line_limit(mut self) -> Self {
        self.0.line_limit = None;
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
