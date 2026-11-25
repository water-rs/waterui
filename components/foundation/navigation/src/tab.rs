//! Native adaptive tab navigation.

use alloc::{rc::Rc, vec, vec::Vec};
use core::fmt::Debug;
use core::marker::PhantomData;
use core::ops::{Range, RangeBounds};

use nami::collection::{Collection, CollectionChange};
use nami::watcher::Context;
use nami::{Binding, Computed, SignalExt as _};
pub use waterui_controls::{BadgeValue, SlotBuilder};
use waterui_controls::{IconSlot, IntoLabel, LabelKind, TitleLabel, label::Label};
use waterui_core::{
    AnyView, Environment, IntoSignal, Native, NativeView, View,
    backend::{Backend, ChildSlot, MapSlots},
    handler::ViewBuilder,
    id::{Id, Mapping},
    layout::StretchAxis,
    views::{AnyViews, ForEach, ViewSnapshot, Views},
};

/// Adaptive presentation style for a [`Tabs`] container.
#[expect(
    clippy::exhaustive_enums,
    reason = "Step 3 marks TabStyle #[non_exhaustive] once split crate hydrolysis-m3 updates its match on item.placement"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TabStyle {
    /// Platform- and window-adaptive default.
    #[default]
    Automatic = 0,
    /// Always request the platform tab bar presentation.
    TabBar = 1,
    /// Always request the platform sidebar or navigation-rail presentation.
    Sidebar = 2,
}

/// Temporary compatibility alias for split crates during the `#4431` migration.
#[doc(hidden)]
pub type NativeTabStyle = TabStyle;

/// Temporary compatibility constructors for split crates during the `#4431` migration.
#[doc(hidden)]
pub mod tab_style {
    use super::TabStyle;

    /// Returns [`TabStyle::Automatic`].
    #[must_use]
    pub const fn automatic() -> TabStyle {
        TabStyle::Automatic
    }

    /// Returns [`TabStyle::TabBar`].
    #[must_use]
    pub const fn tab_bar() -> TabStyle {
        TabStyle::TabBar
    }

    /// Returns [`TabStyle::Sidebar`].
    #[must_use]
    pub const fn sidebar() -> TabStyle {
        TabStyle::Sidebar
    }
}

/// The part a tab plays in the container's chrome.
///
/// A role is an attribute of a tab, not a different kind of tab: the tab keeps
/// its label, icon, badge and content, and the platform decides how a tab
/// with that role is presented. On iOS the search role is the system's search
/// tab — placed trailing, with its own glass treatment and the platform's own
/// search presentation. Platforms without such a concept present the tab as a
/// regular one.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabRole {
    /// An ordinary section of the app.
    #[default]
    Regular,
    /// The tab the platform presents as search.
    Search,
}

/// How the tab bar behaves while the selected tab's content scrolls.
///
/// A platform convention, not a semantic requirement: iOS 26 can collapse the
/// tab bar into a compact glass pill as the user scrolls and expand it again
/// on the way back. `Automatic` is the platform's own default — on iOS the bar
/// stays expanded until an app opts in. Platforms whose tab chrome does not
/// collapse ignore the behavior.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabBarMinimizeBehavior {
    /// The platform's default.
    #[default]
    Automatic,
    /// The bar never minimizes.
    Never,
    /// The bar minimizes while scrolling down and expands when scrolling back up.
    OnScrollDown,
    /// The bar minimizes while scrolling up and expands when scrolling back down.
    OnScrollUp,
}

/// Marker on [`Tab`] when no `.tag(...)` value has been attached.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Untagged;

/// Marker on [`Tab`] carrying a selection tag `T`.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct TabTag<T>(T);

/// One stable native tab.
///
/// By default, the collection a tab lives in supplies its positional or item
/// identity; chaining [`Tab::tag`] keys selection by an application value `T`
/// (such as a `Pane` enum) without `ForEach` or `SelfId`.
///
/// `C` builds the tab's root page and `L` keeps the tab label's concrete
/// [`LabelKind`] until lowered into a backend's child slot. `Clone` shares the
/// builder and signals — a `Vec<Tab<C, L, Tag>>` collection clones on snapshot.
#[non_exhaustive]
pub struct Tab<C, L = TitleLabel, Tag = Untagged> {
    /// Semantic tab label.
    label: Label<L>,
    /// Stable lazily built tab root.
    content: Rc<C>,
    /// Optional reactive badge value.
    badge: Option<Computed<BadgeValue>>,
    /// Whether the native tab item is enabled.
    enabled: Computed<bool>,
    /// The part the tab plays in the container's chrome.
    role: TabRole,
    /// Optional selection tag attached via [`Tab::tag`].
    tag: Tag,
}

impl<C, L: Clone, Tag: Clone> Clone for Tab<C, L, Tag> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            content: Rc::clone(&self.content),
            badge: self.badge.clone(),
            enabled: self.enabled.clone(),
            role: self.role,
            tag: self.tag.clone(),
        }
    }
}

impl<C, L, Tag> Debug for Tab<C, L, Tag> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tab")
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl<C, L> Tab<C, L, Untagged>
where
    C: ViewBuilder,
    L: LabelKind + Clone,
{
    /// Creates a stable tab with a semantic label and lazy root builder.
    ///
    /// The root may be any view, including a [`NavigationStack`], a
    /// [`NavigationSplitView`], a [`NavigationView`], or plain view content.
    /// Chain [`Tab::tag`] to key the tab by an application selection value; tabs
    /// without a tag fall back to the collection's positional or `ForEach` identity.
    ///
    /// [`NavigationStack`]: crate::NavigationStack
    /// [`NavigationSplitView`]: crate::NavigationSplitView
    /// [`NavigationView`]: crate::NavigationView
    pub fn new<I: IntoLabel<Kind = L>>(label: I, content: C) -> Self {
        Self {
            label: label.into_label(),
            content: Rc::new(content),
            badge: None,
            enabled: Computed::constant(true),
            role: TabRole::Regular,
            tag: Untagged,
        }
    }
}

impl<C, L, Tag> Tab<C, L, Tag>
where
    C: ViewBuilder,
    L: LabelKind + Clone,
{
    /// Lowers the tab into its backend [`TabItem`] and returns its attached tag.
    fn into_parts_with_tag<S: ChildSlot>(self) -> (TabItem<S>, Tag) {
        let Self {
            label,
            content,
            badge,
            enabled,
            role,
            tag,
        } = self;
        let icon = label.icon_slot::<S>();
        (
            TabItem {
                label: SlotBuilder::new(move || S::from_view(label.clone())),
                icon,
                content: SlotBuilder::new(move || S::from_view(content.build())),
                badge,
                enabled,
                role,
            },
            tag,
        )
    }

    /// The tab with its parts built in the child slot `S`.
    fn into_item<S: ChildSlot>(self) -> TabItem<S> {
        self.into_parts_with_tag().0
    }
}

impl<C, L: LabelKind, Tag> Tab<C, L, Tag> {
    /// Whether this tab's label carries an icon.
    #[must_use]
    pub fn has_icon(&self) -> bool {
        self.label.has_icon()
    }
}

impl<C, L, Tag> Tab<C, L, Tag> {
    /// Semantic tab label.
    #[must_use]
    pub const fn label(&self) -> &Label<L> {
        &self.label
    }

    /// Stable lazily built tab root.
    #[must_use]
    pub fn content(&self) -> &C {
        &self.content
    }

    /// Optional reactive badge value.
    #[must_use]
    pub const fn tab_badge(&self) -> Option<&Computed<BadgeValue>> {
        self.badge.as_ref()
    }

    /// Whether the native tab item is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> &Computed<bool> {
        &self.enabled
    }

    /// The part the tab plays in the container's chrome.
    #[must_use]
    pub const fn tab_role(&self) -> TabRole {
        self.role
    }

    /// Associates a selection value with this tab for [`Tabs`].
    #[must_use]
    pub fn tag<T: Ord + Clone + 'static>(self, value: T) -> Tab<C, L, TabTag<T>> {
        Tab {
            label: self.label,
            content: self.content,
            badge: self.badge,
            enabled: self.enabled,
            role: self.role,
            tag: TabTag(value),
        }
    }

    /// Sets a reactive badge value (`u32`, [`BadgeValue`], or signal; `0` hides).
    #[must_use]
    pub fn badge(mut self, value: impl IntoSignal<BadgeValue> + 'static) -> Self {
        self.badge = Some(value.into_signal().computed());
        self
    }

    /// Controls whether this tab can be selected.
    #[must_use]
    pub fn enabled(mut self, enabled: impl IntoSignal<bool> + 'static) -> Self {
        self.enabled = enabled.into_signal().computed();
        self
    }

    /// Sets the part this tab plays in the container's chrome.
    ///
    /// See [`TabRole`] for what each role means per platform.
    #[must_use]
    pub const fn role(mut self, role: TabRole) -> Self {
        self.role = role;
        self
    }
}

/// A bare tab shows its label — outside a [`Tabs`] container there is no
/// chrome to hang the content on.
impl<C: 'static, L: LabelKind, Tag: 'static> View for Tab<C, L, Tag> {
    fn body(self, _env: &Environment) -> impl View {
        self.label
    }
}

/// A tab as its backend receives it: its label, icon, and root page built in
/// the backend's child slot `S`.
#[non_exhaustive]
pub struct TabItem<S> {
    /// Semantic tab label.
    label: SlotBuilder<S>,
    /// The label's icon, lifted out so native chrome can place or inspect it itself.
    icon: Option<IconSlot<S>>,
    /// Stable lazily built tab root.
    content: SlotBuilder<S>,
    /// Optional reactive badge value.
    badge: Option<Computed<BadgeValue>>,
    /// Whether the native tab item is enabled.
    enabled: Computed<bool>,
    /// The part the tab plays in the container's chrome.
    role: TabRole,
}

impl<S> Clone for TabItem<S> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            icon: self.icon.clone(),
            content: self.content.clone(),
            badge: self.badge.clone(),
            enabled: self.enabled.clone(),
            role: self.role,
        }
    }
}

impl<S> Debug for TabItem<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabItem")
            .field("icon", &self.icon)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl<S: 'static> TabItem<S> {
    /// The same tab with its parts mapped through `f`.
    #[must_use]
    pub fn map_slot<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> TabItem<T> {
        let Self {
            label,
            icon,
            content,
            badge,
            enabled,
            role,
        } = self;
        let icon_f = f.clone();
        let content_f = f.clone();
        TabItem {
            label: SlotBuilder::new(move || f(label.build())),
            icon: icon.map(|icon| icon.map_slot(icon_f)),
            content: SlotBuilder::new(move || content_f(content.build())),
            badge,
            enabled,
            role,
        }
    }
}

impl<S> TabItem<S> {
    /// Semantic tab label.
    #[must_use]
    pub const fn label(&self) -> &SlotBuilder<S> {
        &self.label
    }

    /// The label's icon, lifted out so native chrome can place or inspect it itself.
    #[must_use]
    pub const fn icon(&self) -> Option<&IconSlot<S>> {
        self.icon.as_ref()
    }

    /// Stable lazily built tab root.
    #[must_use]
    pub const fn content(&self) -> &SlotBuilder<S> {
        &self.content
    }

    /// Optional reactive badge value.
    #[must_use]
    pub const fn badge(&self) -> Option<&Computed<BadgeValue>> {
        self.badge.as_ref()
    }

    /// Whether the native tab item is enabled.
    #[must_use]
    pub const fn enabled(&self) -> &Computed<bool> {
        &self.enabled
    }

    /// The part the tab plays in the container's chrome.
    #[must_use]
    pub const fn role(&self) -> TabRole {
        self.role
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    #[expect(
        clippy::type_complexity,
        reason = "the tuple is the value's complete field list"
    )]
    pub fn into_parts(
        self,
    ) -> (
        SlotBuilder<S>,
        Option<IconSlot<S>>,
        SlotBuilder<S>,
        Option<Computed<BadgeValue>>,
        Computed<bool>,
        TabRole,
    ) {
        (
            self.label,
            self.icon,
            self.content,
            self.badge,
            self.enabled,
            self.role,
        )
    }
}

/// Native tab container consumed by platform backends, its tabs and bottom
/// accessory in the backend's child slot `S`.
///
/// The C ABI cannot carry generics, so tab identity is erased to [`Id`] here.
/// Application code uses [`Tabs`], which keeps the app's own identifier type.
/// `tabs` is a reactive collection: a backend mounts its snapshot once and
/// then watches it, applying inserts, removals and moves by tab id so a tab
/// that survives a change keeps its retained state.
#[derive(Debug)]
#[non_exhaustive]
pub struct TabsLayout<S> {
    /// Currently selected tab identifier.
    selection: Binding<Id>,
    /// The tab collection, keyed by erased tab identity.
    tabs: AnyViews<TabItem<S>>,
    /// Native adaptive style.
    style: TabStyle,
    /// How the bar behaves while content scrolls.
    minimize_behavior: TabBarMinimizeBehavior,
    /// A view the platform floats above the tab bar, when it has such a slot.
    bottom_accessory: Option<S>,
}

impl<S> TabsLayout<S> {
    /// Currently selected tab identifier.
    #[must_use]
    pub const fn selection(&self) -> &Binding<Id> {
        &self.selection
    }

    /// The tab collection, keyed by erased tab identity.
    #[must_use]
    pub const fn tabs(&self) -> &AnyViews<TabItem<S>> {
        &self.tabs
    }

    /// Native adaptive style.
    #[must_use]
    pub const fn style(&self) -> TabStyle {
        self.style
    }

    /// How the bar behaves while content scrolls.
    #[must_use]
    pub const fn minimize_behavior(&self) -> TabBarMinimizeBehavior {
        self.minimize_behavior
    }

    /// A view the platform floats above the tab bar, when it has such a slot.
    #[must_use]
    pub const fn bottom_accessory(&self) -> Option<&S> {
        self.bottom_accessory.as_ref()
    }

    /// Takes the `bottom_accessory` out of the value, leaving none behind.
    pub const fn take_bottom_accessory(&mut self) -> Option<S> {
        self.bottom_accessory.take()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        Binding<Id>,
        AnyViews<TabItem<S>>,
        TabStyle,
        TabBarMinimizeBehavior,
        Option<S>,
    ) {
        (
            self.selection,
            self.tabs,
            self.style,
            self.minimize_behavior,
            self.bottom_accessory,
        )
    }
}

impl<S: 'static> TabsLayout<S> {
    /// The same container with every tab's parts and the accessory mapped
    /// through `f`: a backend holds them in its own slot.
    #[must_use]
    pub fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> TabsLayout<T> {
        let Self {
            selection,
            tabs,
            style,
            minimize_behavior,
            bottom_accessory,
        } = self;
        let bottom_accessory = bottom_accessory.map(f.clone());
        TabsLayout {
            selection,
            tabs: tabs.map_views(move |tab| tab.map_slot(f.clone())),
            style,
            minimize_behavior,
            bottom_accessory,
        }
    }
}

impl<S: 'static> MapSlots<S> for TabsLayout<S> {
    type With<T: 'static> = TabsLayout<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> TabsLayout<T> {
        Self::map_slots(self, f)
    }
}

impl<S: 'static> NativeView for TabsLayout<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// The tabs of a [`Tabs`] container: a `Vec` or array of tabs sharing one
/// root builder type, a `ForEach` producing them, or a tuple of tabs whose
/// roots differ.
pub trait TabSet: 'static {
    /// The identity of a tab in the set, which the container's selection
    /// binds.
    type Id: Ord + Clone + 'static;

    /// The tabs with their parts built in the child slot `S`, keyed by erased
    /// identity, with the mapping of the set's ids into it.
    #[doc(hidden)]
    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<Self::Id>);
}

/// View wrapper carrying an already-lowered [`TabItem`] through [`AnyViews::new_with_ids`].
struct SlotTabItem<S>(TabItem<S>);

impl<S: 'static> View for SlotTabItem<S> {
    fn body(self, _env: &Environment) -> impl View {}
}

/// Fixed collection of tagged [`TabItem`]s erased through [`AnyViews::new_with_ids`].
struct TaggedTabItems<T, S> {
    items: Rc<[(T, TabItem<S>)]>,
}

impl<T, S> Clone for TaggedTabItems<T, S> {
    fn clone(&self) -> Self {
        Self {
            items: Rc::clone(&self.items),
        }
    }
}

impl<T: Ord + Clone + 'static, S: 'static> ViewSnapshot for TaggedTabItems<T, S> {
    type Id = T;
    type View = SlotTabItem<S>;

    fn range(&self) -> Range<usize> {
        0..self.items.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.items.get(index).map(|(id, _)| id.clone())
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.items
            .get(index)
            .map(|(_, item)| SlotTabItem(item.clone()))
    }
}

impl<T: Ord + Clone + 'static, S: 'static> Views for TaggedTabItems<T, S> {
    type Id = T;
    type Guard = ();
    type View = SlotTabItem<S>;
    type Snapshot = Self;

    fn len(&self) -> Computed<usize> {
        Computed::constant(self.items.len())
    }

    fn snapshot(&self) -> Self::Snapshot {
        self.clone()
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
    }
}

/// Reactive collection wrapper projecting each [`Tab`]'s `.tag(T)` as the collection's `Id`.
struct TaggedTabViews<V, C, L, T, S> {
    inner: V,
    _marker: PhantomData<fn(C, L, T) -> S>,
}

struct TaggedTabViewsSnapshot<Snap, C, L, T, S> {
    inner: Snap,
    _marker: PhantomData<fn(C, L, T) -> S>,
}

impl<Snap: Clone, C, L, T, S> Clone for TaggedTabViewsSnapshot<Snap, C, L, T, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<Snap, C, L, T, S> ViewSnapshot for TaggedTabViewsSnapshot<Snap, C, L, T, S>
where
    Snap: ViewSnapshot<View = Tab<C, L, TabTag<T>>>,
    C: ViewBuilder,
    L: LabelKind + Clone,
    T: Ord + Clone + 'static,
    S: ChildSlot,
{
    type Id = T;
    type View = SlotTabItem<S>;

    fn range(&self) -> Range<usize> {
        self.inner.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.inner.get_view(index).map(|tab| tab.tag.0)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.inner
            .get_view(index)
            .map(|tab| SlotTabItem(tab.into_item::<S>()))
    }
}

impl<V, C, L, T, S> Views for TaggedTabViews<V, C, L, T, S>
where
    V: Views<View = Tab<C, L, TabTag<T>>> + 'static,
    C: ViewBuilder,
    L: LabelKind + Clone,
    T: Ord + Clone + 'static,
    S: ChildSlot,
{
    type Id = T;
    type Guard = V::Guard;
    type View = SlotTabItem<S>;
    type Snapshot = TaggedTabViewsSnapshot<V::Snapshot, C, L, T, S>;

    fn len(&self) -> Computed<usize> {
        self.inner.len()
    }

    fn snapshot(&self) -> Self::Snapshot {
        TaggedTabViewsSnapshot {
            inner: self.inner.snapshot(),
            _marker: PhantomData,
        }
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        self.inner.watch(range, move |ctx, change| {
            watcher(
                ctx.map(|inner| TaggedTabViewsSnapshot {
                    inner,
                    _marker: PhantomData,
                }),
                change,
            );
        })
    }
}

/// Lowers fixed tagged tab entries through `AnyViews::new_with_ids` so `Mapping<T>` aligns
/// with every snapshot `Id`.
fn tagged_items<T, S>(items: Vec<(T, TabItem<S>)>) -> (AnyViews<TabItem<S>>, Mapping<T>)
where
    T: Ord + Clone + 'static,
    S: 'static,
{
    let (items, ids) = AnyViews::new_with_ids(TaggedTabItems {
        items: Rc::from(items),
    });
    (items.map_views(|SlotTabItem(item)| item), ids)
}

/// A homogeneous collection of tabs: erased by identity, each tab mapped
/// into the slot.
fn collection_items<T, S, C, L>(tabs: T) -> (AnyViews<TabItem<S>>, Mapping<T::Id>)
where
    T: Views<View = Tab<C, L, Untagged>> + 'static,
    S: ChildSlot,
    C: ViewBuilder,
    L: LabelKind + Clone,
{
    let (items, ids) = AnyViews::new_with_ids(tabs);
    (items.map_views(Tab::into_item::<S>), ids)
}

impl<C, L> TabSet for Vec<Tab<C, L, Untagged>>
where
    C: ViewBuilder,
    L: LabelKind + Clone,
{
    type Id = usize;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<usize>) {
        collection_items(self)
    }
}

impl<C, L, T> TabSet for Vec<Tab<C, L, TabTag<T>>>
where
    C: ViewBuilder,
    L: LabelKind + Clone,
    T: Ord + Clone + 'static,
{
    type Id = T;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<T>) {
        let pairs = self
            .into_iter()
            .map(|tab| {
                let (item, TabTag(tag)) = tab.into_parts_with_tag::<S>();
                (tag, item)
            })
            .collect();
        tagged_items(pairs)
    }
}

impl<C, L, const N: usize> TabSet for [Tab<C, L, Untagged>; N]
where
    C: ViewBuilder,
    L: LabelKind + Clone,
{
    type Id = usize;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<usize>) {
        collection_items(self)
    }
}

impl<C, L, T, const N: usize> TabSet for [Tab<C, L, TabTag<T>>; N]
where
    C: ViewBuilder,
    L: LabelKind + Clone,
    T: Ord + Clone + 'static,
{
    type Id = T;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<T>) {
        let pairs = self
            .into_iter()
            .map(|tab| {
                let (item, TabTag(tag)) = tab.into_parts_with_tag::<S>();
                (tag, item)
            })
            .collect();
        tagged_items(pairs)
    }
}

impl<D, F, C, L> TabSet for ForEach<D, F, Tab<C, L, Untagged>>
where
    Self: Views<View = Tab<C, L, Untagged>> + 'static,
    D: Collection,
    F: Fn(D::Item) -> Tab<C, L, Untagged>,
    C: ViewBuilder,
    L: LabelKind + Clone,
{
    type Id = <Self as Views>::Id;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<Self::Id>) {
        collection_items(self)
    }
}

impl<D, F, C, L, T> TabSet for ForEach<D, F, Tab<C, L, TabTag<T>>>
where
    Self: Views<View = Tab<C, L, TabTag<T>>> + 'static,
    D: Collection,
    F: Fn(D::Item) -> Tab<C, L, TabTag<T>>,
    C: ViewBuilder,
    L: LabelKind + Clone,
    T: Ord + Clone + 'static,
{
    type Id = T;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<T>) {
        let (items, ids) = AnyViews::new_with_ids(TaggedTabViews::<_, C, L, T, S> {
            inner: self,
            _marker: PhantomData,
        });
        (items.map_views(|SlotTabItem(item)| item), ids)
    }
}

macro_rules! tuple_tab_sets {
    ($(($($tab:ident $label:ident),+))+) => {$(
        impl<$($tab, $label),+> TabSet for ($(Tab<$tab, $label, Untagged>,)+)
        where
            $(
                $tab: ViewBuilder,
                $label: LabelKind + Clone,
            )+
        {
            type Id = usize;

            #[allow(non_snake_case, reason = "the bindings are the tuple's type parameters")]
            fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<usize>) {
                let ($($tab,)+) = self;
                AnyViews::from_items(vec![$($tab.into_item::<S>()),+])
            }
        }

        impl<T, $($tab, $label),+> TabSet for ($(Tab<$tab, $label, TabTag<T>>,)+)
        where
            T: Ord + Clone + 'static,
            $(
                $tab: ViewBuilder,
                $label: LabelKind + Clone,
            )+
        {
            type Id = T;

            #[allow(non_snake_case, reason = "the bindings are the tuple's type parameters")]
            fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<T>) {
                let ($($tab,)+) = self;
                let pairs = vec![$({
                    let (item, TabTag(tag)) = $tab.into_parts_with_tag::<S>();
                    (tag, item)
                }),+];
                tagged_items(pairs)
            }
        }
    )+};
}

tuple_tab_sets! {
    (A0 L0)
    (A0 L0, A1 L1)
    (A0 L0, A1 L1, A2 L2)
    (A0 L0, A1 L1, A2 L2, A3 L3)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4, A5 L5)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4, A5 L5, A6 L6)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4, A5 L5, A6 L6, A7 L7)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4, A5 L5, A6 L6, A7 L7, A8 L8)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4, A5 L5, A6 L6, A7 L7, A8 L8, A9 L9)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4, A5 L5, A6 L6, A7 L7, A8 L8, A9 L9, A10 L10)
    (A0 L0, A1 L1, A2 L2, A3 L3, A4 L4, A5 L5, A6 L6, A7 L7, A8 L8, A9 L9, A10 L10, A11 L11)
}

/// Adaptive native tab container keyed by the application's own tab type.
///
/// Selection is a caller-owned `Binding<T::Id>`; each [`Tab`] is identified
/// by its place in the [`TabSet`] — a `Vec`'s or tuple's index, a `ForEach`
/// item's `Id`. Identity is erased to [`Id`] only when projecting into
/// [`TabsLayout`] for the native backends. The set may be empty: the
/// container mounts with no tab content or bar items. `A` is the bottom
/// accessory's view type, `()` while there is none.
#[must_use]
pub struct Tabs<T: TabSet, A> {
    selection: Binding<T::Id>,
    set: T,
    style: TabStyle,
    minimize_behavior: TabBarMinimizeBehavior,
    bottom_accessory: Option<A>,
}

impl<T: TabSet, A> core::fmt::Debug for Tabs<T, A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tabs")
            .field("style", &self.style)
            .field("minimize_behavior", &self.minimize_behavior)
            .field("bottom_accessory", &self.bottom_accessory.is_some())
            .finish_non_exhaustive()
    }
}

impl<T: TabSet> Tabs<T, ()> {
    /// Creates a tab container driven by a caller-owned selection.
    ///
    /// `tabs` is a [`TabSet`]: a `Vec` for a fixed set, a tuple for tabs whose
    /// roots differ, `ForEach` over a reactive [`nami::collection::StateList`] for a set that
    /// changes at runtime. The set's id is the tab's identity — it is also
    /// the type `selection` binds.
    pub fn new(selection: &Binding<T::Id>, tabs: T) -> Self {
        Self {
            selection: selection.clone(),
            set: tabs,
            style: TabStyle::Automatic,
            minimize_behavior: TabBarMinimizeBehavior::Automatic,
            bottom_accessory: None,
        }
    }

    /// Sets a view the platform floats above the tab bar.
    ///
    /// The slot iOS 26 keeps for persistent content such as a now-playing
    /// mini-player: a glass bar that stays above the tab bar and collapses
    /// inline with it. The accessory is an arbitrary view; the platform gives
    /// it the capsule, placement and collapse behavior.
    ///
    /// This is an iOS primitive. macOS has no counterpart and does not show
    /// the accessory; Android ignores it. Content that must be visible on
    /// every platform belongs in the tab's own view tree.
    pub fn bottom_accessory<A: View>(self, accessory: A) -> Tabs<T, A> {
        Tabs {
            selection: self.selection,
            set: self.set,
            style: self.style,
            minimize_behavior: self.minimize_behavior,
            bottom_accessory: Some(accessory),
        }
    }
}

impl<T: TabSet, A> Tabs<T, A> {
    /// Sets native adaptive tab presentation.
    pub const fn style(mut self, style: TabStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets how the bar behaves while content scrolls.
    ///
    /// See [`TabBarMinimizeBehavior`]; platforms whose tab chrome does not
    /// collapse ignore it.
    pub const fn minimize_behavior(mut self, behavior: TabBarMinimizeBehavior) -> Self {
        self.minimize_behavior = behavior;
        self
    }
}

impl<T: TabSet, A: View> Tabs<T, A> {
    /// Erases tab identity into the container the native backends consume,
    /// with every tab's parts and the accessory in the child slot `S`.
    #[doc(hidden)]
    #[must_use]
    pub fn into_layout<S: ChildSlot>(self) -> TabsLayout<S> {
        let (tabs, ids) = self.set.into_items::<S>();
        TabsLayout {
            selection: ids.binding(&self.selection),
            tabs,
            style: self.style,
            minimize_behavior: self.minimize_behavior,
            bottom_accessory: self.bottom_accessory.map(S::from_view),
        }
    }
}

impl<T: TabSet, A: View> View for Tabs<T, A> {
    /// The erased form, for code that expands views by hand.
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_layout::<AnyView>())
    }

    /// Reaches the backend with every tab's parts and the accessory in the
    /// backend's child slot.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_layout::<B::Slot>(), env, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NavigationView;
    use alloc::vec;
    use alloc::vec::Vec;
    use nami::Signal as _;
    use nami::collection::StateList;
    use nami::id::SelfId;
    use waterui_core::Str;

    /// The label-only `Tab` fixture — content is irrelevant to id erasure.
    fn tab(label: &'static str) -> Tab<impl ViewBuilder<Output = ()>> {
        Tab::new(label, || ())
    }

    #[test]
    fn app_ids_erase_into_the_layout_collection() {
        let selection = Binding::new(1i32);
        let panes = [1, 2, 3].map(SelfId::new);
        let tabs = Tabs::new(
            &selection,
            ForEach::new(StateList::from(panes.to_vec()), |pane: SelfId<i32>| {
                tab(match *pane {
                    1 => "One",
                    2 => "Two",
                    _ => "Three",
                })
            }),
        );

        // Erasure projects every app id into the erased `Id` namespace and
        // keeps it consistent between the collection and the selection
        // binding a backend reads.
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(
            layout.selection.snapshot(),
            snapshot.get_id(0).expect("a non-empty set reports an id"),
        );
    }

    #[test]
    fn the_erased_selection_binding_maps_both_ways() {
        let selection = Binding::new(1i32);
        let panes = [1, 2].map(SelfId::new);
        let tabs = Tabs::new(
            &selection,
            ForEach::new(StateList::from(panes.to_vec()), |pane: SelfId<i32>| {
                tab(if *pane == 1 { "One" } else { "Two" })
            }),
        );
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();

        // An app write reaches the erased binding; an erased write — a
        // backend's tap — reaches the app binding.
        selection.set(2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(1).expect("id"),);
        layout.selection.set(snapshot.get_id(0).expect("id"));
        assert_eq!(selection.snapshot(), 1);
    }

    #[test]
    fn a_fixed_vec_keys_tabs_by_index() {
        let selection = Binding::new(0usize);
        let tabs = Tabs::new(&selection, vec![tab("One"), tab("Two")]);
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(0).expect("id"),);
    }

    /// Tabs whose roots differ form a tuple, keyed by position.
    #[test]
    fn a_tuple_keys_tabs_of_different_roots_by_index() {
        let selection = Binding::new(1usize);
        let tabs = Tabs::new(
            &selection,
            (
                Tab::new("Text", || Str::from("one")),
                Tab::new("Empty", || NavigationView::new("Empty", ())),
            ),
        )
        .bottom_accessory(Str::from("accessory"));
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(1).expect("id"));
        let page = snapshot.get_view(0).expect("a tab").content().build();
        assert!(page.downcast::<Str>().is_ok(), "the root keeps its content");
        assert!(layout.bottom_accessory.is_some());
    }

    #[test]
    fn an_empty_collection_is_a_valid_layout() {
        let selection = Binding::new(0usize);
        let tabs = Tabs::new(&selection, Vec::<Tab<fn() -> ()>>::new());
        let layout = tabs.into_layout::<AnyView>();
        assert_eq!(layout.tabs.snapshot().len(), 0);
    }

    #[test]
    fn tagged_vec_tabs_map_selection_both_ways() {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Pane {
            Inbox,
            Settings,
        }

        let pane = Binding::new(Pane::Inbox);
        let tabs = Tabs::new(
            &pane,
            vec![
                tab("Inbox").tag(Pane::Inbox),
                tab("Settings").tag(Pane::Settings),
            ],
        );
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(0).expect("id"));

        pane.set(Pane::Settings);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(1).expect("id"));

        layout.selection.set(snapshot.get_id(0).expect("id"));
        assert_eq!(pane.snapshot(), Pane::Inbox);
    }

    #[test]
    fn tagged_tuple_tabs_key_divergent_roots() {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Pane {
            Text,
            Nav,
        }

        let pane = Binding::new(Pane::Nav);
        let tabs = Tabs::new(
            &pane,
            (
                Tab::new("Text", || Str::from("one")).tag(Pane::Text),
                Tab::new("Empty", || NavigationView::new("Empty", ()))
                    .tag(Pane::Nav)
                    .badge(3u32),
            ),
        );
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(1).expect("id"));
        layout.selection.set(snapshot.get_id(0).expect("id"));
        assert_eq!(pane.snapshot(), Pane::Text);
    }

    #[test]
    fn tagged_array_and_foreach_tabs_map_selection() {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Pane {
            First,
            Second,
        }

        let pane = Binding::new(Pane::Second);
        let array_layout = Tabs::new(
            &pane,
            [
                tab("First").tag(Pane::First),
                tab("Second").tag(Pane::Second),
            ],
        )
        .into_layout::<AnyView>();
        let snapshot = array_layout.tabs.snapshot();
        assert_eq!(
            array_layout.selection.snapshot(),
            snapshot.get_id(1).expect("id")
        );

        let list = StateList::from(vec![SelfId::new(Pane::First), SelfId::new(Pane::Second)]);
        let foreach_layout = Tabs::new(
            &pane,
            ForEach::new(list, |entry: SelfId<Pane>| {
                let p = *entry;
                tab(match p {
                    Pane::First => "First",
                    Pane::Second => "Second",
                })
                .tag(p)
            }),
        )
        .into_layout::<AnyView>();
        let foreach_snap = foreach_layout.tabs.snapshot();
        assert_eq!(
            foreach_layout.selection.snapshot(),
            foreach_snap.get_id(1).expect("id")
        );
        foreach_layout
            .selection
            .set(foreach_snap.get_id(0).expect("id"));
        assert_eq!(pane.snapshot(), Pane::First);
    }
}
