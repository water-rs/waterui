//! Structural section container for [`List::content`](super::List::content).
//!
//! [`Section<C>`] groups a sub-tree of [`super::ListContent`] under a header
//! (and optional footer). Its rows form one section of the list's
//! [`super::ListSections`]; backends translate that into native chrome — iOS
//! `UITableView` section header + footer, macOS `NSTableView` group rows,
//! Material section dividers — attached to whichever rows the section holds
//! at the moment.

use nami::Binding;
use waterui_core::View;
use waterui_text::{IntoText, Text};

use super::ListSection;
use super::content::{ContentTags, ListContent, ListScope, SectionScope};
use super::rows::ListItemSink;

/// Groups a sub-tree of [`ListContent`] under an optional header and footer.
///
/// `Section` is structurally typed in its inner content `C`, mirroring the
/// shape of [`Vec<C>`] / `[C; N]` — there is no type erasure and no `Box<dyn>`
/// at the call site. Compose sections with the existing tuple impls of
/// [`ListContent`].
///
/// The content may be static rows, a `ForEach` over a reactive collection, or
/// both. Rows of a reactive collection are inserted and removed as it
/// changes; the header opens whichever row currently leads the section, and
/// a section with no rows draws neither header nor footer until rows return.
///
/// Chain [`Section::expanded`] to make a section collapsible: backends render
/// a native disclosure affordance on its header, animate collapse and expand,
/// expose the expanded state to assistive technology, and preserve tags and
/// selection on collapsed rows.
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
    expanded: Option<Binding<bool>>,
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
            expanded: None,
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
            expanded: None,
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
    pub fn content<NewC: ListContent<SectionScope>>(self, content: NewC) -> Section<NewC> {
        Section {
            label: self.label,
            footer: self.footer,
            expanded: self.expanded,
            content,
        }
    }

    /// Attaches a footer note rendered below the section.
    #[must_use]
    pub fn footer(mut self, footer: impl IntoText) -> Self {
        self.footer = Some(footer.into_text());
        self
    }

    /// Makes this section collapsible, bound to `expanded`.
    ///
    /// Backends realize a native disclosure affordance on the section header,
    /// animate collapse and expand, toggle `expanded` on activation, hide
    /// collapsed rows while preserving their tags and selection state, and
    /// expose the expanded state to assistive technology.
    #[must_use]
    pub fn expanded(mut self, expanded: &Binding<bool>) -> Self {
        self.expanded = Some(expanded.clone());
        self
    }
}

impl<C: ListContent<SectionScope>> ListContent<ListScope> for Section<C> {
    type Row = C::Row;

    fn collect_items<R, W>(self, sink: &mut ListItemSink<R>, wrap: W)
    where
        R: View,
        W: Fn(Self::Row) -> R + Clone + 'static,
    {
        sink.begin_section(ListSection::new(self.label, self.footer, self.expanded));
        self.content.collect_items(sink, wrap);
        sink.end_section();
    }
}

impl<T, C: ContentTags<T>> ContentTags<T> for Section<C> {
    type Presence = C::Presence;
}
