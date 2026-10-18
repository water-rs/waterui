//! The selection a `selection_menu` command acts on.
//!
//! A [`TextField`](crate::TextField)'s `.selection_menu` rows run with the
//! field's live selection installed as a [`TextSelection`] in their
//! environment. A command declares it as a handler parameter —
//! `|selection: TextSelection| { selection.replace("**…**") }` — or
//! tolerates its absence with `Option<TextSelection>`.
//!
//! The backend that owns the field's editing state implements
//! [`TextSelectionImpl`] and installs the handle while building the
//! selection menu; application code only consumes it.

use alloc::borrow::ToOwned;
use alloc::rc::Rc;
use core::ops::Range;

use nami::Binding;
use waterui_core::Str;
use waterui_text::styled::StyledStr;

/// The live selection of the field a `selection_menu` command fired on.
///
/// The handle answers at call time, so a command never works on a
/// snapshot the user has already moved past. Every range is a pair of
/// byte offsets into [`Self::text`]; both edges sit on UTF-8 boundaries.
#[derive(Clone)]
pub struct TextSelection {
    driver: Rc<dyn TextSelectionImpl>,
}

impl core::fmt::Debug for TextSelection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextSelection")
            .field("range", &self.range())
            .finish_non_exhaustive()
    }
}

impl TextSelection {
    /// The backend half of the contract: wraps the field's selection driver
    /// for installation into a selection-menu environment. Backends call
    /// this; application code never does.
    #[doc(hidden)]
    pub fn from_driver(driver: Rc<dyn TextSelectionImpl>) -> Self {
        Self { driver }
    }

    /// The field's complete plain text.
    #[must_use]
    pub fn text(&self) -> Str {
        self.driver.text()
    }

    /// The selected byte range of `text()` — an empty range is a caret.
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.driver.range()
    }

    /// The selected plain text — `text()[range()]`; empty for a caret.
    #[must_use]
    pub fn selected_text(&self) -> Str {
        let text = self.text();
        text.get(self.range())
            .map_or_else(Str::default, |slice| Str::from(slice.to_owned()))
    }

    /// Replaces the selection with `inserted`, collapsing the caret to the
    /// end of the inserted text — the same normalization a typed
    /// replacement goes through (a line-limited field never grows a line).
    pub fn replace(&self, inserted: &str) {
        let range = self.driver.range();
        self.driver.replace_range(range, inserted);
    }

    /// Moves the selection; an empty range collapses to a caret.
    pub fn select(&self, range: Range<usize>) {
        self.driver.select(range);
    }

    /// The field's styled-text binding when it has one — the escape hatch
    /// for commands that re-style a span rather than replace text. A plain
    /// field always has one; a secure field never does.
    #[must_use]
    pub fn styled_value(&self) -> Option<Binding<StyledStr>> {
        self.driver.styled_value()
    }
}

/// What a backend's text field reports and accepts for a [`TextSelection`].
///
/// Implemented once per backend that realizes text fields; commands hold
/// only the `TextSelection` handle that delegates to it. All ranges are
/// byte offsets into `text()`.
pub trait TextSelectionImpl: 'static {
    /// The field's complete plain text.
    fn text(&self) -> Str;

    /// The selected byte range of `text()`; an empty range is a caret.
    fn range(&self) -> Range<usize>;

    /// Replaces `range` in `text()` with `inserted`, then collapses the
    /// selection to a caret after the inserted text.
    fn replace_range(&self, range: Range<usize>, inserted: &str);

    /// Selects `range` — or sets the caret when `range` is empty.
    fn select(&self, range: Range<usize>);

    /// The field's styled-text binding when it has one.
    fn styled_value(&self) -> Option<Binding<StyledStr>> {
        None
    }
}

waterui_core::impl_extractor!(TextSelection);
