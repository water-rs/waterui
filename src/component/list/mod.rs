//! List component implementation for `WaterUI`.
//!
//! This module provides the necessary components to build and configure lists
//! in the `WaterUI` framework. It includes the `List` component for displaying
//! collections of data, [`ListItem`] for configuring individual items, and
//! the [`ListContent`] / [`Section`] / [`row`] surface for composing static
//! heterogeneous lists with sections.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use core::ops::RangeBounds;
use nami::collection::{Collection, CollectionChange};
use nami::watcher::Context;
use nami::{Binding, Computed, signal::IntoComputed};

use crate::views::{AnyViews, ForEach, SharedAnyViews, Views, ViewsExt};
use waterui_core::id::{Id as RawId, Mapping, SelfId};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{
    AnyView, Environment, Metadata, Native, NativeView, View,
    handler::{AnyViewBuilder, Handler, shared_action},
    id::Identifiable,
    impl_extractor,
    layout::StretchAxis,
};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::scroll::ScrollController;
use waterui_text::{IntoText, Text};

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

/// Selection a list binds to row identity: no selection, a single row, or a
/// set of rows — at most one mode per list.
///
/// The generic `Id` is the row's identity type: `V::Id` while the binding is
/// stored typed on [`ListBuilder`], `SelfId<RawId>` after `config()` erases
/// it through the same generator the collection's `get_id` reports, so the
/// backends and the FFI read and write exactly the ids the rows answer to.
#[derive(Clone, Default)]
pub enum ListSelection<Id: 'static> {
    /// Rows are not selectable.
    #[default]
    None,
    /// A single selected row — `None` inside the binding means nothing is
    /// selected.
    Single(Binding<Option<Id>>),
    /// The set of selected row ids.
    Multiple(Binding<BTreeSet<Id>>),
}

impl<Id: 'static> core::fmt::Debug for ListSelection<Id> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(core::any::type_name::<Self>())
    }
}

impl<Id: 'static + Ord + Clone> ListSelection<Id> {
    /// Re-keys the bindings through `ids`, the same generator the erased
    /// contents feed: reads register each `Id` to the `SelfId<RawId>` the
    /// collection reports, and backend writes resolve it back — both
    /// directions the way `Mapping::binding` maps a `Picker` selection.
    fn erased(self, ids: &Mapping<Id>) -> ListSelection<SelfId<RawId>> {
        match self {
            Self::None => ListSelection::None,
            Self::Single(binding) => {
                let to = ids.clone();
                let from = ids.clone();
                ListSelection::Single(Binding::mapping(
                    &binding,
                    move |selected| selected.map(|id| SelfId::new(to.to_id(id))),
                    move |binding, erased| {
                        binding.set(erased.map(|id| {
                            from.to_data(id.into_inner()).expect(
                                "list selection row id is not registered in the list's id mapping",
                            )
                        }));
                    },
                ))
            }
            Self::Multiple(binding) => {
                let to = ids.clone();
                let from = ids.clone();
                ListSelection::Multiple(Binding::mapping(
                    &binding,
                    move |selected| {
                        selected
                            .iter()
                            .map(|id| SelfId::new(to.to_id(id.clone())))
                            .collect()
                    },
                    move |binding, erased: BTreeSet<SelfId<RawId>>| {
                        binding.set(
                            erased
                                .iter()
                                .map(|id| {
                                    from.to_data(id.into_inner()).expect(
                                        "list selection row id is not registered in the list's id mapping",
                                    )
                                })
                                .collect(),
                        );
                    },
                ))
            }
        }
    }
}

/// Configuration for a list component.
pub struct ListConfig {
    /// Content items to be displayed in the list.
    pub contents: SharedAnyViews<ListItem>,
    /// The list's selection bindings, keyed by the same erased row ids
    /// `contents.get_id` returns. `ListSelection::None` when the list is not
    /// selectable.
    pub selection: ListSelection<SelfId<RawId>>,
    /// Read-only signal for edit mode state.
    pub editing: Computed<bool>,
    /// Optional callback when any item is deleted.
    pub on_delete: Option<OnDelete>,
    /// Optional callback when items are moved/reordered.
    pub on_move: Option<OnMove>,
    /// Optional programmatic item-index scroll controller.
    pub scroll_controller: Option<ScrollController<usize>>,
    /// Whether rows carry semantic section markers.
    pub uses_sections: bool,
    /// The minimum height of a row, from [`ListMinRowHeight`] in the list's
    /// environment. `None` uses the theme's one-line row height.
    pub min_row_height: Option<f32>,
}

impl_debug!(ListConfig);

/// The minimum height of every row in the lists below it, in points.
///
/// Replaces the theme's one-line row height as the floor a row is measured
/// against. `0.0` sizes each row to its content plus its insets. Set it with
/// [`ViewExt::list_min_row_height`](crate::ViewExt::list_min_row_height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMinRowHeight(pub f32);

impl NativeView for ListConfig {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// A component that displays items in a list format.
#[derive(Debug)]
pub struct List<V: Views<View = ListItem> = AnyViews<ListItem>> {
    contents: V,
    uses_sections: bool,
}

impl<V> List<V>
where
    V: Views<View = ListItem>,
{
    /// Creates a new list with the specified contents.
    pub const fn new(contents: V) -> Self {
        Self {
            contents,
            uses_sections: true,
        }
    }

    /// Enables edit mode with the given reactive signal.
    ///
    /// When edit mode is enabled, delete buttons and drag handles are shown.
    #[must_use]
    pub fn editing(self, editing: impl IntoComputed<bool>) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            editing: editing.into_computed(),
            selection: ListSelection::None,
            on_delete: None,
            on_move: None,
            scroll_controller: None,
            uses_sections: self.uses_sections,
        }
    }

    /// Sets the callback for when any item is deleted.
    #[must_use]
    pub fn on_delete<H, Args>(self, on_delete: H) -> ListBuilder<V>
    where
        H: Handler<Args, ()>,
    {
        ListBuilder {
            contents: self.contents,
            editing: Computed::new(false),
            selection: ListSelection::None,
            on_delete: Some(list_delete_action(on_delete)),
            on_move: None,
            scroll_controller: None,
            uses_sections: self.uses_sections,
        }
    }

    /// Sets the callback for when items are moved/reordered.
    #[must_use]
    pub fn on_move<H, Args>(self, on_move: H) -> ListBuilder<V>
    where
        H: Handler<Args, ()>,
    {
        ListBuilder {
            contents: self.contents,
            editing: Computed::new(false),
            selection: ListSelection::None,
            on_delete: None,
            on_move: Some(list_move_action(on_move)),
            scroll_controller: None,
            uses_sections: self.uses_sections,
        }
    }

    /// Connects a controller that jumps the list to a requested item index.
    #[must_use]
    pub fn scroll_controller(self, controller: &ScrollController<usize>) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            selection: ListSelection::None,
            editing: Computed::new(false),
            on_delete: None,
            on_move: None,
            scroll_controller: Some(controller.clone()),
            uses_sections: self.uses_sections,
        }
    }

    /// Single selection keyed by row identity. The framework and backends
    /// write it on pointer, keyboard and accessibility input.
    #[must_use]
    pub fn selection(self, selection: &Binding<Option<V::Id>>) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            selection: ListSelection::Single(selection.clone()),
            editing: Computed::new(false),
            on_delete: None,
            on_move: None,
            scroll_controller: None,
            uses_sections: self.uses_sections,
        }
    }

    /// Multiple selection keyed by row identity.
    #[must_use]
    pub fn multi_selection(self, selection: &Binding<BTreeSet<V::Id>>) -> ListBuilder<V> {
        ListBuilder {
            contents: self.contents,
            selection: ListSelection::Multiple(selection.clone()),
            editing: Computed::new(false),
            on_delete: None,
            on_move: None,
            scroll_controller: None,
            uses_sections: self.uses_sections,
        }
    }
}

impl<C, F> List<ForEach<C, F, ListItem>>
where
    C: Collection + Clone,
    C::Item: Identifiable,
    F: 'static + Fn(C::Item) -> ListItem,
{
    /// Creates a lazy list over an identity-keyed reactive collection.
    ///
    /// Renderers request only rows in the visible viewport. Programmatic jumps
    /// through [`ScrollController`] therefore do not materialize preceding rows.
    /// Use [`List::content`] instead when rows carry semantic section markers.
    pub const fn for_each(data: C, generator: F) -> Self {
        Self {
            contents: ForEach::new(data, generator),
            uses_sections: false,
        }
    }
}

impl List<BuiltViews> {
    /// Creates a list from a static [`ListContent`] tree.
    ///
    /// `ListContent` accepts `|| ListItem::new(…)` row builders, [`Row`] (the
    /// [`row`] / [`detail_row`] helpers produce one), [`Section<C>`], tuples,
    /// arrays, vectors, and `Option<T>`, so heterogeneous content composes
    /// structurally just like `SwiftUI`'s `Section { row; row }` form. A row is
    /// a builder rather than a finished [`ListItem`] because the list rebuilds
    /// it whenever the row is realized again. For dynamic identity-keyed data,
    /// use [`List::for_each`] instead; for any pre-built [`Views`]
    /// implementation, use [`List::new`].
    #[must_use]
    pub fn content(content: impl ListContent) -> Self {
        let mut sink = ListItemSink::new();
        content.collect_items(&mut sink);
        Self {
            contents: BuiltViews::new(sink),
            uses_sections: true,
        }
    }
}

/// `Views` adapter that materializes the entries collected by a
/// [`ListContent`] tree on demand.
///
/// Each entry stores a cloneable builder that produces a fresh [`ListItem`]
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
        _watcher: impl for<'a> Fn(Context<&'a [Self::Id]>, CollectionChange) + 'static,
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
            contents: SharedAnyViews::new(self.contents),
            selection: ListSelection::None,
            editing: Computed::new(false),
            on_delete: None,
            on_move: None,
            scroll_controller: None,
            uses_sections: self.uses_sections,
            min_row_height: None,
        }
    }
}

impl ViewConfiguration for ListConfig {
    type View = List<SharedAnyViews<ListItem>>;

    fn render(self) -> Self::View {
        List {
            contents: self.contents,
            uses_sections: self.uses_sections,
        }
    }
}

impl From<ListConfig> for List<SharedAnyViews<ListItem>> {
    fn from(value: ListConfig) -> Self {
        value.render()
    }
}

fn render_list_config(mut config: ListConfig, env: &Environment) -> impl View {
    // Section headers and footers are semantic text: they localize, and they
    // may be driven by a signal. Resolving them here is the one place the
    // environment is in hand — rows are materialized lazily by the renderer,
    // long after this body has run.
    let section_env = env.clone();
    // Erasing the themed contents again re-keys every row id, so the erased
    // selection travels through the same generator — a backend's `get_id`
    // must return exactly the ids the selection is keyed by.
    let selection = config.selection.clone();
    let theme_selection = selection.clone();
    let (contents, ids) = AnyViews::new_with_ids(WithId {
        contents: config.contents.clone(),
        transform: move |id, item| {
            selection_themed(
                resolve_item_section(item, &section_env),
                &theme_selection,
                id,
            )
        },
    });
    config.contents = SharedAnyViews::from(contents);
    config.selection = selection.erased(&ids);
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
    transform: F,
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

    fn len(&self) -> Computed<usize> {
        self.contents.len()
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

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl for<'a> Fn(Context<&'a [Self::Id]>, CollectionChange) + 'static,
    ) -> Self::Guard {
        self.contents.watch(range, watcher)
    }
}

/// Resolves the section marker a row carries against the list's environment.
fn resolve_item_section(mut item: ListItem, env: &Environment) -> ListItem {
    item.section = item.section.map(|section| section.resolved(env));
    item
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
/// the row itself is not rebuilt.
fn selection_themed(
    mut item: ListItem,
    selection: &ListSelection<SelfId<RawId>>,
    id: SelfId<RawId>,
) -> ListItem {
    use crate::color::ResolvedColor;
    use crate::theme::{color, install_color_signal};
    use nami::SignalExt;
    use waterui_core::env::use_env;
    use waterui_core::resolve::Resolvable;

    // A row's selected state derives from the list selection and the row's
    // own id: the row is selected exactly when its id is in the binding.
    let selected: Computed<bool> = match selection {
        ListSelection::None => return item,
        ListSelection::Single(selection) => selection
            .clone()
            .map(move |current| current == Some(id))
            .computed(),
        ListSelection::Multiple(selection) => selection
            .clone()
            .map(move |current| current.contains(&id))
            .computed(),
    };
    let content = core::mem::take(&mut item.content);
    item.content = AnyView::new(use_env(move |mut env: Environment| {
        let on_selection = color::SelectionForeground.resolve(&env).computed();
        let flip = |normal: Computed<ResolvedColor>| {
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
        Metadata::new(content, env)
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
    selection: ListSelection<V::Id>,
    editing: Computed<bool>,
    on_delete: Option<OnDelete>,
    on_move: Option<OnMove>,
    scroll_controller: Option<ScrollController<usize>>,
    uses_sections: bool,
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

    /// Connects a controller that jumps the list to a requested item index.
    #[must_use]
    pub fn scroll_controller(mut self, controller: &ScrollController<usize>) -> Self {
        self.scroll_controller = Some(controller.clone());
        self
    }

    /// Single selection keyed by row identity. The framework and backends
    /// write it on pointer, keyboard and accessibility input.
    #[must_use]
    pub fn selection(mut self, selection: &Binding<Option<V::Id>>) -> Self {
        self.selection = ListSelection::Single(selection.clone());
        self
    }

    /// Multiple selection keyed by row identity.
    #[must_use]
    pub fn multi_selection(mut self, selection: &Binding<BTreeSet<V::Id>>) -> Self {
        self.selection = ListSelection::Multiple(selection.clone());
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
            selection: self.selection.erased(&ids),
            editing: self.editing,
            on_delete: self.on_delete,
            on_move: self.on_move,
            scroll_controller: self.scroll_controller,
            uses_sections: self.uses_sections,
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
///
/// The header and footer are [`Text`], not strings: a section title localizes
/// and may be driven by a signal ("3 unread"), and the backend follows that
/// signal in place rather than rebuilding the section. The chrome's
/// typography belongs to the platform, so a header takes the list's own
/// section style regardless of any styling applied to the text.
#[derive(Debug, Clone, Default)]
pub struct ListSection {
    /// Title shown above the section.
    pub label: Option<Text>,
    /// Footer text shown below the section.
    pub footer: Option<Text>,
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
    fn resolved(self, env: &Environment) -> Self {
        Self {
            label: self.label.map(|label| Text::from(label.resolve(env))),
            footer: self.footer.map(|footer| Text::from(footer.resolve(env))),
        }
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
    /// The insets between the row's edges and its content. `None` uses the
    /// theme's row insets.
    pub insets: Option<EdgeInsets>,
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
            List::content((
                || ListItem::new(text!("Alice joined")),
                || ListItem::new(text!("Hello")),
            ))
            .list_min_row_height(0.0),
        );
        let list = *content
            .downcast::<List<BuiltViews>>()
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
        let selection = nami::Binding::container(Option::<SelfId<usize>>::None);
        let config = render_config(
            ConfigurableView::config(
                List::content((
                    || ListItem::new(text!("Alice joined")).insets(EdgeInsets::all(4.0)),
                    || ListItem::new(text!("Hello")),
                ))
                .selection(&selection),
            ),
            &Environment::new(),
        );
        let first = config.contents.get_view(0).expect("row 0 exists");
        let second = config.contents.get_view(1).expect("row 1 exists");
        assert_eq!(first.insets, Some(EdgeInsets::all(4.0)));
        assert_eq!(second.insets, None);
    }
}
