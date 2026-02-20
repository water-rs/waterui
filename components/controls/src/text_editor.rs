use core::num::NonZeroUsize;

use nami::Binding;
use waterui_core::{View, layout::StretchAxis};
use waterui_text::{Text, styled::StyledStr};

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
/// The current implementation is built on the native `TextField` control path
/// and edits a `Binding<StyledStr>` directly.
///
/// # Layout Behavior
///
/// TextEditor **expands horizontally** to fill available space, but has a fixed height.
/// In an `HStack`, it will take up all remaining width after other views are sized.
#[derive(Debug)]
pub struct RichTextEditor(RichTextEditorConfig);

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
    /// # Panics
    ///
    /// Panics if `line_limit` is `0`.
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
}

impl View for RichTextEditor {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // TODO(rich-text-editor): replace this bridge with a dedicated rich-text editor surface
        // (multi-line layout, attachment model, rich selection/editing commands, and media spans).
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
    fn line_limit_non_one_is_supported() {
        let value = Binding::container(StyledStr::default());
        let _ = RichTextEditor::new(&value).line_limit(2);
    }

    #[test]
    fn disable_line_limit_does_not_panic() {
        let value = Binding::container(StyledStr::default());
        let _ = RichTextEditor::new(&value).disable_line_limit();
    }
}
