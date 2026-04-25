//! A text input component wired to a reactive string binding.
//!
//! ![`TextField`](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/text_field.svg)
use core::num::NonZeroUsize;

use alloc::vec::Vec;
use nami::{Binding, Computed};
use waterui_core::Str;
use waterui_core::configurable;
use waterui_core::{AnyView, Environment, View, layout::StretchAxis};
use waterui_text::{IntoText, Text, TextConfig, styled::StyledStr};

use crate::label::IntoLabel;
use crate::menu::{MenuItem, MenuView, ResolvedMenuItem, resolve_menu_items};

/// Configuration options for a `TextField`.
#[non_exhaustive]
#[derive(Debug)]
pub struct TextFieldConfig {
    /// The label displayed for the text field.
    pub label: AnyView,
    /// The binding to the text value.
    pub value: Binding<StyledStr>,
    /// The placeholder text shown when the field is empty.
    pub prompt: Text,
    /// The type of keyboard to use for input.
    pub keyboard: KeyboardType,
    /// Optional selected-text menu items.
    ///
    /// These actions are shown by native text selection UI.
    pub selection_menu: Computed<Vec<MenuItem>>,
    /// Reserved for future multi-line support.
    ///
    /// Currently only `Some(1)` is supported across backends.
    pub line_limit: Option<NonZeroUsize>,
}

/// Native-resolved text field payload.
#[derive(Debug)]
pub struct ResolvedTextFieldConfig {
    /// The label displayed for the text field.
    pub label: AnyView,
    /// The binding to the text value.
    pub value: Binding<StyledStr>,
    /// The resolved placeholder text shown when the field is empty.
    pub prompt: TextConfig,
    /// The type of keyboard to use for input.
    pub keyboard: KeyboardType,
    /// Optional selected-text menu items resolved for the effective locale.
    pub selection_menu: Computed<Vec<ResolvedMenuItem>>,
    /// Reserved for future multi-line support.
    pub line_limit: Option<NonZeroUsize>,
}

configurable!(
    /// Resolved text field view passed to native backends.
    ResolvedTextField,
    ResolvedTextFieldConfig,
    StretchAxis::Horizontal
);

/// A single-line text input field.
#[derive(Debug)]
pub struct TextField(TextFieldConfig);

#[derive(Debug, Default)]
#[non_exhaustive]
/// Enum representing the type of keyboard to use for text input.
pub enum KeyboardType {
    #[default]
    /// Default keyboard type, typically used for general text input.
    Text,
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
        let styled_value = map_plain_binding(value);
        Self::styled(&styled_value)
    }

    /// Creates a new `TextField` backed by a styled text binding.
    #[must_use]
    pub fn styled(value: &Binding<StyledStr>) -> Self {
        Self(TextFieldConfig {
            label: AnyView::default(),
            value: value.clone(),
            prompt: Text::default(),
            keyboard: KeyboardType::default(),
            selection_menu: Computed::constant(Vec::new()),
            line_limit: NonZeroUsize::new(1),
        })
    }

    /// Sets the label for the text field.
    #[must_use]
    pub fn label(mut self, label: impl IntoLabel) -> Self {
        self.0.label = AnyView::new(label.into_label());
        self
    }

    /// Sets the maximum number of lines to show.
    ///
    /// By default, the line limit is 1.
    ///
    /// # Panics
    ///
    /// Panics if `line_limit` is 0.
    #[must_use]
    pub fn line_limit(mut self, line_limit: usize) -> Self {
        assert!(line_limit > 0, "Line limit must be greater than 0");
        self.0.line_limit = NonZeroUsize::new(line_limit);
        self
    }

    /// Disables the line limit.
    #[must_use]
    pub const fn disable_line_limit(mut self) -> Self {
        self.0.line_limit = None;
        self
    }

    /// Sets the prompt for the text field.
    #[must_use]
    pub fn prompt(mut self, prompt: impl IntoText) -> Self {
        self.0.prompt = prompt.into_text();
        self
    }

    /// Sets custom selected-text menu items for this text field.
    #[must_use]
    pub fn selection_menu(mut self, items: impl MenuView) -> Self {
        self.0.selection_menu = items.into_menu_items();
        self
    }
}

impl View for TextField {
    fn body(self, env: &Environment) -> impl View {
        let selection_menu = resolve_menu_items(&self.0.selection_menu, env);

        AnyView::new(ResolvedTextField(ResolvedTextFieldConfig {
            label: self.0.label,
            value: self.0.value,
            prompt: self.0.prompt.resolve(env),
            keyboard: self.0.keyboard,
            selection_menu,
            line_limit: self.0.line_limit,
        }))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

/// Creates a new [`TextField`] with the specified label and value binding.
pub fn field(label: impl IntoLabel, value: &Binding<Str>) -> TextField {
    TextField::new(value).label(label)
}

fn map_plain_binding(value: &Binding<Str>) -> Binding<StyledStr> {
    Binding::mapping(value, StyledStr::plain, |plain_binding, styled| {
        assert!(
            styled.is_plain(),
            "TextField::new(&Binding<Str>) cannot accept styled text updates; use TextField::styled(&Binding<StyledStr>)"
        );
        plain_binding.set(styled.to_plain());
    })
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use nami::Binding;
    use waterui_core::Str;
    use waterui_text::styled::StyledStr;

    use super::TextField;

    #[test]
    #[should_panic(expected = "Line limit must be greater than 0")]
    fn line_limit_zero_panics() {
        let value = Binding::container(Str::from(""));
        let _ = TextField::new(&value).line_limit(0);
    }

    #[test]
    fn line_limit_non_one_is_supported() {
        let value = Binding::container(Str::from(""));
        let _ = TextField::new(&value).line_limit(2);
    }

    #[test]
    fn disable_line_limit_does_not_panic() {
        let value = Binding::container(Str::from(""));
        let _ = TextField::new(&value).disable_line_limit();
    }

    #[test]
    fn styled_constructor_accepts_styled_binding() {
        let styled = Binding::container(StyledStr::plain("a"));
        let _ = TextField::styled(&styled);
    }

    #[test]
    #[should_panic(expected = "cannot accept styled text updates")]
    fn plain_mapping_panics_on_styled_write_back() {
        let plain = Binding::container(Str::from("a"));
        let mapped = super::map_plain_binding(&plain);
        mapped.set(StyledStr::plain("b").italic(true));
    }

    #[test]
    fn plain_mapping_updates_nested_mapped_source() {
        let source = Binding::container(String::from("a"));
        let plain = Binding::mapping(&source, Str::from, |binding, value: Str| {
            binding.set(value.into());
        });
        let mapped = super::map_plain_binding(&plain);
        mapped.set(StyledStr::plain("updated"));

        assert_eq!(source.get(), "updated");
    }
}
