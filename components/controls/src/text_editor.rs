use core::num::NonZeroUsize;

use nami::Binding;
use waterui_core::configurable;
use waterui_layout::StretchAxis;
use waterui_text::{Text, styled::StyledStr};

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

configurable!(
    /// A text editor component that allows users to edit text.
    ///
    /// TextEditor lets users enter and edit text.
    ///
    /// # Layout Behavior
    ///
    /// TextEditor **expands horizontally** to fill available space, but has a fixed height.
    /// In an `HStack`, it will take up all remaining width after other views are sized.
    ///
    RichTextEditor, RichTextEditorConfig, StretchAxis::Horizontal);

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
    /// By default, the line limit is 1.
    #[must_use]
    pub const fn line_limit(mut self, line_limit: usize) -> Self {
        self.0.line_limit = NonZeroUsize::new(line_limit);
        self
    }

    /// Disables the line limit.
    #[must_use]
    pub const fn disable_line_limit(mut self) -> Self {
        self.0.line_limit = None;
        self
    }
}
