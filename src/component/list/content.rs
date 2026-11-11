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
//!
//! A static row becomes selectable by tagging it with the value that selects
//! it — `(|| ListItem::new(…)).tag(value)` through
//! [`ViewExt::tag`](crate::ViewExt::tag), or [`Row::tag`] — and a `ForEach`
//! row is selected by its item's id. [`ContentTags`] states, per selection
//! type, which content a list may select by it and whether any of its rows
//! is selected by it; [`TaggedBy`] names content that is.

use alloc::vec::Vec;

use nami::collection::Collection;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::TaggedView;

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

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Tagged {}
    impl Sealed for super::Untagged {}
}

/// Whether list content holds a row selected by the list's selection type:
/// [`Tagged`] or [`Untagged`].
pub trait TagPresence: sealed::Sealed {
    /// [`Tagged`] when `Self` or `Other` is: the presence of content that
    /// holds both.
    type Or<Other: TagPresence>: TagPresence;
}

/// The content holds at least one row selected by the selection type.
#[derive(Debug)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a marker type that carries no data"
)]
pub struct Tagged;

/// The content holds no row selected by the selection type: only untagged
/// rows, which are not selectable.
#[derive(Debug)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a marker type that carries no data"
)]
pub struct Untagged;

impl TagPresence for Tagged {
    type Or<Other: TagPresence> = Self;
}

impl TagPresence for Untagged {
    type Or<Other: TagPresence> = Other;
}

/// A [`TagPresence`] that holds a selectable row: only [`Tagged`].
///
/// A list built by [`List::content`](super::List::content) is selectable
/// by `T` only when its content's [`ContentTags<T>`] presence is `HoldsTag`,
/// so a selection over rows none of which can be selected fails to compile.
#[diagnostic::on_unimplemented(
    message = "this list has no selectable row: every row is untagged",
    label = "a selection over untagged rows could never select one",
    note = "tag a static row with the value that selects it, `.tag(value)` with `value` of the selection's type, or add a `ForEach`, whose rows are selected by their items' ids; an untagged row is not selectable"
)]
pub trait HoldsTag: TagPresence {}

impl HoldsTag for Tagged {}

/// What the rows of list content are selected by, checked against the
/// selection type `T`.
///
/// Content is `ContentTags<T>` when every selectable row in it is selected
/// by `T`: a static row tagged with `.tag(value)` is `ContentTags` of the
/// value's type, a [`ForEach`] row of its item's id type, and an untagged
/// static row of every type, because it is not selectable. Sections, tuples,
/// arrays, vectors and `Option` are `ContentTags<T>` when everything they
/// hold is. [`Presence`](Self::Presence) records whether any row is selected
/// by `T`: a tagged row and a `ForEach` are [`Tagged`], an untagged row is
/// [`Untagged`], and content holding several parts is `Tagged` when any part
/// is.
///
/// A list built by [`List::content`](super::List::content) accepts a
/// selection of `T` only when its content is `ContentTags<T>` with a
/// [`Tagged`] presence (see [`TaggedBy`]), so a selection type the rows do
/// not carry, or rows that carry none, fail to compile.
///
/// An implementation for custom [`ListContent`] must agree with the rows it
/// collects: a row tagged by another type panics when the list's selection
/// is set.
#[diagnostic::on_unimplemented(
    message = "`{Self}` holds rows that are not selected by `{T}`",
    label = "rows not selected by `{T}`",
    note = "a `ForEach` row is selected by its item's id and a static row by the value given to `.tag(…)`; every selectable row of one list is selected by the same type"
)]
pub trait ContentTags<T> {
    /// [`Tagged`] when the content holds a row selected by `T`, otherwise
    /// [`Untagged`].
    type Presence: TagPresence;
}

/// List content a list can select by `T`: every selectable row is selected
/// by `T`, and at least one row is.
///
/// It names such content where its type cannot be spelled, as in
/// `fn album_row(album: Album) -> impl ListContent<SectionScope> + TaggedBy<Album>`;
/// every [`ContentTags<T>`] content with a [`Tagged`] presence is
/// `TaggedBy<T>`.
pub trait TaggedBy<T>: ContentTags<T, Presence = Tagged> {}

impl<T, C: ContentTags<T, Presence = Tagged> + ?Sized> TaggedBy<T> for C {}

// ----------------------------------------------------------------------------
// Leaf impls
// ----------------------------------------------------------------------------

impl<S> ListContent<S> for () {
    fn collect_items(self, _sink: &mut ListItemSink) {}
}

impl<T> ContentTags<T> for () {
    type Presence = Untagged;
}

impl<S, C: ListContent<S>> ListContent<S> for Option<C> {
    fn collect_items(self, sink: &mut ListItemSink) {
        if let Some(content) = self {
            content.collect_items(sink);
        }
    }
}

impl<T, C: ContentTags<T>> ContentTags<T> for Option<C> {
    type Presence = C::Presence;
}

impl<S, C: ListContent<S>> ListContent<S> for Vec<C> {
    fn collect_items(self, sink: &mut ListItemSink) {
        for item in self {
            item.collect_items(sink);
        }
    }
}

impl<T, C: ContentTags<T>> ContentTags<T> for Vec<C> {
    type Presence = C::Presence;
}

impl<S, C: ListContent<S>, const N: usize> ListContent<S> for [C; N] {
    fn collect_items(self, sink: &mut ListItemSink) {
        for item in self {
            item.collect_items(sink);
        }
    }
}

impl<T, C: ContentTags<T>, const N: usize> ContentTags<T> for [C; N] {
    type Presence = C::Presence;
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

/// An untagged row builder is not selectable, so it fits a list selected by
/// any type and gives it nothing to select.
impl<T, F> ContentTags<T> for F
where
    F: Fn() -> ListItem + 'static,
{
    type Presence = Untagged;
}

/// A row builder tagged with [`ViewExt::tag`](crate::ViewExt::tag) is
/// selected by its tag: `(|| ListItem::new(…)).tag(value)`.
impl<S, T, F> ListContent<S> for TaggedView<T, F>
where
    T: 'static,
    F: Fn() -> ListItem + 'static,
{
    fn collect_items(self, sink: &mut ListItemSink) {
        sink.push_tagged(AnyViewBuilder::new(self.content), self.tag);
    }
}

impl<T, F> ContentTags<T> for TaggedView<T, F>
where
    F: Fn() -> ListItem + 'static,
{
    type Presence = Tagged;
}

/// A [`ForEach`] over a reactive collection contributes one row per item,
/// keyed and selected by the item's id. The rows follow the collection:
/// items it inserts, removes or moves are inserted, removed or moved in
/// place, and inside a [`super::Section`] the header opens whichever row
/// currently leads it.
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

impl<C, F> ContentTags<C::Id> for ForEach<C, F, ListItem>
where
    C: Collection,
    F: Fn(C::Item) -> ListItem,
{
    type Presence = Tagged;
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

        impl<Tag, $($T: ContentTags<Tag>),+> ContentTags<Tag> for ($($T,)+) {
            type Presence = any_tagged!(Tag; $($T),+);
        }
    };
}

/// The [`TagPresence`] of a tuple of `$T`s selected by `$Tag`: [`Tagged`]
/// when any element is.
macro_rules! any_tagged {
    ($Tag:ident; $T:ident) => {
        <$T as ContentTags<$Tag>>::Presence
    };
    ($Tag:ident; $T:ident, $($rest:ident),+) => {
        <<$T as ContentTags<$Tag>>::Presence as TagPresence>::Or<any_tagged!($Tag; $($rest),+)>
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

    /// Makes the row selectable by `tag`: a list whose selection holds `tag`
    /// shows this row selected, and selecting the row writes `tag`.
    #[must_use]
    pub const fn tag<T>(self, tag: T) -> TaggedView<T, Self> {
        TaggedView { tag, content: self }
    }

    /// The builder the list calls whenever it realizes this row.
    fn into_builder(self) -> AnyViewBuilder<ListItem> {
        AnyViewBuilder::new(move || {
            let item = self.build_item();
            match self.deletable {
                Some(deletable) => item.deletable(deletable),
                None => item,
            }
        })
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
        sink.push(self.into_builder());
    }
}

/// An untagged [`Row`] is not selectable, so it fits a list selected by any
/// type and gives it nothing to select.
impl<T> ContentTags<T> for Row {
    type Presence = Untagged;
}

/// A [`Row`] tagged with [`Row::tag`] is selected by its tag.
impl<S, T: 'static> ListContent<S> for TaggedView<T, Row> {
    fn collect_items(self, sink: &mut ListItemSink) {
        sink.push_tagged(self.content.into_builder(), self.tag);
    }
}

impl<T> ContentTags<T> for TaggedView<T, Row> {
    type Presence = Tagged;
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
