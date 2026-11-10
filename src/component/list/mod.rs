//! List component implementation for `WaterUI`.
//!
//! This module provides the necessary components to build and configure lists
//! in the `WaterUI` framework. It includes the `List` component for displaying
//! collections of data, [`ListItem`] for configuring individual items, and
//! the [`ListContent`] / [`Section`] / [`row`] surface for composing
//! heterogeneous lists with sections, static or reactive.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use core::marker::PhantomData;
use core::ops::{Range, RangeBounds};
use nami::collection::{Collection, CollectionChange};
use nami::watcher::Context;
use nami::{Binding, Computed, signal::IntoComputed};

use crate::views::{AnyViews, ForEach, SharedAnyViews, ViewSnapshot, Views, ViewsExt};
use waterui_core::id::Mapping;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{
    AnyView, Environment, Metadata, Native, NativeView, View,
    handler::{Handler, shared_action},
    impl_extractor,
    layout::StretchAxis,
};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::scroll::ScrollController;
use waterui_text::{IntoText, Text};

mod content;
mod rows;
mod section;
mod selection;

pub use content::{
    ContentTags, HoldsTag, ListContent, ListScope, Row, RowLayout, SectionScope, TagPresence,
    Tagged, TaggedBy, Untagged, detail_row, row,
};
use rows::RowSections;
pub use rows::{
    ContentRows, ContentRowsGuard, ContentRowsSnapshot, ListItemSink, ListSectionSpan, ListSections,
};
pub use section::Section;
use selection::RowTags;
pub use selection::{ListRowTags, ListSelection, ListTagId, SelectableRows};

/// A list reorder operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Move {
    from: usize,
    to: usize,
}

/// Per-row delete payload injected into list delete handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListDelete(pub usize);

/// Per-row move payload injected into list move handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListMove(pub Move);

impl_extractor!(ListDelete);
impl_extractor!(ListMove);

impl Move {
    /// Creates a new move operation.
    #[must_use]
    pub const fn new(from: usize, to: usize) -> Self {
        Self { from, to }
    }

    /// Returns the source index.
    #[must_use]
    pub const fn from(self) -> usize {
        self.from
    }

    /// Returns the destination index.
    #[must_use]
    pub const fn to(self) -> usize {
        self.to
    }
}

/// Callback type for delete operations (receives environment and index).
pub type OnDelete = Box<dyn Fn(&Environment, usize)>;

/// Callback type for move/reorder operations (receives environment and movement).
pub type OnMove = Box<dyn Fn(&Environment, Move)>;

/// Configuration for a list component.
#[non_exhaustive]
pub struct ListConfig {
    /// Content items to be displayed in the list.
    contents: SharedAnyViews<ListItem>,
    /// The list's selection bindings, keyed by the erased tags `tags`
    /// answers for the rows. `ListSelection::None` when the list is not
    /// selectable.
    selection: ListSelection<ListTagId>,
    /// The tag each row is selected by, keyed by the same row ids
    /// `contents.get_id` returns.
    tags: ListRowTags,
    /// Read-only signal for edit mode state.
    editing: Computed<bool>,
    /// Optional callback when any item is deleted.
    on_delete: Option<OnDelete>,
    /// Optional callback when items are moved/reordered.
    on_move: Option<OnMove>,
    /// Optional programmatic item-index scroll controller.
    scroll_controller: Option<ScrollController<usize>>,
    /// The sections `contents` groups into, keyed by the same row ids.
    sections: ListSections,
    /// The minimum height of a row, from [`ListMinRowHeight`] in the list's
    /// environment. `None` uses the theme's one-line row height.
    min_row_height: Option<f32>,
}

impl ListConfig {
    /// Content items to be displayed in the list.
    #[must_use]
    pub const fn contents(&self) -> &SharedAnyViews<ListItem> {
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

    /// Optional callback when any item is deleted.
    #[must_use]
    pub const fn on_delete(&self) -> Option<&OnDelete> {
        self.on_delete.as_ref()
    }

    /// Takes the `on_delete` out of the value, leaving none behind.
    pub const fn take_on_delete(&mut self) -> Option<OnDelete> {
        self.on_delete.take()
    }

    /// Optional callback when items are moved/reordered.
    #[must_use]
    pub const fn on_move(&self) -> Option<&OnMove> {
        self.on_move.as_ref()
    }

    /// Takes the `on_move` out of the value, leaving none behind.
    pub const fn take_on_move(&mut self) -> Option<OnMove> {
        self.on_move.take()
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

impl_debug!(ListConfig);

/// The minimum height of every row in the lists below it, in points.
///
/// Replaces the theme's one-line row height as the floor a row is measured
/// against. `0.0` sizes each row to its content plus its insets. Set it with
/// [`ViewExt::list_min_row_height`](crate::ViewExt::list_min_row_height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMinRowHeight(pub f32);

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

impl NativeView for ListConfig {
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
pub struct List<V: Views<View = ListItem> = AnyViews<ListItem>> {
    contents: V,
    sections: RowSections<V::Id>,
}

impl<V> List<V>
where
    V: Views<View = ListItem>,
{
    /// Creates a new list with the specified contents, all in one section
    /// without chrome.
    pub fn new(contents: V) -> Self {
        Self {
            contents,
            sections: RowSections::plain(),
        }
    }

    /// The builder carrying this list's contents and sections, with every
    /// option unset.
    fn builder(self) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            selection: ListSelection::None,
            tags: RowTags::none(),
            editing: Computed::new(false),
            on_delete: None,
            on_move: None,
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

    /// Sets the callback for when any item is deleted.
    #[must_use]
    pub fn on_delete<H, Args>(self, on_delete: H) -> ListBuilder<V>
    where
        H: Handler<Args, ()>,
    {
        self.builder().on_delete(on_delete)
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move<H, Args>(self, on_move: H) -> ListBuilder<V>
    where
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

impl<C, F> List<ForEach<C, F, ListItem>>
where
    C: Collection + Clone,
    C::Item: Clone,
    F: 'static + Fn(C::Item) -> ListItem,
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

impl<C: ListContent> List<ContentRows<C>> {
    /// Creates a list from a [`ListContent`] tree.
    ///
    /// `ListContent` accepts `|| ListItem::new(…)` row builders, [`Row`] (the
    /// [`row`] / [`detail_row`] helpers produce one), a [`ForEach`] over a
    /// reactive collection, [`Section<C>`], tuples, arrays, vectors, and
    /// `Option<T>`, so heterogeneous content composes structurally just like
    /// `SwiftUI`'s `Section { row; ForEach(items) { … } }` form. A static row
    /// is a builder rather than a finished [`ListItem`] because the list
    /// rebuilds it whenever the row is realized again; a `ForEach` follows
    /// its collection row by row. For any pre-built [`Views`]
    /// implementation, use [`List::new`].
    ///
    /// The list is selected the way `SwiftUI`'s `List(selection:)` is: a
    /// `ForEach` row by its item's id, a static row by the value it is
    /// tagged with — `(|| ListItem::new(…)).tag(value)` or
    /// [`Row::tag`] — and an untagged static row not at all.
    ///
    /// ```rust
    /// use waterui::component::list::{List, ListItem, Section, row};
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
    ///     (|| ListItem::new(text("Inbox"))).tag(Mailbox::Inbox),
    ///     row("Archive", "12").tag(Mailbox::Archive),
    ///     row("Storage", "3 GB"),
    /// )))
    /// .selection(&selection);
    /// ```
    #[must_use]
    pub fn content(content: C) -> Self {
        let mut sink = ListItemSink::new();
        content.collect_items(&mut sink);
        let (contents, sections) = sink.finish();
        Self { contents, sections }
    }
}

impl<V> ConfigurableView for List<V>
where
    V: Views<View = ListItem> + 'static,
{
    type Config = ListConfig;

    fn config(self) -> Self::Config {
        self.builder().config()
    }
}

impl ViewConfiguration for ListConfig {
    type View = List<SharedAnyViews<ListItem>>;

    fn render(self) -> Self::View {
        List {
            contents: self.contents,
            sections: RowSections::from_erased(self.sections),
        }
    }
}

impl From<ListConfig> for List<SharedAnyViews<ListItem>> {
    fn from(value: ListConfig) -> Self {
        value.render()
    }
}

fn render_list_config(mut config: ListConfig, env: &Environment) -> impl View {
    // Erasing the themed contents again re-keys every row id, so the row
    // tags and sections travel through the same generator — a backend's
    // `get_id` must return exactly the ids both are keyed by. The selection
    // holds tags, which the re-keying leaves alone.
    let selection = config.selection.clone();
    let tags = config.tags.clone();
    let (contents, ids) = AnyViews::new_with_ids(WithId {
        contents: config.contents.clone(),
        transform: Rc::new(move |id, item| selection_themed(item, &selection, tags.tag(id))),
    });
    config.contents = SharedAnyViews::from(contents);
    config.tags = config.tags.clone().rekeyed(&ids);
    // Section headers and footers are semantic text: they localize, and they
    // may be driven by a signal. Resolving them here is the one place the
    // environment is in hand.
    config.sections = config.sections.clone().rekeyed(&ids).resolved(env);
    config.min_row_height = env.get::<ListMinRowHeight>().map(|height| height.0);
    if let Some(hook) = env.get::<Hook<ListConfig>>() {
        AnyView::new(hook.apply(env, config))
    } else {
        let fallback =
            crate::component::lazy::Lazy::vstack(config.contents.clone().map(|item| item.content));
        AnyView::new(Native::new(config).with_fallback(fallback))
    }
}

/// `Views` adapter that hands each element's id to the mapping closure — the
/// row-keyed sibling of `Map`, so `selection_themed` can derive a row's
/// selected state from the list selection and the row's own id.
struct WithId<C, F> {
    contents: C,
    transform: Rc<F>,
}

/// The immutable snapshot a [`WithId`] captures: the source snapshot plus a
/// shared handle to the transform, so the id and the view a row reports come
/// from the same captured source state.
struct WithIdSnapshot<S, F, V> {
    contents: S,
    transform: Rc<F>,
    _marker: PhantomData<fn() -> V>,
}

impl<S: Clone, F, V> Clone for WithIdSnapshot<S, F, V> {
    fn clone(&self) -> Self {
        Self {
            contents: self.contents.clone(),
            transform: self.transform.clone(),
            _marker: PhantomData,
        }
    }
}

impl<S, F, V> core::fmt::Debug for WithIdSnapshot<S, F, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(core::any::type_name::<Self>())
    }
}

impl<S, F, V> ViewSnapshot for WithIdSnapshot<S, F, V>
where
    S: ViewSnapshot,
    F: Fn(S::Id, S::View) -> V,
    V: View,
{
    type Id = S::Id;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.contents.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.contents.get_id(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        Some((self.transform)(
            self.contents.get_id(index)?,
            self.contents.get_view(index)?,
        ))
    }
}

impl<V, C, F> Views for WithId<C, F>
where
    V: View,
    C: Views,
    F: 'static + Fn(C::Id, C::View) -> V,
{
    type Id = C::Id;
    type Guard = C::Guard;
    type View = V;
    type Snapshot = WithIdSnapshot<C::Snapshot, F, V>;

    fn len(&self) -> Computed<usize> {
        self.contents.len()
    }

    fn snapshot(&self) -> Self::Snapshot {
        WithIdSnapshot {
            contents: self.contents.snapshot(),
            transform: self.transform.clone(),
            _marker: PhantomData,
        }
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        let transform = self.transform.clone();
        self.contents.watch(range, move |ctx, change| {
            let snapshot = ctx.map(|contents| WithIdSnapshot {
                contents,
                transform: transform.clone(),
                _marker: PhantomData,
            });
            watcher(snapshot, change);
        })
    }
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
fn selection_themed(
    mut item: ListItem,
    selection: &ListSelection<ListTagId>,
    tag: Option<ListTagId>,
) -> ListItem {
    use crate::color::WorkingColor;
    use crate::theme::{color, install_color_signal};
    use nami::SignalExt;
    use waterui_core::env::use_env;
    use waterui_core::resolve::Resolvable;

    let Some(tag) = tag else {
        return item;
    };
    // A row's selected state derives from the list selection and the row's
    // tag: the row is selected exactly when its tag is in the binding.
    let selected: Computed<bool> = match selection {
        ListSelection::None => return item,
        ListSelection::Single(selection) => selection
            .clone()
            .map(move |current| current == Some(tag))
            .computed(),
        ListSelection::Multiple(selection) => selection
            .clone()
            .map(move |current| current.contains(&tag))
            .computed(),
    };
    let content = core::mem::take(&mut item.content);
    item.content = AnyView::new(use_env(move |mut env: Environment| {
        // A sidebar's selection chrome is the unemphasised pill, not the
        // accent fill `SelectionForeground` is written for — a selected
        // sidebar row keeps its own colors (water-rs/waterui#1495).
        if env.get::<SidebarContent>().is_some() {
            return AnyView::new(content);
        }
        let on_selection = color::SelectionForeground.resolve(&env).computed();
        let flip = |normal: Computed<WorkingColor>| {
            selected
                .clone()
                .zip(&normal)
                .zip(&on_selection)
                .map(|((selected, normal), selection)| if selected { selection } else { normal })
                .computed()
        };
        let foreground = flip(color::Foreground.resolve(&env).computed());
        let muted = flip(color::MutedForeground.resolve(&env).computed());
        let accent = flip(color::Accent.resolve(&env).computed());
        install_color_signal::<color::Foreground>(&mut env, foreground);
        install_color_signal::<color::MutedForeground>(&mut env, muted);
        install_color_signal::<color::Accent>(&mut env, accent);
        AnyView::new(Metadata::new(content, env))
    }));
    item
}

impl<V> View for List<V>
where
    V: Views<View = ListItem> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        render_list_config(ConfigurableView::config(self), env)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

// ============================================================================
// ListBuilder - Fluent API for configuring lists
// ============================================================================

/// Builder for configuring a list with editing, delete, and move capabilities.
pub struct ListBuilder<V: Views<View = ListItem>> {
    contents: V,
    /// The selection, already erased into tags through the value mapping
    /// `tags` erases the rows' values with.
    selection: ListSelection<ListTagId>,
    tags: RowTags<V::Id>,
    editing: Computed<bool>,
    on_delete: Option<OnDelete>,
    on_move: Option<OnMove>,
    scroll_controller: Option<ScrollController<usize>>,
    sections: RowSections<V::Id>,
}

impl<V: Views<View = ListItem>> core::fmt::Debug for ListBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListBuilder")
    }
}

impl<V> ListBuilder<V>
where
    V: Views<View = ListItem>,
{
    /// Enables edit mode with the given reactive signal.
    #[must_use]
    pub fn editing(mut self, editing: impl IntoComputed<bool>) -> Self {
        self.editing = editing.into_computed();
        self
    }

    /// Sets the callback for when any item is deleted.
    #[must_use]
    pub fn on_delete<H, Args>(mut self, on_delete: H) -> Self
    where
        H: Handler<Args, ()>,
    {
        self.on_delete = Some(list_delete_action(on_delete));
        self
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move<H, Args>(mut self, on_move: H) -> Self
    where
        H: Handler<Args, ()>,
    {
        self.on_move = Some(list_move_action(on_move));
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

impl<V> ConfigurableView for ListBuilder<V>
where
    V: Views<View = ListItem> + 'static,
{
    type Config = ListConfig;

    fn config(self) -> Self::Config {
        let (contents, ids) = AnyViews::new_with_ids(self.contents);
        ListConfig {
            contents: SharedAnyViews::from(contents),
            selection: self.selection,
            tags: self.tags.erased(&ids),
            editing: self.editing,
            on_delete: self.on_delete,
            on_move: self.on_move,
            scroll_controller: self.scroll_controller,
            sections: self.sections.erased(&ids),
            min_row_height: None,
        }
    }
}

impl<V> View for ListBuilder<V>
where
    V: Views<View = ListItem> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        render_list_config(ConfigurableView::config(self), env)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

fn list_delete_action<H, Args>(handler: H) -> OnDelete
where
    H: Handler<Args, ()>,
{
    let action = shared_action(handler);
    Box::new(move |env, index| action.call(&env.extending(ListDelete(index))))
}

fn list_move_action<H, Args>(handler: H) -> OnMove
where
    H: Handler<Args, ()>,
{
    let action = shared_action(handler);
    Box::new(move |env, movement| action.call(&env.extending(ListMove(movement))))
}

// ============================================================================
// ListItem - Individual item in a list
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
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ListSection {
    /// Title shown above the section.
    label: Option<Text>,
    /// Footer text shown below the section.
    footer: Option<Text>,
}

impl ListSection {
    /// Title shown above the section.
    #[must_use]
    pub const fn label(&self) -> Option<&Text> {
        self.label.as_ref()
    }

    /// Footer text shown below the section.
    #[must_use]
    pub const fn get_footer(&self) -> Option<&Text> {
        self.footer.as_ref()
    }

    /// Sets the title shown above the section.
    pub fn set_label(&mut self, label: Option<Text>) {
        self.label = label;
    }

    /// Sets the footer text shown below the section.
    pub fn set_footer(&mut self, footer: Option<Text>) {
        self.footer = footer;
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Option<Text>, Option<Text>) {
        (self.label, self.footer)
    }
}

impl ListSection {
    /// Creates a new section descriptor with just a header label.
    #[must_use]
    pub fn new(label: impl IntoText) -> Self {
        Self {
            label: Some(label.into_text()),
            footer: None,
        }
    }

    /// Creates an unlabeled section break (visual divider only, no header).
    #[must_use]
    pub const fn unlabeled() -> Self {
        Self {
            label: None,
            footer: None,
        }
    }

    /// Adds a footer note shown below the section.
    #[must_use]
    pub fn footer(mut self, footer: impl IntoText) -> Self {
        self.footer = Some(footer.into_text());
        self
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

/// An item in a list that can be configured with various behaviors.
///
/// There is no per-item selectable flag: selection follows the content's
/// own disabled state. Wrap the row's content in
/// [`.disabled(true)`](crate::ViewExt::disabled) to make the row inert to
/// selection — the way a disabled `Button` cannot be pressed — while its
/// label and disabled accessibility state still report.
#[non_exhaustive]
pub struct ListItem {
    /// The view content to display for this item.
    content: AnyView,
    /// Read-only signal indicating whether this item can be deleted.
    deletable: Computed<bool>,
    /// The insets between the row's edges and its content. `None` uses the
    /// theme's row insets.
    insets: Option<EdgeInsets>,
}

impl ListItem {
    /// The view content to display for this item.
    #[must_use]
    pub const fn content(&self) -> &AnyView {
        &self.content
    }

    /// Takes the `content` out of the value.
    pub fn into_content(self) -> AnyView {
        self.content
    }

    /// Takes the `content` out of the item, leaving an empty view behind.
    pub fn take_content(&mut self) -> AnyView {
        core::mem::take(&mut self.content)
    }

    /// Sets the `content`.
    pub fn set_content(&mut self, content: AnyView) {
        self.content = content;
    }

    /// Read-only signal indicating whether this item can be deleted.
    #[must_use]
    pub const fn get_deletable(&self) -> &Computed<bool> {
        &self.deletable
    }

    /// The insets between the row's edges and its content. `None` uses the
    /// theme's row insets.
    #[must_use]
    pub const fn get_insets(&self) -> Option<&EdgeInsets> {
        self.insets.as_ref()
    }

    /// Takes the value apart into its fields, in declaration order.
    pub fn into_parts(self) -> (AnyView, Computed<bool>, Option<EdgeInsets>) {
        (self.content, self.deletable, self.insets)
    }
}

impl NativeView for ListItem {}

impl View for ListItem {
    fn body(self, _env: &Environment) -> impl View {
        self.content
    }
}

impl_debug!(ListItem);

impl ListItem {
    /// Creates a new list item with the given content.
    ///
    /// By default, the item is deletable when the list has `on_delete`.
    pub fn new(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            deletable: Computed::new(true),
            insets: None,
        }
    }

    /// Sets whether this item can be deleted using a reactive signal.
    ///
    /// When false, swipe-to-delete and delete button are disabled for this item.
    #[must_use]
    pub fn deletable(mut self, deletable: impl IntoComputed<bool>) -> Self {
        self.deletable = deletable.into_computed();
        self
    }

    /// Replaces the theme's row insets for this item.
    ///
    /// Together with [`ListMinRowHeight`], this lets one row size to its
    /// content (a one-line event in a chat log, a compact header) while the
    /// rest keep the list's row metrics.
    #[must_use]
    pub const fn insets(mut self, insets: EdgeInsets) -> Self {
        self.insets = Some(insets);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewExt;
    use waterui_macros::text;

    /// Renders `config` the way `List::body` does and returns the `Native`
    /// payload, so the resolved metrics can be asserted on.
    fn render_config(config: ListConfig, env: &Environment) -> ListConfig {
        AnyView::new(render_list_config(config, env))
            .downcast::<Native<ListConfig>>()
            .map_or_else(
                |_| panic!("the list did not render a Native<ListConfig>"),
                |native| native.into_inner(),
            )
    }

    /// Unpacks the `Metadata<Environment>` an environment modifier emits —
    /// the same step a renderer runs before realizing the wrapped view.
    fn modifier_parts(view: impl View) -> (AnyView, Environment) {
        let metadata = AnyView::new(view.body(&Environment::new()))
            .downcast::<Metadata<Environment>>()
            .map_or_else(
                |_| panic!("the modifier did not emit environment metadata"),
                |metadata| *metadata,
            );
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
            .downcast::<List<ContentRows<(Row, Row)>>>()
            .unwrap_or_else(|_| panic!("the metadata did not wrap the list"));
        let with_floor = render_config(ConfigurableView::config(list), &env);
        assert_eq!(with_floor.min_row_height, Some(0.0));

        let without = render_config(
            ConfigurableView::config(List::content((
                || ListItem::new(text!("Alice joined")),
                || ListItem::new(text!("Hello")),
            ))),
            &Environment::new(),
        );
        assert_eq!(without.min_row_height, None);
    }

    /// `water-rs/waterui#1249`: a row's `insets` survives the section and
    /// selection transforms `render_list_config` wraps every item in.
    #[test]
    fn row_insets_survive_the_section_and_selection_transform() {
        let selection = nami::Binding::container(Option::<i32>::None);
        let config = render_config(
            ConfigurableView::config(
                List::content((
                    (|| ListItem::new(text!("Alice joined")).insets(EdgeInsets::all(4.0))).tag(1),
                    (|| ListItem::new(text!("Hello"))).tag(2),
                ))
                .selection(&selection),
            ),
            &Environment::new(),
        );
        let contents = config.contents.snapshot();
        let first = contents.get_view(0).expect("row 0 exists");
        let second = contents.get_view(1).expect("row 1 exists");
        assert_eq!(first.insets, Some(EdgeInsets::all(4.0)));
        assert_eq!(second.insets, None);
    }
}
