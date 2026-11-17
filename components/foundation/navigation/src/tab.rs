//! Native adaptive tab navigation.

use alloc::{rc::Rc, vec, vec::Vec};
use core::fmt::Debug;

use nami::collection::Collection;
use nami::{Binding, Computed, SignalExt as _};
use waterui_controls::{IntoLabel, label::Label};
use waterui_core::backend::MapSlots;
use waterui_core::{
    AnyView, Environment, IntoSignal, Native, NativeView, Str, View,
    backend::{Backend, ChildSlot},
    handler::{AnyViewBuilder, ViewBuilder},
    id::{Id, Mapping},
    layout::StretchAxis,
    views::{AnyViews, ForEach, Views},
};
use waterui_icon::SystemIcon;

use super::NavigationView;

/// Native tab presentation selected by a public style.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each tab style variant by variant; a new tab style must fail their build instead of falling into a wildcard arm"
)]
pub enum NativeTabStyle {
    /// Platform- and window-adaptive default.
    Automatic = 0,
    /// Always request the platform tab bar presentation.
    TabBar = 1,
    /// Always request the platform sidebar or navigation-rail presentation.
    Sidebar = 2,
}

/// Extensible public tab style contract.
pub trait TabStyle: 'static {
    /// Resolves the native presentation capability.
    #[doc(hidden)]
    fn into_native(self) -> NativeTabStyle;
}

/// Built-in tab styles.
pub mod tab_style {
    use super::{NativeTabStyle, TabStyle};

    /// Platform- and window-adaptive tab style.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Automatic;

    impl TabStyle for Automatic {
        fn into_native(self) -> NativeTabStyle {
            NativeTabStyle::Automatic
        }
    }

    /// Native tab-bar style.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct TabBar;

    impl TabStyle for TabBar {
        fn into_native(self) -> NativeTabStyle {
            NativeTabStyle::TabBar
        }
    }

    /// Native sidebar or navigation-rail style.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Sidebar;

    impl TabStyle for Sidebar {
        fn into_native(self) -> NativeTabStyle {
            NativeTabStyle::Sidebar
        }
    }

    /// Returns the platform-adaptive tab style.
    #[must_use]
    pub const fn automatic() -> Automatic {
        Automatic
    }

    /// Returns the native tab-bar style.
    #[must_use]
    pub const fn tab_bar() -> TabBar {
        TabBar
    }

    /// Returns the native sidebar or navigation-rail style.
    #[must_use]
    pub const fn sidebar() -> Sidebar {
        Sidebar
    }
}

/// A tab's icon, kept apart from its title.
///
/// A native tab bar item is an image next to a title rather than one composed
/// view, so the icon has to reach the backend on its own. The two forms are not
/// interchangeable: a platform that recognizes the symbol renders it at whatever
/// size and weight its chrome calls for, while a view has to be rasterized at a
/// chosen size.
#[derive(Clone)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each tab icon source variant by variant; a new tab icon source must fail their build instead of falling into a wildcard arm"
)]
pub enum TabIcon {
    /// A symbol the platform knows by name.
    System(SystemIcon),
    /// A view the backend renders itself.
    View(AnyViewBuilder<AnyView>),
}

impl Debug for TabIcon {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::System(icon) => f.debug_tuple("System").field(icon).finish(),
            Self::View(_) => f.write_str("View"),
        }
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

/// One stable native tab. The collection it lives in supplies its identity.
///
/// `C` builds the tab's root page. `Clone` shares the builder and the
/// signals — a `Vec<Tab<C>>` collection clones on snapshot, which is what
/// makes `vec![tab, ...]` a valid set of tabs sharing one builder type.
#[non_exhaustive]
pub struct Tab<C> {
    /// Semantic tab label.
    label: Label,
    /// The label's icon, lifted out so native chrome can place it itself.
    icon: Option<TabIcon>,
    /// Stable lazily built tab root.
    content: Rc<C>,
    /// Optional reactive badge count.
    badge: Option<Computed<i32>>,
    /// Whether the native tab item is enabled.
    enabled: Computed<bool>,
    /// The part the tab plays in the container's chrome.
    role: TabRole,
}

impl<C> Clone for Tab<C> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            icon: self.icon.clone(),
            content: Rc::clone(&self.content),
            badge: self.badge.clone(),
            enabled: self.enabled.clone(),
            role: self.role,
        }
    }
}

impl<C> Debug for Tab<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tab")
            .field("icon", &self.icon)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl<C, V> Tab<C>
where
    C: ViewBuilder<Output = NavigationView<V>>,
    V: View,
{
    /// Creates a stable tab with a semantic label and navigation root.
    ///
    /// The tab's identity comes from the collection handed to [`Tabs`]: a
    /// tab in a `Vec` is keyed by its index, one built through `ForEach` by
    /// the item it was built from.
    pub fn new(label: impl IntoLabel, content: C) -> Self {
        let label = label.into_label();
        // Lift the icon out: a tab bar item wants it separately, and once the
        // label is drawn nothing can tell which part of it was the icon.
        let icon = label.semantic_icon().map_or_else(
            || label.icon_view().map(TabIcon::View),
            |icon| Some(TabIcon::System(icon)),
        );
        Self {
            label,
            icon,
            content: Rc::new(content),
            badge: None,
            enabled: Computed::constant(true),
            role: TabRole::Regular,
        }
    }

    /// The tab with its parts built in the child slot `S`.
    fn into_item<S: ChildSlot>(self) -> TabItem<S> {
        let Self {
            label,
            icon,
            content,
            badge,
            enabled,
            role,
        } = self;
        TabItem {
            label: SlotBuilder::new(move || S::from_view(label.clone())),
            icon,
            content: SlotBuilder::new(move || content.build().map_content(S::from_view)),
            badge,
            enabled,
            role,
        }
    }
}

/// The builder [`Tab::container`] wraps a container's root in: a page with
/// no bar of its own.
#[derive(Debug)]
pub struct ContainerPage<B>(B);

impl<B: ViewBuilder> ViewBuilder for ContainerPage<B> {
    type Output = NavigationView<B::Output>;

    fn build(&self) -> Self::Output {
        NavigationView::new(Str::default(), self.0.build()).navigation_bar_visibility(false)
    }
}

impl<B: ViewBuilder> Tab<ContainerPage<B>> {
    /// Creates a tab whose root supplies its own navigation chrome.
    ///
    /// Each tab owns one navigation container — a [`NavigationStack`] or a
    /// [`NavigationSplitView`] — so pushing inside a tab leaves the other tabs'
    /// navigation state untouched, and returning to a tab shows it exactly
    /// where the user left it. The tab page itself contributes no bar; the
    /// container inside it does.
    ///
    /// [`NavigationStack`]: crate::NavigationStack
    /// [`NavigationSplitView`]: crate::NavigationSplitView
    pub fn container(label: impl IntoLabel, content: B) -> Self {
        Self::new(label, ContainerPage(content))
    }
}

impl<C> Tab<C> {
    /// Semantic tab label.
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// The label's icon, lifted out so native chrome can place it itself.
    #[must_use]
    pub const fn icon(&self) -> Option<&TabIcon> {
        self.icon.as_ref()
    }

    /// Stable lazily built tab root.
    #[must_use]
    pub fn content(&self) -> &C {
        &self.content
    }

    /// Optional reactive badge count.
    #[must_use]
    pub const fn get_badge(&self) -> Option<&Computed<i32>> {
        self.badge.as_ref()
    }

    /// Whether the native tab item is enabled.
    #[must_use]
    pub const fn get_enabled(&self) -> &Computed<bool> {
        &self.enabled
    }

    /// The part the tab plays in the container's chrome.
    #[must_use]
    pub const fn get_role(&self) -> TabRole {
        self.role
    }

    /// Sets a reactive badge count.
    #[must_use]
    pub fn badge(mut self, count: impl IntoSignal<i32> + 'static) -> Self {
        self.badge = Some(count.into_signal().computed());
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
impl<C: 'static> View for Tab<C> {
    fn body(self, _env: &Environment) -> impl View {
        self.label
    }
}

/// Builds one part of a tab — its label or its root page — each time its
/// backend mounts it.
pub struct SlotBuilder<T>(Rc<dyn Fn() -> T>);

impl<T> SlotBuilder<T> {
    fn new(build: impl Fn() -> T + 'static) -> Self {
        Self(Rc::new(build))
    }

    /// Builds the part.
    #[must_use]
    pub fn build(&self) -> T {
        (self.0)()
    }
}

impl<T> Clone for SlotBuilder<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> Debug for SlotBuilder<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SlotBuilder")
    }
}

/// A tab as its backend receives it: its label and root page built in the
/// backend's child slot `S`.
#[non_exhaustive]
pub struct TabItem<S> {
    /// Semantic tab label.
    label: SlotBuilder<S>,
    /// The label's icon, lifted out so native chrome can place it itself.
    icon: Option<TabIcon>,
    /// Stable lazily built tab root.
    content: SlotBuilder<NavigationView<S>>,
    /// Optional reactive badge count.
    badge: Option<Computed<i32>>,
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
    pub fn map_slot<T>(self, f: impl Fn(S) -> T + Clone + 'static) -> TabItem<T> {
        let Self {
            label,
            icon,
            content,
            badge,
            enabled,
            role,
        } = self;
        let content_f = f.clone();
        TabItem {
            label: SlotBuilder::new(move || f(label.build())),
            icon,
            content: SlotBuilder::new(move || content.build().map_content(content_f.clone())),
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

    /// The label's icon, lifted out so native chrome can place it itself.
    #[must_use]
    pub const fn icon(&self) -> Option<&TabIcon> {
        self.icon.as_ref()
    }

    /// Stable lazily built tab root.
    #[must_use]
    pub const fn content(&self) -> &SlotBuilder<NavigationView<S>> {
        &self.content
    }

    /// Optional reactive badge count.
    #[must_use]
    pub const fn get_badge(&self) -> Option<&Computed<i32>> {
        self.badge.as_ref()
    }

    /// Whether the native tab item is enabled.
    #[must_use]
    pub const fn get_enabled(&self) -> &Computed<bool> {
        &self.enabled
    }

    /// The part the tab plays in the container's chrome.
    #[must_use]
    pub const fn get_role(&self) -> TabRole {
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
        Option<TabIcon>,
        SlotBuilder<NavigationView<S>>,
        Option<Computed<i32>>,
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
    style: NativeTabStyle,
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
    pub const fn get_style(&self) -> NativeTabStyle {
        self.style
    }

    /// How the bar behaves while content scrolls.
    #[must_use]
    pub const fn get_minimize_behavior(&self) -> TabBarMinimizeBehavior {
        self.minimize_behavior
    }

    /// A view the platform floats above the tab bar, when it has such a slot.
    #[must_use]
    pub const fn get_bottom_accessory(&self) -> Option<&S> {
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
        NativeTabStyle,
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

/// A homogeneous collection of tabs: erased by identity, each tab mapped
/// into the slot.
fn collection_items<T, S, C, V>(tabs: T) -> (AnyViews<TabItem<S>>, Mapping<T::Id>)
where
    T: Views<View = Tab<C>> + 'static,
    S: ChildSlot,
    C: ViewBuilder<Output = NavigationView<V>>,
    V: View,
{
    let (items, ids) = AnyViews::new_with_ids(tabs);
    (items.map_views(Tab::into_item::<S>), ids)
}

impl<C, V> TabSet for Vec<Tab<C>>
where
    C: ViewBuilder<Output = NavigationView<V>>,
    V: View,
{
    type Id = usize;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<usize>) {
        collection_items(self)
    }
}

impl<C, V, const N: usize> TabSet for [Tab<C>; N]
where
    C: ViewBuilder<Output = NavigationView<V>>,
    V: View,
{
    type Id = usize;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<usize>) {
        collection_items(self)
    }
}

impl<D, F, C, V> TabSet for ForEach<D, F, Tab<C>>
where
    Self: Views<View = Tab<C>> + 'static,
    D: Collection,
    F: Fn(D::Item) -> Tab<C>,
    C: ViewBuilder<Output = NavigationView<V>>,
    V: View,
{
    type Id = <Self as Views>::Id;

    fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<Self::Id>) {
        collection_items(self)
    }
}

macro_rules! tuple_tab_sets {
    ($(($($tab:ident $view:ident),+))+) => {$(
        impl<$($tab, $view),+> TabSet for ($(Tab<$tab>,)+)
        where
            $($tab: ViewBuilder<Output = NavigationView<$view>>, $view: View,)+
        {
            type Id = usize;

            #[allow(non_snake_case, reason = "the bindings are the tuple's type parameters")]
            fn into_items<S: ChildSlot>(self) -> (AnyViews<TabItem<S>>, Mapping<usize>) {
                let ($($tab,)+) = self;
                AnyViews::from_items(vec![$($tab.into_item::<S>()),+])
            }
        }
    )+};
}

tuple_tab_sets! {
    (A0 V0)
    (A0 V0, A1 V1)
    (A0 V0, A1 V1, A2 V2)
    (A0 V0, A1 V1, A2 V2, A3 V3)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4, A5 V5)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4, A5 V5, A6 V6)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4, A5 V5, A6 V6, A7 V7)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4, A5 V5, A6 V6, A7 V7, A8 V8)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4, A5 V5, A6 V6, A7 V7, A8 V8, A9 V9)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4, A5 V5, A6 V6, A7 V7, A8 V8, A9 V9, A10 V10)
    (A0 V0, A1 V1, A2 V2, A3 V3, A4 V4, A5 V5, A6 V6, A7 V7, A8 V8, A9 V9, A10 V10, A11 V11)
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
    style: NativeTabStyle,
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
    /// roots differ, `ForEach` over a reactive [`nami::List`] for a set that
    /// changes at runtime. The set's id is the tab's identity — it is also
    /// the type `selection` binds.
    pub fn new(selection: &Binding<T::Id>, tabs: T) -> Self {
        Self {
            selection: selection.clone(),
            set: tabs,
            style: NativeTabStyle::Automatic,
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
    pub fn style(mut self, style: impl TabStyle) -> Self {
        self.style = style.into_native();
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
    use alloc::vec;
    use alloc::vec::Vec;
    use nami::Signal as _;
    use nami::collection::List as SignalList;
    use nami::id::SelfId;

    /// The label-only `Tab` fixture — content is irrelevant to id erasure.
    fn tab(label: &'static str) -> Tab<impl ViewBuilder<Output = NavigationView<()>>> {
        Tab::new(label, move || NavigationView::new(label, ()))
    }

    #[test]
    fn app_ids_erase_into_the_layout_collection() {
        let selection = Binding::container(1i32);
        let panes = [1, 2, 3].map(SelfId::new);
        let tabs = Tabs::new(
            &selection,
            ForEach::new(SignalList::from(panes.to_vec()), |pane: SelfId<i32>| {
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
        let selection = Binding::container(1i32);
        let panes = [1, 2].map(SelfId::new);
        let tabs = Tabs::new(
            &selection,
            ForEach::new(SignalList::from(panes.to_vec()), |pane: SelfId<i32>| {
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
        let selection = Binding::container(0usize);
        let tabs = Tabs::new(&selection, vec![tab("One"), tab("Two")]);
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(0).expect("id"),);
    }

    /// Tabs whose roots differ form a tuple, keyed by position.
    #[test]
    fn a_tuple_keys_tabs_of_different_roots_by_index() {
        let selection = Binding::container(1usize);
        let tabs = Tabs::new(
            &selection,
            (
                Tab::new("Text", || NavigationView::new("Text", Str::from("one"))),
                Tab::new("Empty", || NavigationView::new("Empty", ())),
            ),
        )
        .bottom_accessory(Str::from("accessory"));
        let layout = tabs.into_layout::<AnyView>();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(1).expect("id"));
        let page = snapshot.get_view(0).expect("a tab").content().build();
        assert!(
            page.content.downcast::<Str>().is_ok(),
            "the root keeps its content"
        );
        assert!(layout.bottom_accessory.is_some());
    }

    #[test]
    fn an_empty_collection_is_a_valid_layout() {
        let selection = Binding::container(0usize);
        let tabs = Tabs::new(&selection, Vec::<Tab<fn() -> NavigationView<()>>>::new());
        let layout = tabs.into_layout::<AnyView>();
        assert_eq!(layout.tabs.snapshot().len(), 0);
    }
}
