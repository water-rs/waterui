//! Closed-collection trait and ergonomic row helpers for static list content.
//!
//! [`ListContent`] mirrors the closed-collection pattern used by `MenuView`
//! (see `components/controls/src/menu.rs`): every value type the user can
//! drop into [`crate::component::list::List::content`] explicitly opts in
//! through this trait. Implementations exist for [`super::ListItem`], the
//! [`Row`] builder produced by [`row`] / [`detail_row`], [`super::Section`],
//! tuples up to 15 elements, fixed-size arrays, [`Vec<T>`], and `Option<T>`.
//!
//! The accumulator design (`collect_items(self, sink)`) lets `Section<C>`
//! attach its header / footer marker to whichever item the inner content
//! produces first, without forcing each implementation to allocate an
//! intermediate `Vec`.

use alloc::vec::Vec;

use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;

use waterui_controls::label::{IntoLabel, Label};
use waterui_layout::{
    padding::EdgeInsets,
    spacer::spacer,
    stack::{HorizontalAlignment, hstack, vstack},
};
use waterui_text::{IntoText, Text};

use crate::Color;
use crate::view::ViewExt;

use super::{ListItem, ListSection};

// ============================================================================
// ListContent trait + sink
// ============================================================================

/// Converts semantic list content into a flat sequence of [`ListItem`]
/// builders.
///
/// Implementations are limited to building blocks the framework explicitly
/// blesses (see the module docs). This keeps `List::content` a closed
/// surface in the same spirit as the `MenuView` trait, so user code cannot
/// leak arbitrary `View` types into list rows by accident.
pub trait ListContent {
    /// Pushes the items produced by this content into `sink`.
    fn collect_items(self, sink: &mut ListItemSink);
}

/// Accumulator handed to [`ListContent::collect_items`].
///
/// Stores [`ListItem`] *builders* (not finished items) so that the [`Views`]
/// adapter behind `List::content` can reproduce a fresh [`ListItem`] every
/// time the framework requests it. Each entry also carries an optional
/// [`ListSection`] marker attached by [`super::Section`].
///
/// [`Views`]: crate::views::Views
pub struct ListItemSink {
    entries: Vec<(AnyViewBuilder<ListItem>, Option<ListSection>)>,
}

impl Default for ListItemSink {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for ListItemSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListItemSink")
            .field("len", &self.entries.len())
            .finish()
    }
}

impl ListItemSink {
    /// Creates an empty sink.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Returns the number of entries collected so far.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the sink has collected no entries yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Pushes one entry built from a cloneable builder.
    pub fn push(&mut self, builder: AnyViewBuilder<ListItem>) {
        self.entries.push((builder, None));
    }

    /// Attaches a section marker to the entry at `index`.
    ///
    /// Used by [`super::Section`] to tag the first item produced by its
    /// inner content. Subsequent items without their own marker render
    /// inside the same section.
    pub(crate) fn set_section(&mut self, index: usize, section: ListSection) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.1 = Some(section);
        }
    }

    /// Consumes the sink and returns the collected entries in order.
    pub(crate) fn into_entries(self) -> Vec<(AnyViewBuilder<ListItem>, Option<ListSection>)> {
        self.entries
    }
}

// ----------------------------------------------------------------------------
// Leaf impls
// ----------------------------------------------------------------------------

impl ListContent for () {
    fn collect_items(self, _sink: &mut ListItemSink) {}
}

impl<T: ListContent> ListContent for Option<T> {
    fn collect_items(self, sink: &mut ListItemSink) {
        if let Some(content) = self {
            content.collect_items(sink);
        }
    }
}

impl<T: ListContent> ListContent for Vec<T> {
    fn collect_items(self, sink: &mut ListItemSink) {
        for item in self {
            item.collect_items(sink);
        }
    }
}

impl<T: ListContent, const N: usize> ListContent for [T; N] {
    fn collect_items(self, sink: &mut ListItemSink) {
        for item in self {
            item.collect_items(sink);
        }
    }
}

/// Closures that return a fresh [`ListItem`] are valid leaf content. This is
/// the escape hatch for embedding arbitrary [`ListItem`] producers — e.g.
/// pre-built items from another module — without forcing them through
/// [`Row`]. The closure is called by the framework whenever the row needs to
/// be (re)materialized, so it must be `Fn`, not `FnOnce`.
impl<F> ListContent for F
where
    F: Fn() -> ListItem + 'static,
{
    fn collect_items(self, sink: &mut ListItemSink) {
        sink.push(AnyViewBuilder::new(self));
    }
}

// ----------------------------------------------------------------------------
// Tuple impls (1..=15)
// ----------------------------------------------------------------------------

macro_rules! list_content_tuples {
    ($macro:ident) => {
        $macro!(T0);
        $macro!(T0, T1);
        $macro!(T0, T1, T2);
        $macro!(T0, T1, T2, T3);
        $macro!(T0, T1, T2, T3, T4);
        $macro!(T0, T1, T2, T3, T4, T5);
        $macro!(T0, T1, T2, T3, T4, T5, T6);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
        $macro!(
            T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14
        );
    };
}

macro_rules! impl_tuple_list_content {
    ($($T:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($T: ListContent),+> ListContent for ($($T,)+) {
            fn collect_items(self, sink: &mut ListItemSink) {
                let ($($T,)+) = self;
                $(
                    $T.collect_items(sink);
                )+
            }
        }
    };
}

list_content_tuples!(impl_tuple_list_content);

// ============================================================================
// Row helpers
// ============================================================================

/// Visual layout chosen for a [`Row`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowLayout {
    /// `label …………… value` on one line. Default for [`row`].
    Inline,
    /// Two-line layout: `label` on top, full-width `value` underneath.
    /// Default for [`detail_row`].
    Detail,
}

/// Builder produced by [`row`] / [`detail_row`].
///
/// Composes a label + value pair using the same padding / spacing the
/// inset-grouped list style expects, and exposes a small fluent surface for
/// the customizations callers actually need (value foreground color, layout
/// switch, deletability).
#[derive(Clone)]
pub struct Row {
    label: Label,
    value: Text,
    layout: RowLayout,
    value_color: Option<Color>,
    deletable: Option<bool>,
}

impl core::fmt::Debug for Row {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Row")
            .field("layout", &self.layout)
            .field("has_value_color", &self.value_color.is_some())
            .field("deletable_override", &self.deletable)
            .finish_non_exhaustive()
    }
}

impl Row {
    /// Switches this row to the two-line [`RowLayout::Detail`] layout, where
    /// the value renders below the label and stretches to the full row width.
    #[must_use]
    pub const fn detail(mut self) -> Self {
        self.layout = RowLayout::Detail;
        self
    }

    /// Switches this row back to the single-line [`RowLayout::Inline`] layout.
    #[must_use]
    pub const fn inline(mut self) -> Self {
        self.layout = RowLayout::Inline;
        self
    }

    /// Sets the foreground color applied to the value view.
    ///
    /// Theme tokens like `Accent` and `MutedForeground` work here because
    /// they implement `Into<Color>` via the theme module.
    #[must_use]
    pub fn value_color(mut self, color: impl Into<Color>) -> Self {
        self.value_color = Some(color.into());
        self
    }

    /// Overrides the deletability of the resulting [`ListItem`].
    #[must_use]
    pub const fn deletable(mut self, deletable: bool) -> Self {
        self.deletable = Some(deletable);
        self
    }

    fn build_item(&self) -> ListItem {
        let label = self.label.clone();
        let value = self.value.clone();
        let value_view: AnyView = match self.value_color.clone() {
            Some(color) => AnyView::new(value.foreground(color)),
            None => AnyView::new(value),
        };
        ListItem::new(self.assemble(label, value_view))
    }

    fn assemble(&self, label: Label, value: AnyView) -> AnyView {
        match self.layout {
            RowLayout::Inline => AnyView::new(
                hstack((label, spacer(), value))
                    .spacing(12.0)
                    .padding_with(EdgeInsets::symmetric(10.0, 16.0)),
            ),
            RowLayout::Detail => AnyView::new(
                vstack((label, value))
                    .alignment(HorizontalAlignment::Leading)
                    .spacing(6.0)
                    .padding_with(EdgeInsets::symmetric(10.0, 16.0)),
            ),
        }
    }
}

impl ListContent for Row {
    fn collect_items(self, sink: &mut ListItemSink) {
        let row = self;
        let deletable_override = row.deletable;
        let builder = AnyViewBuilder::new(move || {
            let mut item = row.build_item();
            if let Some(d) = deletable_override {
                item = item.deletable(d);
            }
            item
        });
        sink.push(builder);
    }
}

/// Builds a single-line key/value [`Row`].
///
/// The label uses [`IntoLabel`] so string literals enter the i18n pipeline
/// and accessibility tree as semantic labels (see
/// `components/controls/src/label.rs`); the value uses [`IntoText`] so it
/// participates in the same translation / styling story. Use
/// [`Row::value_color`] to tint the value with a theme token, and
/// [`Row::detail`] to switch to the multi-line layout used for free-form
/// payloads such as truncating timestamps or backtraces.
#[must_use]
pub fn row(label: impl IntoLabel, value: impl IntoText) -> Row {
    Row {
        label: label.into_label(),
        value: value.into_text(),
        layout: RowLayout::Inline,
        value_color: None,
        deletable: None,
    }
}

/// Builds a two-line [`Row`] with the value stacked below the label and
/// stretched to the full row width. Equivalent to [`Row::detail`].
#[must_use]
pub fn detail_row(label: impl IntoLabel, value: impl IntoText) -> Row {
    row(label, value).detail()
}
