//! Structural section container for [`List::content`](super::List::content).
//!
//! [`Section<C>`] groups a sub-tree of [`super::ListContent`] under a header
//! (and optional footer). It compiles down to the existing wire-format on
//! [`super::ListItem`] (`section: Option<ListSection>`), tagging the first
//! item produced by its inner content. Backends translate that marker into
//! native chrome — iOS `UITableView` section header + footer, macOS
//! `NSTableView` group rows, Material section dividers — without any extra
//! FFI surface.

use waterui_text::{IntoText, Text};

use super::ListSection;
use super::content::{ListContent, ListItemSink};

/// Groups a sub-tree of [`ListContent`] under an optional header and footer.
///
/// `Section` is structurally typed in its inner content `C`, mirroring the
/// shape of [`Vec<C>`] / `[C; N]` — there is no type erasure and no `Box<dyn>`
/// at the call site. Compose sections with the existing tuple impls of
/// [`ListContent`].
///
/// The header and footer are semantic text, so a string literal localizes and
/// a signal-backed title updates in place: the backend repaints the chrome
/// without rebuilding the section's rows.
///
/// # Example
///
/// ```rust
/// use waterui::component::list::{List, Section, row};
/// use waterui::prelude::*;
///
/// let stalls = binding::<i32>(0);
/// let list = List::content((
///     Section::new("Connection").content((
///         row("Status", text!("Connected")),
///         row("Endpoint", text!("wss://example.invalid")),
///     )),
///     Section::new(text!("Activity ({stalls} stalled)"))
///         .footer("Updated every poll")
///         .content((
///             row("Polls", text!("12")),
///             row("Stalls", text!("0"))
///         ))
/// ));
/// ```
#[derive(Debug, Clone)]
pub struct Section<C> {
    label: Option<Text>,
    footer: Option<Text>,
    content: C,
}

impl Section<()> {
    /// Creates a section with the given header and no content yet.
    ///
    /// Chain [`Section::content`] to attach inner [`ListContent`].
    #[must_use]
    pub fn new(label: impl IntoText) -> Self {
        Self {
            label: Some(label.into_text()),
            footer: None,
            content: (),
        }
    }

    /// Creates a section break with no header — purely a visual / semantic
    /// divider — that still lets backends render their group chrome around
    /// the contained items.
    #[must_use]
    pub const fn unlabeled() -> Self {
        Self {
            label: None,
            footer: None,
            content: (),
        }
    }
}

impl<C> Section<C> {
    /// Sets the inner content for this section.
    ///
    /// Returns a new `Section<NewC>` so the inner type is preserved without
    /// type erasure; the old content is dropped if it had not been used.
    #[must_use]
    pub fn content<NewC: ListContent>(self, content: NewC) -> Section<NewC> {
        Section {
            label: self.label,
            footer: self.footer,
            content,
        }
    }

    /// Attaches a footer note rendered below the section.
    #[must_use]
    pub fn footer(mut self, footer: impl IntoText) -> Self {
        self.footer = Some(footer.into_text());
        self
    }
}

impl<C: ListContent> ListContent for Section<C> {
    fn collect_items(self, sink: &mut ListItemSink) {
        let start = sink.len();
        self.content.collect_items(sink);
        // Tag the first item produced by the inner content. If the content
        // produced no items, the section is silently dropped — this matches
        // how SwiftUI renders an empty Section{}.
        if sink.len() > start {
            sink.set_section(
                start,
                ListSection {
                    label: self.label,
                    footer: self.footer,
                },
            );
        }
    }
}
