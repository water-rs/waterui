//! Closed-collection trait and ergonomic row helpers for list content.
//!
//! [`ListContent`] mirrors the closed-collection pattern used by `MenuView`
//! (see `components/controls/src/menu.rs`): every value type the user can
//! drop into [`crate::component::list::List::content`] explicitly opts in
//! through this trait. Implementations exist for any `Fn() -> ListItem` row
//! builder, the [`Row`] builder produced by [`row`] / [`detail_row`], a
//! [`ForEach`] over a reactive collection, [`super::Section`], tuples up to
//! 15 elements, fixed-size arrays, [`Vec<T>`], and `Option<T>`.
//!
//! Static rows are *builders*, not finished [`super::ListItem`] values,
//! because the list rematerializes a row every time it needs one — write
//! `|| ListItem::new(…)`, not `ListItem::new(…)`. A [`ForEach`] keeps its
//! collection's identity and follows it row by row.
//!
//! The scope parameter keeps sections flat: a [`super::Section`] is
//! [`ListContent`] of the list ([`ListScope`]) but not of another section
//! ([`SectionScope`]), so nesting one section in another does not compile.

use alloc::vec::Vec;

use nami::collection::Collection;
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
use crate::views::ForEach;

use super::ListItem;
use super::rows::ListItemSink;

// ============================================================================
// ListContent trait
// ============================================================================

/// Content of the list itself: rows and [`super::Section`]s.
#[derive(Debug)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a marker type that carries no data"
)]
pub struct ListScope;

/// Content of a [`super::Section`]: rows, but no further sections.
#[derive(Debug)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a marker type that carries no data"
)]
pub struct SectionScope;

/// Converts semantic list content into the rows and sections a list shows.
///
/// `Scope` is where the content may appear: [`ListScope`] (the default) for
/// the content of [`List::content`](super::List::content), [`SectionScope`]
/// for the content of a [`super::Section`]. Rows are content in both;
/// a section only in the list's.
///
/// Implementations are limited to building blocks the framework explicitly
/// blesses (see the module docs). This keeps `List::content` a closed
/// surface in the same spirit as the `MenuView` trait, so user code cannot
/// leak arbitrary `View` types into list rows by accident.
pub trait ListContent<Scope = ListScope> {
    /// Pushes the rows produced by this content into `sink`.
    fn collect_items(self, sink: &mut ListItemSink);
}

// ----------------------------------------------------------------------------
// Leaf impls
// ----------------------------------------------------------------------------

impl<S> ListContent<S> for () {
    fn collect_items(self, _sink: &mut ListItemSink) {}
}

impl<S, T: ListContent<S>> ListContent<S> for Option<T> {
    fn collect_items(self, sink: &mut ListItemSink) {
        if let Some(content) = self {
            content.collect_items(sink);
        }
    }
}

impl<S, T: ListContent<S>> ListContent<S> for Vec<T> {
    fn collect_items(self, sink: &mut ListItemSink) {
        for item in self {
            item.collect_items(sink);
        }
    }
}

impl<S, T: ListContent<S>, const N: usize> ListContent<S> for [T; N] {
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
impl<S, F> ListContent<S> for F
where
    F: Fn() -> ListItem + 'static,
{
    fn collect_items(self, sink: &mut ListItemSink) {
        sink.push(AnyViewBuilder::new(self));
    }
}

/// A [`ForEach`] over a reactive collection contributes one row per item,
/// keyed by the item's id. The rows follow the collection: items it inserts,
/// removes or moves are inserted, removed or moved in place, and inside a
/// [`super::Section`] the header opens whichever row currently leads it.
impl<S, C, F> ListContent<S> for ForEach<C, F, ListItem>
where
    C: Collection + Clone + 'static,
    C::Item: Clone,
    F: Fn(C::Item) -> ListItem + 'static,
{
    fn collect_items(self, sink: &mut ListItemSink) {
        sink.push_rows(self);
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
        impl<S, $($T: ListContent<S>),+> ListContent<S> for ($($T,)+) {
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
#[non_exhaustive]
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

impl<S> ListContent<S> for Row {
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
