//! List component implementation for `WaterUI`.
//!
//! This module provides the necessary components to build and configure lists
//! in the `WaterUI` framework. It includes the `List` component for displaying
//! collections of data, and `ListItem` for configuring individual items in the
//! list.

use alloc::boxed::Box;
use nami::collection::Collection;
use nami::{Computed, Signal};

use crate::views::{AnyViews, ForEach, SharedAnyViews, Views, ViewsExt};
use nami::SignalExt;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{
    id::Identifiable, layout::StretchAxis, AnyView, Environment, Native, NativeView, View,
};

/// A list reorder operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Move {
    from: usize,
    to: usize,
}

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

    /// Starts building a list with captured state.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ListStatefulBuilder<V, T> {
        ListStatefulBuilder {
            contents: self.0,
            editing: Computed::new(false),
            on_delete: None,
            on_move: None,
            state: state.clone(),
        }
    }

    /// Enables edit mode with the given reactive signal.
    ///
    /// When edit mode is enabled, delete buttons and drag handles are shown.
    #[must_use]
    pub fn editing(self, editing: impl Signal<Output = bool> + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.0,
            editing: editing.computed(),
            on_delete: None,
            on_move: None,
        }
    }

    /// Sets the callback for when any item is deleted.
    #[must_use]
    pub fn on_delete(self, on_delete: impl Fn(&Environment, usize) + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.0,
            editing: Computed::new(false),
            on_delete: Some(Box::new(on_delete)),
            on_move: None,
        }
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move(self, on_move: impl Fn(&Environment, Move) + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.0,
            editing: Computed::new(false),
            on_delete: None,
            on_move: Some(Box::new(on_move)),
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

fn render_list_config(config: ListConfig, env: &Environment) -> AnyView {
    if let Some(hook) = env.get::<Hook<ListConfig>>() {
        return AnyView::new(hook.apply(env, config));
    }

    let fallback =
        crate::component::lazy::Lazy::vstack(config.contents.clone().map(|item| item.content));
    AnyView::new(Native::new(config).with_fallback(fallback))
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
    /// Adds state to capture for subsequent event handlers.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ListStatefulBuilder<V, T> {
        ListStatefulBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: self.on_delete,
            on_move: self.on_move,
            state: state.clone(),
        }
    }

    /// Enables edit mode with the given reactive signal.
    #[must_use]
    pub fn editing(mut self, editing: impl Signal<Output = bool> + 'static) -> Self {
        self.editing = editing.computed();
        self
    }

    /// Sets the callback for when any item is deleted.
    #[must_use]
    pub fn on_delete(mut self, on_delete: impl Fn(&Environment, usize) + 'static) -> Self {
        self.on_delete = Some(Box::new(on_delete));
        self
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move(mut self, on_move: impl Fn(&Environment, Move) + 'static) -> Self {
        self.on_move = Some(Box::new(on_move));
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

// ============================================================================
// ListStatefulBuilder - captured state for list event handlers
// ============================================================================

/// Builder for configuring a list with captured state.
pub struct ListStatefulBuilder<V: Views<View = ListItem>, State> {
    contents: V,
    editing: Computed<bool>,
    on_delete: Option<OnDelete>,
    on_move: Option<OnMove>,
    state: State,
}

impl<V: Views<View = ListItem>, S> core::fmt::Debug for ListStatefulBuilder<V, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListStatefulBuilder")
    }
}

impl<V, __S> ListStatefulBuilder<V, __S>
where
    V: Views<View = ListItem>,
    __S: Clone + 'static,
{
    #[must_use]
    pub fn with_state<__T: Clone + 'static>(self, state: &__T) -> ListStatefulBuilder<V, (__S, __T)> {
        ListStatefulBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: self.on_delete,
            on_move: self.on_move,
            state: (self.state, state.clone()),
        }
    }
}

impl<V, S> ListStatefulBuilder<V, S>
where
    V: Views<View = ListItem>,
    S: Clone + 'static,
{
    /// Enables edit mode with the given reactive signal.
    #[must_use]
    pub fn editing(mut self, editing: impl Signal<Output = bool> + 'static) -> Self {
        self.editing = editing.computed();
        self
    }

    /// Sets the callback for when any item is deleted.
    #[must_use]
    pub fn on_delete(mut self, on_delete: impl Fn(S, &Environment, usize) + 'static) -> Self {
        let state = self.state.clone();
        self.on_delete = Some(Box::new(move |env, index| {
            on_delete(state.clone(), env, index);
        }));
        self
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move(mut self, on_move: impl Fn(S, &Environment, Move) + 'static) -> Self {
        let state = self.state.clone();
        self.on_move = Some(Box::new(move |env, movement| {
            on_move(state.clone(), env, movement);
        }));
        self
    }
}

impl<V, S> ConfigurableView for ListStatefulBuilder<V, S>
where
    V: Views<View = ListItem> + 'static,
    S: 'static,
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

impl<V, S> View for ListStatefulBuilder<V, S>
where
    V: Views<View = ListItem> + 'static,
    S: 'static,
{
    fn body(self, env: &Environment) -> impl View {
        render_list_config(ConfigurableView::config(self), env)
    }
}

// ============================================================================
// ListItem - Individual item in a list
// ============================================================================

/// An item in a list that can be configured with various behaviors.
pub struct ListItem {
    /// The view content to display for this item.
    pub content: AnyView,
    /// Read-only signal indicating whether this item can be deleted.
    pub deletable: Computed<bool>,
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
    /// By default, the item is deletable (if the list has on_delete).
    pub fn new(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            deletable: Computed::new(true),
        }
    }

    /// Sets whether this item can be deleted using a reactive signal.
    ///
    /// When false, swipe-to-delete and delete button are disabled for this item.
    #[must_use]
    pub fn deletable(mut self, deletable: impl Signal<Output = bool> + 'static) -> Self {
        self.deletable = deletable.computed();
        self
    }
}
