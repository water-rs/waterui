//! List component implementation for `WaterUI`.
//!
//! This module provides the necessary components to build and configure lists
//! in the `WaterUI` framework. It includes the `List` component for displaying collections
//! of data, and `ListItem` for configuring individual items in the list.
//!

use alloc::boxed::Box;
use nami::collection::Collection;
use nami::{Computed, Signal};

use crate::views::{AnyViews, ForEach, SharedAnyViews, Views, ViewsExt};
use nami::SignalExt;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{
    AnyView, Environment, Native, NativeView, View, id::Identifiable, layout::StretchAxis,
};

/// Callback type for delete operations (receives environment and index).
pub type OnDelete = Box<dyn Fn(&Environment, usize)>;

/// Callback type for move/reorder operations (receives environment, from index, to index).
pub type OnMove = Box<dyn Fn(&Environment, usize, usize)>;

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

    /// Creates a delete handler builder for use with `.with_state()`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// List::new(items)
    ///     .on_delete_builder()
    ///     .with_state(&collection)
    ///     .handler(|collection, _env, idx| collection.remove(idx))
    /// ```
    #[must_use]
    pub fn on_delete_builder(self) -> ListDeleteBuilder<V> {
        ListDeleteBuilder { contents: self.0 }
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move(self, on_move: impl Fn(&Environment, usize, usize) + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.0,
            editing: Computed::new(false),
            on_delete: None,
            on_move: Some(Box::new(on_move)),
        }
    }

    /// Creates a move handler builder for use with `.with_state()`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// List::new(items)
    ///     .on_move_builder()
    ///     .with_state(&collection)
    ///     .handler(|collection, _env, from, to| collection.move_item(from, to))
    /// ```
    #[must_use]
    pub fn on_move_builder(self) -> ListMoveBuilder<V> {
        ListMoveBuilder { contents: self.0 }
    }
}

impl<C, F> List<ForEach<C, F, ListItem>>
where
    C: Collection,
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

impl<V> View for List<V>
where
    V: Views<View = ListItem> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let config = ConfigurableView::config(self);
        // User customization via Hook takes precedence
        if let Some(hook) = env.get::<Hook<ListConfig>>() {
            return AnyView::new(hook.apply(env, config));
        }
        // Native backend can catch ListConfig, otherwise falls back to Lazy::vstack
        let fallback =
            crate::component::lazy::Lazy::vstack(config.contents.clone().map(|item| item.content));
        AnyView::new(Native::new(config).with_fallback(fallback))
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

    /// Creates a delete handler builder for use with `.with_state()`.
    #[must_use]
    pub fn on_delete_builder(self) -> ListBuilderDeleteBuilder<V> {
        ListBuilderDeleteBuilder {
            contents: self.contents,
            editing: self.editing,
            on_move: self.on_move,
        }
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move(mut self, on_move: impl Fn(&Environment, usize, usize) + 'static) -> Self {
        self.on_move = Some(Box::new(on_move));
        self
    }

    /// Creates a move handler builder for use with `.with_state()`.
    #[must_use]
    pub fn on_move_builder(self) -> ListBuilderMoveBuilder<V> {
        ListBuilderMoveBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: self.on_delete,
        }
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
        let config = ConfigurableView::config(self);
        // User customization via Hook takes precedence
        if let Some(hook) = env.get::<Hook<ListConfig>>() {
            return AnyView::new(hook.apply(env, config));
        }
        // Native backend can catch ListConfig, otherwise falls back to Lazy::vstack
        let fallback =
            crate::component::lazy::Lazy::vstack(config.contents.clone().map(|item| item.content));
        AnyView::new(Native::new(config).with_fallback(fallback))
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

// ============================================================================
// List Delete Builder
// ============================================================================

/// Builder for creating list delete handlers with captured state.
pub struct ListDeleteBuilder<V: Views<View = ListItem>> {
    contents: V,
}

impl<V: Views<View = ListItem>> core::fmt::Debug for ListDeleteBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListDeleteBuilder")
    }
}

impl<V: Views<View = ListItem>> ListDeleteBuilder<V> {
    /// Sets the delete handler (no state).
    #[must_use]
    pub fn handler(self, on_delete: impl Fn(&Environment, usize) + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            editing: Computed::new(false),
            on_delete: Some(Box::new(on_delete)),
            on_move: None,
        }
    }

    /// Adds state to capture for the handler.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ListDeleteStatefulBuilder<V, T> {
        ListDeleteStatefulBuilder {
            contents: self.contents,
            state: state.clone(),
        }
    }
}

/// Builder for list delete handlers with captured state.
pub struct ListDeleteStatefulBuilder<V: Views<View = ListItem>, State> {
    contents: V,
    state: State,
}

impl<V: Views<View = ListItem>, S> core::fmt::Debug for ListDeleteStatefulBuilder<V, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListDeleteStatefulBuilder")
    }
}

impl<V: Views<View = ListItem>, S: Clone + 'static> ListDeleteStatefulBuilder<V, S> {
    /// Adds another state value, accumulating as nested tuples.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ListDeleteStatefulBuilder<V, (S, T)> {
        ListDeleteStatefulBuilder {
            contents: self.contents,
            state: (self.state, state.clone()),
        }
    }

    /// Sets the delete handler with captured state.
    #[must_use]
    pub fn handler(self, on_delete: impl Fn(S, &Environment, usize) + 'static) -> ListBuilder<V> {
        let state = self.state;
        ListBuilder {
            contents: self.contents,
            editing: Computed::new(false),
            on_delete: Some(Box::new(move |env, idx| on_delete(state.clone(), env, idx))),
            on_move: None,
        }
    }
}

// ============================================================================
// List Move Builder
// ============================================================================

/// Builder for creating list move handlers with captured state.
pub struct ListMoveBuilder<V: Views<View = ListItem>> {
    contents: V,
}

impl<V: Views<View = ListItem>> core::fmt::Debug for ListMoveBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListMoveBuilder")
    }
}

impl<V: Views<View = ListItem>> ListMoveBuilder<V> {
    /// Sets the move handler (no state).
    #[must_use]
    pub fn handler(self, on_move: impl Fn(&Environment, usize, usize) + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            editing: Computed::new(false),
            on_delete: None,
            on_move: Some(Box::new(on_move)),
        }
    }

    /// Adds state to capture for the handler.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ListMoveStatefulBuilder<V, T> {
        ListMoveStatefulBuilder {
            contents: self.contents,
            state: state.clone(),
        }
    }
}

/// Builder for list move handlers with captured state.
pub struct ListMoveStatefulBuilder<V: Views<View = ListItem>, State> {
    contents: V,
    state: State,
}

impl<V: Views<View = ListItem>, S> core::fmt::Debug for ListMoveStatefulBuilder<V, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListMoveStatefulBuilder")
    }
}

impl<V: Views<View = ListItem>, S: Clone + 'static> ListMoveStatefulBuilder<V, S> {
    /// Adds another state value, accumulating as nested tuples.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ListMoveStatefulBuilder<V, (S, T)> {
        ListMoveStatefulBuilder {
            contents: self.contents,
            state: (self.state, state.clone()),
        }
    }

    /// Sets the move handler with captured state.
    #[must_use]
    pub fn handler(
        self,
        on_move: impl Fn(S, &Environment, usize, usize) + 'static,
    ) -> ListBuilder<V> {
        let state = self.state;
        ListBuilder {
            contents: self.contents,
            editing: Computed::new(false),
            on_delete: None,
            on_move: Some(Box::new(move |env, from, to| {
                on_move(state.clone(), env, from, to);
            })),
        }
    }
}

// ============================================================================
// ListBuilder Delete Builder (preserves existing ListBuilder state)
// ============================================================================

/// Builder for creating list delete handlers that preserves existing ListBuilder state.
pub struct ListBuilderDeleteBuilder<V: Views<View = ListItem>> {
    contents: V,
    editing: Computed<bool>,
    on_move: Option<OnMove>,
}

impl<V: Views<View = ListItem>> core::fmt::Debug for ListBuilderDeleteBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListBuilderDeleteBuilder")
    }
}

impl<V: Views<View = ListItem>> ListBuilderDeleteBuilder<V> {
    /// Sets the delete handler (no state).
    #[must_use]
    pub fn handler(self, on_delete: impl Fn(&Environment, usize) + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: Some(Box::new(on_delete)),
            on_move: self.on_move,
        }
    }

    /// Adds state to capture for the handler.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(
        self,
        state: &T,
    ) -> ListBuilderDeleteStatefulBuilder<V, T> {
        ListBuilderDeleteStatefulBuilder {
            contents: self.contents,
            editing: self.editing,
            on_move: self.on_move,
            state: state.clone(),
        }
    }
}

/// Builder for list delete handlers with captured state that preserves existing ListBuilder state.
pub struct ListBuilderDeleteStatefulBuilder<V: Views<View = ListItem>, State> {
    contents: V,
    editing: Computed<bool>,
    on_move: Option<OnMove>,
    state: State,
}

impl<V: Views<View = ListItem>, S> core::fmt::Debug for ListBuilderDeleteStatefulBuilder<V, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListBuilderDeleteStatefulBuilder")
    }
}

impl<V: Views<View = ListItem>, S: Clone + 'static> ListBuilderDeleteStatefulBuilder<V, S> {
    /// Adds another state value, accumulating as nested tuples.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(
        self,
        state: &T,
    ) -> ListBuilderDeleteStatefulBuilder<V, (S, T)> {
        ListBuilderDeleteStatefulBuilder {
            contents: self.contents,
            editing: self.editing,
            on_move: self.on_move,
            state: (self.state, state.clone()),
        }
    }

    /// Sets the delete handler with captured state.
    #[must_use]
    pub fn handler(self, on_delete: impl Fn(S, &Environment, usize) + 'static) -> ListBuilder<V> {
        let state = self.state;
        ListBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: Some(Box::new(move |env, idx| on_delete(state.clone(), env, idx))),
            on_move: self.on_move,
        }
    }
}

// ============================================================================
// ListBuilder Move Builder (preserves existing ListBuilder state)
// ============================================================================

/// Builder for creating list move handlers that preserves existing ListBuilder state.
pub struct ListBuilderMoveBuilder<V: Views<View = ListItem>> {
    contents: V,
    editing: Computed<bool>,
    on_delete: Option<OnDelete>,
}

impl<V: Views<View = ListItem>> core::fmt::Debug for ListBuilderMoveBuilder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListBuilderMoveBuilder")
    }
}

impl<V: Views<View = ListItem>> ListBuilderMoveBuilder<V> {
    /// Sets the move handler (no state).
    #[must_use]
    pub fn handler(self, on_move: impl Fn(&Environment, usize, usize) + 'static) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: self.on_delete,
            on_move: Some(Box::new(on_move)),
        }
    }

    /// Adds state to capture for the handler.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ListBuilderMoveStatefulBuilder<V, T> {
        ListBuilderMoveStatefulBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: self.on_delete,
            state: state.clone(),
        }
    }
}

/// Builder for list move handlers with captured state that preserves existing ListBuilder state.
pub struct ListBuilderMoveStatefulBuilder<V: Views<View = ListItem>, State> {
    contents: V,
    editing: Computed<bool>,
    on_delete: Option<OnDelete>,
    state: State,
}

impl<V: Views<View = ListItem>, S> core::fmt::Debug for ListBuilderMoveStatefulBuilder<V, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ListBuilderMoveStatefulBuilder")
    }
}

impl<V: Views<View = ListItem>, S: Clone + 'static> ListBuilderMoveStatefulBuilder<V, S> {
    /// Adds another state value, accumulating as nested tuples.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(
        self,
        state: &T,
    ) -> ListBuilderMoveStatefulBuilder<V, (S, T)> {
        ListBuilderMoveStatefulBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: self.on_delete,
            state: (self.state, state.clone()),
        }
    }

    /// Sets the move handler with captured state.
    #[must_use]
    pub fn handler(
        self,
        on_move: impl Fn(S, &Environment, usize, usize) + 'static,
    ) -> ListBuilder<V> {
        let state = self.state;
        ListBuilder {
            contents: self.contents,
            editing: self.editing,
            on_delete: self.on_delete,
            on_move: Some(Box::new(move |env, from, to| {
                on_move(state.clone(), env, from, to);
            })),
        }
    }
}
