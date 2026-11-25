//! List component implementation for `WaterUI`.
//!
//! This module provides the necessary components to build and configure lists
//! in the `WaterUI` framework. It includes the `List` component for displaying
//! collections of data, row modifiers ([`ListRowDeletable`], [`ListRowInsets`],
//! [`ListMinRowHeight`]), and the [`ListContent`] / [`Section`] / [`row`]
//! surface for composing heterogeneous lists with sections, static or reactive.

use alloc::collections::BTreeSet;
use nami::collection::Collection;
use nami::{Binding, Computed, signal::IntoComputed};

use crate::component::selection_support::{SelectionThemed, WithId};
use crate::views::{AnyViews, ForEach, SharedAnyViews, Views};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::id::Mapping;
use waterui_core::{
    AnyView, Environment, Native, NativeView, View, handler::Handler, layout::StretchAxis,
};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::scroll::ScrollController;
use waterui_text::Text;

mod content;
mod edits;
mod rows;
mod section;
mod selection;

pub use content::{
    ContentTags, HoldsTag, ListContent, ListScope, Row, RowContent, RowLayout, SectionScope,
    TagPresence, Tagged, TaggedBy, Untagged, detail_row, row,
};
pub use edits::{EditableForEach, EditableRows, ForEachExt, ListDelete, ListEdits, ListMove, Move};
use edits::{RowEdits, TypedOnDelete, TypedOnMove, list_delete_action, list_move_action};
use rows::RowSections;
pub use rows::{
    ContentRows, ContentRowsGuard, ContentRowsSnapshot, ListItemSink, ListSectionSpan, ListSections,
};
pub use section::Section;
use selection::RowTags;
pub use selection::{ListRowTags, ListSelection, ListTagId, SelectableRows};

/// Configuration for a list component.
///
/// `C` is the content every row carries: the rows' own view type up to
/// dispatch, then the backend's child slot.
#[non_exhaustive]
pub struct ListConfig<C> {
    /// Content items to be displayed in the list.
    contents: SharedAnyViews<C>,
    /// The list's selection bindings, keyed by the erased tags `tags`
    /// answers for the rows. `ListSelection::None` when the list is not
    /// selectable.
    selection: ListSelection<ListTagId>,
    /// The tag each row is selected by, keyed by the same row ids
    /// `contents.get_id` returns.
    tags: ListRowTags,
    /// Read-only signal for edit mode state.
    editing: Computed<bool>,
    /// Per-row delete and per-part move routing, keyed by the same row ids
    /// `contents.get_id` returns.
    edits: ListEdits,
    /// Optional programmatic item-index scroll controller.
    scroll_controller: Option<ScrollController<usize>>,
    /// The sections `contents` groups into, keyed by the same row ids.
    sections: ListSections,
    /// The minimum height of a row, from [`ListMinRowHeight`] in the list's
    /// environment. `None` uses the theme's one-line row height.
    min_row_height: Option<f32>,
}

impl<C> ListConfig<C> {
    /// Content items to be displayed in the list.
    #[must_use]
    pub const fn contents(&self) -> &SharedAnyViews<C> {
        &self.contents
    }

    /// The list's selection bindings, keyed by the erased tags
    /// [`ListConfig::row_tags`] answers for the rows: a row is selected when
    /// its tag is in the selection, and selecting it writes its tag.
    /// `ListSelection::None` when the list is not selectable.
    #[must_use]
    pub const fn selection(&self) -> &ListSelection<ListTagId> {
        &self.selection
    }

    /// The tag each row is selected by, keyed by the same row ids
    /// `contents.get_id` returns. A row without a tag is not selectable.
    #[must_use]
    pub const fn row_tags(&self) -> &ListRowTags {
        &self.tags
    }

    /// Read-only signal for edit mode state.
    #[must_use]
    pub const fn editing(&self) -> &Computed<bool> {
        &self.editing
    }

    /// Per-row delete and per-part move routing, keyed by the same row ids
    /// `contents.get_id` returns.
    #[must_use]
    pub const fn edits(&self) -> &ListEdits {
        &self.edits
    }

    /// Optional programmatic item-index scroll controller.
    #[must_use]
    pub const fn scroll_controller(&self) -> Option<&ScrollController<usize>> {
        self.scroll_controller.as_ref()
    }

    /// The sections `contents` groups into, keyed by the same row ids
    /// `contents.get_id` returns. A list built without sections is one
    /// section without chrome.
    #[must_use]
    pub const fn sections(&self) -> &ListSections {
        &self.sections
    }

    /// The minimum height of a row, from [`ListMinRowHeight`] in the list's
    /// environment. `None` uses the theme's one-line row height.
    #[must_use]
    pub const fn min_row_height(&self) -> Option<f32> {
        self.min_row_height
    }
}

impl<C> core::fmt::Debug for ListConfig<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(core::any::type_name::<Self>())
    }
}

/// Moves every row's content into the slot `T`, as rows are realized.
impl<C: 'static> MapSlots<C> for ListConfig<C> {
    type With<T: 'static> = ListConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(C) -> T + Clone + 'static) -> ListConfig<T> {
        ListConfig {
            contents: self.contents.map_views(f),
            selection: self.selection,
            tags: self.tags,
            editing: self.editing,
            edits: self.edits,
            scroll_controller: self.scroll_controller,
            sections: self.sections,
            min_row_height: self.min_row_height,
        }
    }
}

/// The minimum height of every row in the lists below it, in points.
///
/// Replaces the theme's one-line row height as the floor a row is measured
/// against. `0.0` sizes each row to its content plus its insets. Set it with
/// [`ViewExt::list_min_row_height`](crate::ViewExt::list_min_row_height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMinRowHeight(pub f32);

/// Whether a list row can be deleted when its enclosing list has `on_delete`.
///
/// Installed on a row's view environment by
/// [`ViewExt::list_row_deletable`](crate::ViewExt::list_row_deletable).
#[derive(Debug, Clone)]
pub struct ListRowDeletable(pub Computed<bool>);

/// Custom insets between a list row's edges and its content.
///
/// Installed on a row's view environment by
/// [`ViewExt::list_row_insets`](crate::ViewExt::list_row_insets).
#[derive(Debug, Clone, PartialEq)]
pub struct ListRowInsets(pub EdgeInsets);

/// Marks the subtree below it as a navigation container's sidebar column.
///
/// The owner of the sidebar — a `NavigationSplitView` backend — installs it
/// into the sidebar's environment; a list reads it to take the platform's
/// sidebar presentation: the unemphasised selection chrome rather than the
/// accent fill, and a selected row keeping its own colors rather than
/// re-theming onto `SelectionForeground`.
#[derive(Debug, Clone, Copy)]
#[doc(hidden)]
pub struct SidebarContent;

impl<C: 'static> NativeView for ListConfig<C> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// A component that displays items in a list format.
///
/// A row whose content is under [`.disabled(true)`](crate::ViewExt::disabled)
/// is inert to selection the way a disabled `Button` cannot be pressed:
/// pointer, keyboard and accessibility input all leave the selection
/// unchanged, and the row's accessibility node reports disabled without
/// offering the click action. The row's content still renders with its label
/// and disabled state intact.
#[derive(Debug)]
pub struct List<V: Views> {
    contents: V,
    sections: RowSections<V::Id>,
    edits: RowEdits<V::Id>,
}

impl<V> List<V>
where
    V: Views,
{
    /// Creates a new list with the specified contents, all in one section
    /// without chrome.
    pub fn new(contents: V) -> Self {
        Self {
            contents,
            sections: RowSections::plain(),
            edits: RowEdits::none(),
        }
    }

    /// The builder carrying this list's contents, sections, and edits, with
    /// every remaining option unset.
    fn builder(self) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            selection: ListSelection::None,
            tags: RowTags::none(),
            editing: Computed::new(false),
            edits: self.edits,
            single_delete: None,
            single_move: None,
            scroll_controller: None,
            sections: self.sections,
        }
    }

    /// Enables edit mode with the given reactive signal.
    ///
    /// When edit mode is enabled, delete buttons and drag handles are shown.
    #[must_use]
    pub fn editing(self, editing: impl IntoComputed<bool>) -> ListBuilder<V> {
        self.builder().editing(editing)
    }

    /// Sets the callback for when an item is deleted, injecting
    /// [`ListDelete<V::Id>`].
    ///
    /// Available on single-collection lists such as [`List::for_each`]; inside
    /// [`List::content`], attach `.on_delete(...)` to each [`ForEach`] or
    /// [`Row`] instead.
    #[must_use]
    pub fn on_delete<H, Args>(self, on_delete: H) -> ListBuilder<V>
    where
        V: EditableRows,
        H: Handler<Args, ()>,
    {
        self.builder().on_delete(on_delete)
    }

    /// Sets the callback for when an item is moved/reordered, injecting
    /// [`ListMove<V::Id>`].
    ///
    /// Available on single-collection lists such as [`List::for_each`]; inside
    /// [`List::content`], attach `.on_move(...)` to each [`ForEach`] instead.
    #[must_use]
    pub fn on_move<H, Args>(self, on_move: H) -> ListBuilder<V>
    where
        V: EditableRows,
        H: Handler<Args, ()>,
    {
        self.builder().on_move(on_move)
    }

    /// Connects a controller that scrolls the list to a requested item index.
    /// [`ScrollController::scroll_to`] jumps; [`ScrollController::animate_to`]
    /// glides along the request's animation. An `animate_to` further than 100
    /// rows away does not fly over every row: the backend jumps to within 100
    /// rows of the target first and animates only that final stretch.
    #[must_use]
    pub fn scroll_controller(self, controller: &ScrollController<usize>) -> ListBuilder<V> {
        self.builder().scroll_controller(controller)
    }

    /// Single selection by the value each row is selected by: a `ForEach`
    /// row's item id, a static row's `.tag(…)` value (see
    /// [`SelectableRows`]). The framework and backends write it on pointer,
    /// keyboard and accessibility input.
    ///
    /// A row without such a value is not selectable, and neither is a row
    /// whose content is under `.disabled(true)`: input routes around it and
    /// the binding stays unchanged.
    #[must_use]
    pub fn selection<T>(self, selection: &Binding<Option<T>>) -> ListBuilder<V>
    where
        V: SelectableRows<T>,
        T: 'static + Ord + Clone,
    {
        self.builder().selection(selection)
    }

    /// Multiple selection by the value each row is selected by, as
    /// [`List::selection`].
    ///
    /// A row without such a value is not selectable, and neither is a row
    /// whose content is under `.disabled(true)`: input routes around it and
    /// the binding stays unchanged.
    #[must_use]
    pub fn multi_selection<T>(self, selection: &Binding<BTreeSet<T>>) -> ListBuilder<V>
    where
        V: SelectableRows<T>,
        T: 'static + Ord + Clone,
    {
        self.builder().multi_selection(selection)
    }
}

impl<C, F, V> List<ForEach<C, F, V>>
where
    C: Collection + Clone,
    C::Item: Clone,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    /// Creates a lazy list over an identity-keyed reactive collection.
    ///
    /// Renderers request only rows in the visible viewport. A programmatic
    /// scroll through [`ScrollController`] therefore does not materialize the
    /// preceding rows it passes — an animated request realizes only the rows
    /// the moving viewport covers each frame.
    /// Use [`List::content`] instead to group rows into sections.
    pub fn for_each(data: C, generator: F) -> Self {
        Self::new(ForEach::new(data, generator))
    }
}

impl<C: ListContent> List<ContentRows<C, C::Row>> {
    /// Creates a list from a [`ListContent`] tree.
    ///
    /// `ListContent` accepts `|| text("...")` row builders, [`Row`] (the
    /// [`row`] / [`detail_row`] helpers produce one), a [`ForEach`] over a
    /// reactive collection, [`Section<C>`], tuples, arrays, vectors, and
    /// `Option<T>`, so heterogeneous content composes structurally just like
    /// `SwiftUI`'s `Section { row; ForEach(items) { … } }` form. A static row
    /// is a builder rather than a finished view because the list rebuilds it
    /// whenever the row is realized again; a `ForEach` follows its collection
    /// row by row. For any pre-built [`Views`] implementation, use [`List::new`].
    ///
    /// The list is selected the way `SwiftUI`'s `List(selection:)` is: a
    /// `ForEach` row by its item's id, a static row by the value it is
    /// tagged with — `(|| text("...")).tag(value)` or [`Row::tag`] — and an
    /// untagged static row not at all.
    ///
    /// ```rust
    /// use waterui::component::list::{List, Section, row};
    /// use waterui::prelude::*;
    ///
    /// #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    /// enum Mailbox {
    ///     Inbox,
    ///     Archive,
    /// }
    ///
    /// let selection = binding(Some(Mailbox::Inbox));
    /// let list = List::content(Section::new("Mailboxes").content((
    ///     (|| text("Inbox")).tag(Mailbox::Inbox),
    ///     row("Archive", "12").tag(Mailbox::Archive),
    ///     row("Storage", "3 GB"),
    /// )))
    /// .selection(&selection);
    /// ```
    #[must_use]
    pub fn content(content: C) -> Self {
        let mut sink = ListItemSink::new();
        content.collect_items(&mut sink, |row| row);
        let (contents, sections, edits) = sink.finish();
        Self {
            contents,
            sections,
            edits,
        }
    }
}

impl<V> List<V>
where
    V: Views + 'static,
{
    /// Returns the configuration for this list.
    #[must_use]
    pub fn config(self) -> ListConfig<V::View> {
        self.builder().config()
    }
}

impl<C: View> From<ListConfig<C>> for List<SharedAnyViews<C>> {
    fn from(value: ListConfig<C>) -> Self {
        Self {
            contents: value.contents,
            sections: RowSections::from_erased(value.sections),
            edits: RowEdits::from_erased(value.edits),
        }
    }
}

/// `config` as it reaches a backend: every row themed for the selection, the
/// sections resolved and the row metrics read from `env`.
fn resolve_list_config<C: View>(
    config: ListConfig<C>,
    env: &Environment,
) -> ListConfig<SelectionThemed<C>> {
    // Erasing the themed contents again re-keys every row id, so the row
    // tags, sections and edits travel through the same generator — a backend's
    // `get_id` must return exactly the ids all three are keyed by. The
    // selection holds tags, which the re-keying leaves alone.
    let selection = config.selection.clone();
    let tags = config.tags.clone();
    let (contents, ids) = AnyViews::new_with_ids(WithId::new(config.contents, move |id, item| {
        selection_themed(item, &selection, tags.tag(id))
    }));
    ListConfig {
        contents: SharedAnyViews::from(contents),
        tags: config.tags.rekeyed(&ids),
        // Section headers and footers are semantic text: they localize, and
        // they may be driven by a signal. Resolving them here is the one
        // place the environment is in hand.
        sections: config.sections.rekeyed(&ids).resolved(env),
        edits: config.edits.rekeyed(&ids),
        min_row_height: env.get::<ListMinRowHeight>().map(|height| height.0),
        selection: config.selection,
        editing: config.editing,
        scroll_controller: config.scroll_controller,
    }
}

/// The erased form of a list, for code that expands views by hand.
fn erased_list<C: View>(config: ListConfig<C>, env: &Environment) -> Native<ListConfig<AnyView>> {
    Native::new(resolve_list_config(config, env).map_slots(AnyView::new))
}

/// Realizes a list through `backend`, each row in the backend's child slot.
fn dispatch_list<B: Backend, C: View>(
    config: ListConfig<C>,
    backend: &mut B,
    env: &Environment,
    cx: B::Ctx,
) -> B::Output {
    let config = resolve_list_config(config, env).map_slots(B::Slot::from_view);
    backend.native(config, env, cx)
}

/// Re-themes a row's content while it is selected.
///
/// A selected row is filled with the theme's `SelectionContainer`, so everything
/// the row draws on top of it flips to `SelectionForeground` — the way the
/// platforms' own lists flip a selected row's labels. The pair is a slot each
/// backend owns, because the selection fill is not the accent color everywhere:
/// Material tints the row with a tonal container and writes its own on-container
/// color over it. Anything the row resolves through the theme's foreground,
/// muted-foreground, or accent slots follows the `selected` signal reactively;
/// the row itself is not rebuilt. A row without a tag is never selected and
/// is left as it is.
fn selection_themed<C>(
    content: C,
    selection: &ListSelection<ListTagId>,
    tag: Option<ListTagId>,
) -> SelectionThemed<C> {
    use nami::SignalExt;

    // A row's selected state derives from the list selection and the row's
    // tag: the row is selected exactly when its tag is in the binding. A row
    // without a tag is never selected.
    let selected = tag.and_then(|tag| -> Option<Computed<bool>> {
        match selection {
            ListSelection::None => None,
            ListSelection::Single(selection) => Some(
                selection
                    .clone()
                    .map(move |current| current == Some(tag))
                    .computed(),
            ),
            ListSelection::Multiple(selection) => Some(
                selection
                    .clone()
                    .map(move |current| current.contains(&tag))
                    .computed(),
            ),
        }
    });
    SelectionThemed::for_list(content, selected)
}

impl<V> View for List<V>
where
    V: Views + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        erased_list(self.config(), env)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        dispatch_list(self.config(), backend, env, cx)
    }
}

// ============================================================================
// ListBuilder - Fluent API for configuring lists
// ============================================================================

/// Builder for configuring a list with editing, delete, and move capabilities.
pub struct ListBuilder<V: Views> {
    contents: V,
    /// The selection, already erased into tags through the value mapping
    /// `tags` erases the rows' values with.
    selection: ListSelection<ListTagId>,
    tags: RowTags<V::Id>,
    editing: Computed<bool>,
    edits: RowEdits<V::Id>,
    single_delete: Option<TypedOnDelete<V::Id>>,
    single_move: Option<TypedOnMove<V::Id>>,
    scroll_controller: Option<ScrollController<usize>>,
    sections: RowSections<V::Id>,
}

impl<V: Views> core::fmt::Debug for ListBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListBuilder")
    }
}

impl<V> ListBuilder<V>
where
    V: Views,
{
    /// Enables edit mode with the given reactive signal.
    #[must_use]
    pub fn editing(mut self, editing: impl IntoComputed<bool>) -> Self {
        self.editing = editing.into_computed();
        self
    }

    /// Sets the callback for when an item is deleted, injecting
    /// [`ListDelete<V::Id>`].
    #[must_use]
    pub fn on_delete<H, Args>(mut self, on_delete: H) -> Self
    where
        V: EditableRows,
        H: Handler<Args, ()>,
    {
        self.single_delete = Some(list_delete_action(on_delete));
        self.edits = RowEdits::single_part(self.single_delete.clone(), self.single_move.clone());
        self
    }

    /// Sets the callback for when items are moved/reordered, injecting
    /// [`ListMove<V::Id>`].
    #[must_use]
    pub fn on_move<H, Args>(mut self, on_move: H) -> Self
    where
        V: EditableRows,
        H: Handler<Args, ()>,
    {
        self.single_move = Some(list_move_action(on_move));
        self.edits = RowEdits::single_part(self.single_delete.clone(), self.single_move.clone());
        self
    }

    /// Connects a controller that scrolls the list to a requested item index.
    /// [`ScrollController::scroll_to`] jumps; [`ScrollController::animate_to`]
    /// glides along the request's animation. An `animate_to` further than 100
    /// rows away does not fly over every row: the backend jumps to within 100
    /// rows of the target first and animates only that final stretch.
    #[must_use]
    pub fn scroll_controller(mut self, controller: &ScrollController<usize>) -> Self {
        self.scroll_controller = Some(controller.clone());
        self
    }

    /// Single selection by the value each row is selected by, as
    /// [`List::selection`].
    #[must_use]
    pub fn selection<T>(mut self, selection: &Binding<Option<T>>) -> Self
    where
        V: SelectableRows<T>,
        T: 'static + Ord + Clone,
    {
        let values = Mapping::new();
        self.tags = RowTags::new(self.contents.row_tags(), &values);
        self.selection = ListSelection::single(selection, &values);
        self
    }

    /// Multiple selection by the value each row is selected by, as
    /// [`List::multi_selection`].
    #[must_use]
    pub fn multi_selection<T>(mut self, selection: &Binding<BTreeSet<T>>) -> Self
    where
        V: SelectableRows<T>,
        T: 'static + Ord + Clone,
    {
        let values = Mapping::new();
        self.tags = RowTags::new(self.contents.row_tags(), &values);
        self.selection = ListSelection::multiple(selection, &values);
        self
    }
}

impl<V> ListBuilder<V>
where
    V: Views + 'static,
{
    /// Returns the configuration for this list builder.
    #[must_use]
    pub fn config(self) -> ListConfig<V::View> {
        let (contents, ids) = AnyViews::new_with_ids(self.contents);
        ListConfig {
            contents: SharedAnyViews::from(contents),
            selection: self.selection,
            tags: self.tags.erased(&ids),
            editing: self.editing,
            edits: self.edits.erased(&ids),
            scroll_controller: self.scroll_controller,
            sections: self.sections.erased(&ids),
            min_row_height: None,
        }
    }
}

impl<V> View for ListBuilder<V>
where
    V: Views + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        erased_list(self.config(), env)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        dispatch_list(self.config(), backend, env, cx)
    }
}

// ============================================================================
// ListSection - Section chrome descriptor
// ============================================================================

/// The chrome of one section of a list: its header and footer.
///
/// A [`Section`] in [`List::content`] produces one; a backend reads it from
/// [`ListSections`] and draws it around the rows the section currently
/// holds — the header above the first, the footer below the last.
///
/// The visual is left to the backend: iOS renders this as a `UITableView`
/// section header (and inset-grouped chrome around the section), macOS uses
/// `NSTableView` group rows, and Material backends translate it into
/// Material section dividers. View code only declares the semantic intent.
///
/// The header and footer are [`Text`], not strings: a section title localizes
/// and may be driven by a signal ("3 unread"), and the backend follows that
/// signal in place rather than rebuilding the section. The chrome's
/// typography belongs to the platform, so a header takes the list's own
/// section style regardless of any styling applied to the text.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ListSection {
    /// Title shown above the section.
    label: Option<Text>,
    /// Footer text shown below the section.
    footer: Option<Text>,
}

impl ListSection {
    /// Creates a section descriptor from resolved/semantic optional header and
    /// footer text. App code constructs sections through [`Section`].
    #[must_use]
    pub(crate) const fn new(label: Option<Text>, footer: Option<Text>) -> Self {
        Self { label, footer }
    }

    /// Title shown above the section.
    #[must_use]
    pub const fn label(&self) -> Option<&Text> {
        self.label.as_ref()
    }

    /// Footer text shown below the section.
    #[must_use]
    pub const fn footer(&self) -> Option<&Text> {
        self.footer.as_ref()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Option<Text>, Option<Text>) {
        (self.label, self.footer)
    }

    /// Resolves the semantic header and footer against `env`, turning
    /// localized or environment-dependent text into raw reactive configs the
    /// renderer can read without an environment.
    ///
    /// The resolved text stays a signal, so a section header built from a
    /// `Computed` — or one that only changes when the locale does — keeps
    /// updating in place instead of being frozen at construction.
    #[must_use]
    pub(crate) fn resolved(self, env: &Environment) -> Self {
        Self {
            label: self.label.map(|label| Text::from(label.resolve(env))),
            footer: self.footer.map(|footer| Text::from(footer.resolve(env))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewExt;
    use nami::Signal as _;
    use waterui_controls::label::TitleLabel;
    use waterui_core::OneOf2;
    use waterui_core::backend::peel_metadata;
    use waterui_macros::text;

    /// Resolves `config` the way `List` does before it reaches a backend,
    /// so the resolved metrics can be asserted on.
    fn render_config<C: View>(config: ListConfig<C>, env: &Environment) -> ListConfig<impl View> {
        resolve_list_config(config, env)
    }

    /// Unpacks the `Metadata<Environment>` an environment modifier emits —
    /// the same step a renderer runs before realizing the wrapped view.
    fn modifier_parts(view: impl View) -> (AnyView, Environment) {
        let metadata = peel_metadata::<Environment>(view, &Environment::new())
            .expect("the modifier emits environment metadata");
        (metadata.content, metadata.value)
    }

    /// `water-rs/waterui#1249`: `.list_min_row_height` carries the row floor
    /// through the environment into the rendered `ListConfig` — `0.0`
    /// included — while a list without the modifier keeps `None`.
    #[test]
    fn min_row_height_resolves_from_the_environment() {
        let (content, env) = modifier_parts(
            List::content((row("Alice", "joined"), row("Hello", ""))).list_min_row_height(0.0),
        );
        let list = *content
            .downcast::<List<
                ContentRows<
                    (Row<TitleLabel>, Row<TitleLabel>),
                    OneOf2<RowContent<TitleLabel>, RowContent<TitleLabel>>,
                >,
            >>()
            .unwrap_or_else(|_| panic!("the metadata did not wrap the list"));
        let with_floor = render_config(list.config(), &env);
        assert_eq!(with_floor.min_row_height, Some(0.0));

        let without = render_config(
            List::content((|| text!("Alice joined"), || text!("Hello"))).config(),
            &Environment::new(),
        );
        assert_eq!(without.min_row_height, None);
    }

    /// `water-rs/waterui#1249` / `#4300`: a row's `.list_row_insets` and
    /// `.list_row_deletable` modifiers survive the section and selection
    /// transforms `resolve_list_config` wraps every row in.
    #[test]
    fn row_modifiers_survive_the_section_and_selection_transform() {
        use crate::color::WorkingColor;
        use crate::theme::color::{Accent, Foreground, MutedForeground, SelectionForeground};
        use crate::theme::install_color_signal;

        let mut env = Environment::new();
        install_color_signal::<Foreground>(
            &mut env,
            Computed::constant(WorkingColor::new([0.0, 0.0, 0.0, 1.0])),
        );
        install_color_signal::<MutedForeground>(
            &mut env,
            Computed::constant(WorkingColor::new([0.4, 0.4, 0.4, 1.0])),
        );
        install_color_signal::<Accent>(
            &mut env,
            Computed::constant(WorkingColor::new([0.0, 0.5, 1.0, 1.0])),
        );
        install_color_signal::<SelectionForeground>(
            &mut env,
            Computed::constant(WorkingColor::new([1.0, 1.0, 1.0, 1.0])),
        );

        let selection = nami::Binding::container(Option::<i32>::None);
        let config = render_config(
            List::content((
                (|| {
                    text!("Alice joined")
                        .list_row_insets(EdgeInsets::all(4.0))
                        .list_row_deletable(false)
                })
                .tag(1),
                (|| text!("Hello")).tag(2),
            ))
            .selection(&selection)
            .config(),
            &env,
        );
        let contents = config.contents.snapshot();
        // Peel the leading Metadata<Environment> chain (SelectionThemed ->
        // ListRowDeletable -> ListRowInsets) the way backends resolve row env.
        let mut current = AnyView::new(contents.get_view(0).expect("row 0 exists"));
        let mut resolved = env.clone();
        while let Some(meta) = peel_metadata::<Environment>(current, &resolved) {
            resolved = meta.value;
            current = meta.content;
        }
        assert!(
            !resolved
                .get::<ListRowDeletable>()
                .expect("resolved row env carries ListRowDeletable")
                .0
                .snapshot()
        );
        assert_eq!(
            resolved.get::<ListRowInsets>().map(|entry| entry.0.clone()),
            Some(EdgeInsets::all(4.0))
        );

        let mut second = AnyView::new(contents.get_view(1).expect("row 1 exists"));
        let mut second_env = env;
        while let Some(meta) = peel_metadata::<Environment>(second, &second_env) {
            second_env = meta.value;
            second = meta.content;
        }
        assert!(second_env.get::<ListRowDeletable>().is_none());
        assert!(second_env.get::<ListRowInsets>().is_none());
    }

    #[test]
    fn sectioned_mixed_list_routes_edits_by_identity_and_bounds_moves_to_part() {
        use alloc::rc::Rc;
        use alloc::vec;
        use alloc::vec::Vec;
        use core::cell::{Cell, RefCell};
        use nami::collection::StateList;
        use waterui_core::id::Identifiable;

        use crate::component::text;

        #[derive(Clone)]
        struct Entry {
            id: u64,
            title: &'static str,
        }

        impl Identifiable for Entry {
            type Id = u64;
            fn id(&self) -> Self::Id {
                self.id
            }
        }

        let static_deleted = Rc::new(Cell::new(false));
        let deleted_ids = Rc::new(RefCell::new(Vec::new()));
        let moved_events = Rc::new(RefCell::new(Vec::new()));

        let first = StateList::from(vec![
            Entry { id: 10, title: "A" },
            Entry { id: 20, title: "B" },
            Entry { id: 30, title: "C" },
        ]);
        let second = StateList::from(vec![
            Entry {
                id: 100,
                title: "X",
            },
            Entry {
                id: 200,
                title: "Y",
            },
        ]);

        let list = List::content((
            Section::new("Account").content((
                row("Plan", "Pro"),
                row("Cache", "120 MB").on_delete({
                    let static_deleted = Rc::clone(&static_deleted);
                    move || static_deleted.set(true)
                }),
            )),
            Section::new("Items").content(
                ForEach::new(first, |entry: Entry| text(entry.title))
                    .on_delete({
                        let deleted_ids = Rc::clone(&deleted_ids);
                        move |ListDelete(id): ListDelete<u64>| deleted_ids.borrow_mut().push(id)
                    })
                    .on_move({
                        let moved_events = Rc::clone(&moved_events);
                        move |ListMove(movement): ListMove<u64>| {
                            moved_events.borrow_mut().push(movement);
                        }
                    }),
            ),
            Section::new("Archive").content(
                ForEach::new(second, |entry: Entry| text(entry.title))
                    .on_move(|_: ListMove<u64>| {}),
            ),
        ));

        let env = Environment::new();
        let config = render_config(list.config(), &env);
        let snapshot = config.contents().snapshot();
        assert_eq!(snapshot.len(), 7);

        let id = |i: usize| snapshot.get_id(i).expect("row id exists");
        let edits = config.edits();

        // Plain static row is neither deletable nor movable.
        assert!(!edits.can_delete(id(0)));
        assert!(!edits.can_move(id(0)));
        assert_eq!(edits.movable_span(&snapshot, 0), None);

        // Opt-in static row is deletable, not movable, and fires its own handler.
        assert!(edits.can_delete(id(1)));
        assert!(!edits.can_move(id(1)));
        edits.delete(&env, id(1));
        assert!(static_deleted.get());

        // First ForEach spans flat indices 2..5 and carries Entry's u64 ids.
        for flat in 2..5 {
            assert!(edits.can_delete(id(flat)));
            assert!(edits.can_move(id(flat)));
            assert_eq!(edits.movable_span(&snapshot, flat), Some(2..5));
        }
        edits.delete(&env, id(3));
        assert_eq!(*deleted_ids.borrow(), vec![20]);

        edits.move_row(&env, id(2), id(4), 0, 2);
        assert_eq!(*moved_events.borrow(), vec![Move::new(10, 30, 0, 2)]);

        // Second ForEach spans flat indices 5..7, is movable-only, and cannot mix with part 1.
        for flat in 5..7 {
            assert!(!edits.can_delete(id(flat)));
            assert!(edits.can_move(id(flat)));
            assert_eq!(edits.movable_span(&snapshot, flat), Some(5..7));
        }
        assert_ne!(edits.movable_part(id(2)), edits.movable_part(id(5)));
    }
}
