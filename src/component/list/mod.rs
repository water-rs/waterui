//! List component implementation for `WaterUI`.
//!
//! This module provides the necessary components to build and configure lists
//! in the `WaterUI` framework. It includes the `List` component for displaying
//! collections of data, [`ListItem`] for configuring individual items, and
//! the [`ListContent`] / [`Section`] / [`row`] surface for composing static
//! heterogeneous lists with sections.

use alloc::boxed::Box;
use core::ops::RangeBounds;
use nami::collection::Collection;
use nami::watcher::Context;
use nami::{Computed, signal::IntoComputed};

use crate::views::{AnyViews, ForEach, SharedAnyViews, Views, ViewsExt};
use waterui_core::id::SelfId;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{
    AnyView, Environment, Native, NativeView, View,
    handler::{AnyViewBuilder, Handler, shared_action},
    id::Identifiable,
    layout::StretchAxis,
};

mod content;
mod section;

pub use content::{ListContent, ListItemSink, Row, RowLayout, detail_row, row};
pub use section::Section;

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
pub struct ListConfig {
    /// Content items to be displayed in the list.
    pub contents: SharedAnyViews<ListItem>,
    /// Read-only signal for edit mode state.
    pub editing: Computed<bool>,
    /// Optional callback when any item is deleted.
    pub on_delete: Option<OnDelete>,
    /// Optional callback when items are moved/reordered.
    pub on_move: Option<OnMove>,
}

impl_debug!(ListConfig);

impl NativeView for ListConfig {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// A component that displays items in a list format.
#[derive(Debug)]
pub struct List<V: Views<View = ListItem> = AnyViews<ListItem>>(V);

impl<V> List<V>
where
    V: Views<View = ListItem>,
{
    /// Creates a new list with the specified contents.
    pub const fn new(contents: V) -> Self {
        Self(contents)
    }

    /// Enables edit mode with the given reactive signal.
    ///
    /// When edit mode is enabled, delete buttons and drag handles are shown.
    #[must_use]
    pub fn editing(self, editing: impl IntoComputed<bool>) -> ListBuilder<V> {
        ListBuilder {
            contents: self.0,
            editing: editing.into_computed(),
            on_delete: None,
            on_move: None,
        }
    }

    /// Sets the callback for when any item is deleted.
    #[must_use]
    pub fn on_delete<H, Args>(self, on_delete: H) -> ListBuilder<V>
    where
        H: Handler<Args, ()>,
    {
        ListBuilder {
            contents: self.0,
            editing: Computed::new(false),
            on_delete: Some(list_delete_action(on_delete)),
            on_move: None,
        }
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move<H, Args>(self, on_move: H) -> ListBuilder<V>
    where
        H: Handler<Args, ()>,
    {
        ListBuilder {
            contents: self.0,
            editing: Computed::new(false),
            on_delete: None,
            on_move: Some(list_move_action(on_move)),
        }
    }
}

impl<C, F> List<ForEach<C, F, ListItem>>
where
    C: Collection + Clone,
    C::Item: Identifiable,
    F: 'static + Fn(C::Item) -> ListItem,
{
    /// Creates a new list by iterating over a collection and generating items.
    pub const fn for_each(data: C, generator: F) -> Self {
        Self(ForEach::new(data, generator))
    }
}

impl List<BuiltViews> {
    /// Creates a list from a static [`ListContent`] tree.
    ///
    /// `ListContent` accepts [`ListItem`], [`Row`] (the [`row`] / [`detail_row`]
    /// helpers produce one), [`Section<C>`], tuples, arrays, vectors, and
    /// `Option<T>`, so heterogeneous content composes structurally just like
    /// `SwiftUI`'s `Section { row; row }` form. For dynamic identity-keyed data,
    /// use [`List::for_each`] instead; for any pre-built [`Views`]
    /// implementation, use [`List::new`].
    #[must_use]
    pub fn content(content: impl ListContent) -> Self {
        let mut sink = ListItemSink::new();
        content.collect_items(&mut sink);
        Self(BuiltViews::new(sink))
    }
}

/// `Views` adapter that materializes the entries collected by a
/// [`ListContent`] tree on demand.
///
/// Each entry stores a clonable builder that produces a fresh [`ListItem`]
/// every time `Views::get_view` is called, plus an optional [`ListSection`]
/// marker attached by [`Section`].
pub struct BuiltViews {
    entries: alloc::vec::Vec<(AnyViewBuilder<ListItem>, Option<ListSection>)>,
}

impl BuiltViews {
    pub(crate) fn new(sink: ListItemSink) -> Self {
        Self {
            entries: sink.into_entries(),
        }
    }
}

impl core::fmt::Debug for BuiltViews {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuiltViews")
            .field("len", &self.entries.len())
            .finish()
    }
}

impl Views for BuiltViews {
    type Id = SelfId<usize>;
    type Guard = ();
    type View = ListItem;

    fn len(&self) -> Computed<usize> {
        Computed::constant(self.entries.len())
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        (index < self.entries.len()).then(|| SelfId::new(index))
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        let (builder, section) = self.entries.get(index)?;
        let mut item = builder.build();
        item.section = section;
        Some(item)
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static,
    ) -> Self::Guard {
    }
}

impl<V> ConfigurableView for List<V>
where
    V: Views<View = ListItem> + 'static,
{
    type Config = ListConfig;

    fn config(self) -> Self::Config {
        ListConfig {
            contents: SharedAnyViews::new(self.0),
            editing: Computed::new(false),
            on_delete: None,
            on_move: None,
        }
    }
}

impl ViewConfiguration for ListConfig {
    type View = List<SharedAnyViews<ListItem>>;

    fn render(self) -> Self::View {
        List::new(self.contents)
    }
}

impl From<ListConfig> for List<SharedAnyViews<ListItem>> {
    fn from(value: ListConfig) -> Self {
        value.render()
    }
}

fn render_list_config(config: ListConfig, env: &Environment) -> impl View {
    if let Some(hook) = env.get::<Hook<ListConfig>>() {
        AnyView::new(hook.apply(env, config))
    } else {
        let fallback =
            crate::component::lazy::Lazy::vstack(config.contents.clone().map(|item| item.content));
        AnyView::new(Native::new(config).with_fallback(fallback))
    }
}

impl<V> View for List<V>
where
    V: Views<View = ListItem> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        render_list_config(ConfigurableView::config(self), env)
    }
}

// ============================================================================
// ListBuilder - Fluent API for configuring lists
// ============================================================================

/// Builder for configuring a list with editing, delete, and move capabilities.
pub struct ListBuilder<V: Views<View = ListItem>> {
    contents: V,
    editing: Computed<bool>,
    on_delete: Option<OnDelete>,
    on_move: Option<OnMove>,
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
}

impl<V> ConfigurableView for ListBuilder<V>
where
    V: Views<View = ListItem> + 'static,
{
    type Config = ListConfig;

    fn config(self) -> Self::Config {
        ListConfig {
            contents: SharedAnyViews::new(self.contents),
            editing: self.editing,
            on_delete: self.on_delete,
            on_move: self.on_move,
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

/// Semantic section break carried by a [`ListItem`].
///
/// When an item carries a `ListSection`, the renderer treats that item as the
/// start of a new logical group within the same list. Subsequent items
/// without their own `ListSection` belong to the most recently opened group.
///
/// The visual is left to the backend: iOS renders this as a `UITableView`
/// section header (and inset-grouped chrome around the section), macOS uses
/// `NSTableView` group rows, and Material backends translate it into
/// Material section dividers. View code only declares the semantic intent.
#[derive(Debug, Clone, Default)]
pub struct ListSection {
    /// Title shown above the section.
    pub label: Option<waterui_core::Str>,
    /// Footer text shown below the section.
    pub footer: Option<waterui_core::Str>,
}

impl ListSection {
    /// Creates a new section descriptor with just a header label.
    #[must_use]
    pub fn new(label: impl Into<waterui_core::Str>) -> Self {
        Self {
            label: Some(label.into()),
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
    pub fn footer(mut self, footer: impl Into<waterui_core::Str>) -> Self {
        self.footer = Some(footer.into());
        self
    }
}

/// An item in a list that can be configured with various behaviors.
pub struct ListItem {
    /// The view content to display for this item.
    pub content: AnyView,
    /// Read-only signal indicating whether this item can be deleted.
    pub deletable: Computed<bool>,
    /// When `Some`, this item starts a new logical section. The backend uses
    /// this marker to group subsequent items into native chrome (iOS inset
    /// grouped sections, macOS group rows, Material section headers).
    pub section: Option<ListSection>,
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
            section: None,
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

    /// Marks this item as the first row of a new section with the given header.
    ///
    /// All later items without their own [`ListItem::section`] marker render
    /// inside the same section until another marker is encountered.
    #[must_use]
    pub fn section(mut self, section: ListSection) -> Self {
        self.section = Some(section);
        self
    }
}
