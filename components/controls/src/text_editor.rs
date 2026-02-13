use core::num::NonZeroUsize;

use nami::Binding;
use waterui_core::{layout::StretchAxis, View};
use waterui_text::{styled::StyledStr, Text};

use crate::text_field::TextField;

/// Configuration for the rich text editor component.
#[derive(Debug)]
pub struct RichTextEditorConfig {
    /// The binding to the text value being edited.
    pub value: Binding<StyledStr>,
    /// Placeholder text to display when the editor is empty.
    pub placeholder: Text,
    /// Optional line limit for the editor.
    pub line_limit: Option<NonZeroUsize>,
}

/// A text editor component that allows users to edit text.
///
/// The current implementation is built on `TextField` (native single-line control)
/// and stores content as `StyledStr::plain(...)` on user edits.
///
/// # Layout Behavior
///
/// TextEditor **expands horizontally** to fill available space, but has a fixed height.
/// In an `HStack`, it will take up all remaining width after other views are sized.
#[derive(Debug)]
pub struct RichTextEditor(RichTextEditorConfig);

/// Alias for `RichTextEditor`.
///
/// Kept for naming consistency with roadmap/docs that use `RichTextField`.
pub type RichTextField = RichTextEditor;

impl RichTextEditor {
    /// Creates a new [`RichTextEditor`] with the given value binding.
    #[must_use]
    pub fn new(value: &Binding<StyledStr>) -> Self {
        Self(RichTextEditorConfig {
            value: value.clone(),
            placeholder: Text::default(),
            line_limit: NonZeroUsize::new(1),
        })
    }

    /// Sets the placeholder text for the text editor.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<Text>) -> Self {
        self.0.placeholder = placeholder.into();
        self
    }

    /// Sets the maximum number of lines to show.
    ///
    /// Currently only `1` is supported across backends.
    ///
    /// # Panics
    ///
    /// Panics if `line_limit` is `0` or not `1`.
    #[must_use]
    pub fn line_limit(mut self, line_limit: usize) -> Self {
        assert!(line_limit > 0, "Line limit must be greater than 0");
        assert_eq!(
            line_limit, 1,
            "RichTextEditor multi-line editing is not implemented yet"
        );
        self.0.line_limit = NonZeroUsize::new(line_limit);
        self
    }

    /// Disables the line limit.
    #[must_use]
    pub fn disable_line_limit(self) -> Self {
        panic!("RichTextEditor multi-line editing is not implemented yet");
    }
}

impl View for RichTextEditor {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let config = self.0;
        let mut text_field = TextField::styled(&config.value).prompt(config.placeholder);
        if let Some(line_limit) = config.line_limit {
            text_field = text_field.line_limit(line_limit.get());
        }

        text_field
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_text::styled::StyledStr;

    use super::RichTextEditor;

    #[test]
    #[should_panic(expected = "Line limit must be greater than 0")]
    fn line_limit_zero_panics() {
        let value = Binding::container(StyledStr::default());
        let _ = RichTextEditor::new(&value).line_limit(0);
    }

    #[test]
    #[should_panic(expected = "multi-line editing is not implemented yet")]
    fn line_limit_non_one_panics() {
        let value = Binding::container(StyledStr::default());
        let _ = RichTextEditor::new(&value).line_limit(2);
    }

    #[test]
    #[should_panic(expected = "multi-line editing is not implemented yet")]
    fn disable_line_limit_panics() {
        let value = Binding::container(StyledStr::default());
        let _ = RichTextEditor::new(&value).disable_line_limit();
    }
}
