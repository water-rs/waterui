//! Native adaptive tab navigation.

use core::fmt::Debug;

use nami::{Binding, Computed, Signal as _, SignalExt as _};
use waterui_controls::IntoLabel;
use waterui_core::{
    AnyView, Environment, IntoSignal, NativeView, Str, View,
    handler::{AnyViewBuilder, ViewBuilder},
    id::{Id, Mapping},
    impl_debug,
    layout::StretchAxis,
    raw_view,
    views::{AnyViews, SharedAnyViews, Views},
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
/// `Clone` shares a tab's lazily built content and signals — a `Vec<Tab>`
/// collection clones on snapshot, which is what makes `vec![tab, ...]` a
/// valid [`Views`] set for a fixed tab bar.
#[derive(Clone)]
#[non_exhaustive]
pub struct Tab {
    /// Semantic tab label, kept as a builder so a tab in a `Vec` stays
    /// cloneable — the collection materializes one label per mount.
    label: AnyViewBuilder<AnyView>,
    /// The label's icon, lifted out so native chrome can place it itself.
    icon: Option<TabIcon>,
    /// Stable lazily built tab root.
    content: AnyViewBuilder<NavigationView>,
    /// Optional reactive badge count.
    badge: Option<Computed<i32>>,
    /// Whether the native tab item is enabled.
    enabled: Computed<bool>,
    /// The part the tab plays in the container's chrome.
    role: TabRole,
}

impl Tab {
    /// Semantic tab label, kept as a builder so a tab in a `Vec` stays
    /// cloneable — the collection materializes one label per mount.
    #[must_use]
    pub const fn label(&self) -> &AnyViewBuilder<AnyView> {
        &self.label
    }

    /// The label's icon, lifted out so native chrome can place it itself.
    #[must_use]
    pub const fn icon(&self) -> Option<&TabIcon> {
        self.icon.as_ref()
    }

    /// Stable lazily built tab root.
    #[must_use]
    pub const fn content(&self) -> &AnyViewBuilder<NavigationView> {
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
        AnyViewBuilder<AnyView>,
        Option<TabIcon>,
        AnyViewBuilder<NavigationView>,
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

impl_debug!(Tab);

impl Tab {
    /// Creates a stable tab with a semantic label and navigation root.
    ///
    /// The tab's identity comes from the collection handed to [`Tabs`]: a
    /// tab in a `Vec` is keyed by its index, one built through `ForEach` by
    /// the item it was built from.
    pub fn new(label: impl IntoLabel, content: impl ViewBuilder<Output = NavigationView>) -> Self {
        let label = label.into_label();
        // Lift the icon out before the label is erased: a tab bar item wants it
        // separately, and once this is an `AnyView` nothing can tell which part
        // of it was the icon.
        let icon = label.semantic_icon().map_or_else(
            || label.icon_view().map(TabIcon::View),
            |icon| Some(TabIcon::System(icon)),
        );
        Self {
            label: AnyViewBuilder::new(move || AnyView::new(label.clone())),
            icon,
            content: AnyViewBuilder::new(content),
            badge: None,
            enabled: Computed::constant(true),
            role: TabRole::Regular,
        }
    }

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
    pub fn container<B>(label: impl IntoLabel, content: B) -> Self
    where
        B: ViewBuilder,
    {
        Self::new(label, move || {
            NavigationView::new(Str::default(), content.build()).navigation_bar_visibility(false)
        })
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

impl NativeView for Tab {}

/// A bare tab shows its label — outside a [`Tabs`] container there is no
/// chrome to hang the content on.
impl View for Tab {
    fn body(self, _env: &Environment) -> impl View {
        self.label.build()
    }
}

/// Native tab container consumed by platform backends.
///
/// The C ABI cannot carry generics, so tab identity is erased to [`Id`] here.
/// Application code uses [`Tabs`], which keeps the app's own identifier type.
/// `tabs` is a reactive collection: a backend mounts its snapshot once and
/// then watches it, applying inserts, removals and moves by tab id so a tab
/// that survives a change keeps its retained state.
#[derive(Debug)]
#[non_exhaustive]
pub struct TabsLayout {
    /// Currently selected tab identifier.
    selection: Binding<Id>,
    /// The tab collection, keyed by erased tab identity.
    tabs: SharedAnyViews<Tab>,
    /// Native adaptive style.
    style: NativeTabStyle,
    /// How the bar behaves while content scrolls.
    minimize_behavior: TabBarMinimizeBehavior,
    /// A view the platform floats above the tab bar, when it has such a slot.
    bottom_accessory: Option<AnyView>,
}

impl TabsLayout {
    /// Currently selected tab identifier.
    #[must_use]
    pub const fn selection(&self) -> &Binding<Id> {
        &self.selection
    }

    /// The tab collection, keyed by erased tab identity.
    #[must_use]
    pub const fn tabs(&self) -> &SharedAnyViews<Tab> {
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
    pub const fn get_bottom_accessory(&self) -> Option<&AnyView> {
        self.bottom_accessory.as_ref()
    }

    /// Takes the `bottom_accessory` out of the value, leaving none behind.
    pub const fn take_bottom_accessory(&mut self) -> Option<AnyView> {
        self.bottom_accessory.take()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        Binding<Id>,
        SharedAnyViews<Tab>,
        NativeTabStyle,
        TabBarMinimizeBehavior,
        Option<AnyView>,
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

impl TabsLayout {
    /// Creates an erased tab container.
    #[must_use]
    pub const fn new(selection: Binding<Id>, tabs: SharedAnyViews<Tab>) -> Self {
        Self {
            selection,
            tabs,
            style: NativeTabStyle::Automatic,
            minimize_behavior: TabBarMinimizeBehavior::Automatic,
            bottom_accessory: None,
        }
    }

    /// Sets native adaptive tab presentation.
    #[must_use]
    pub fn style(mut self, style: impl TabStyle) -> Self {
        self.style = style.into_native();
        self
    }

    /// Sets how the bar behaves while content scrolls.
    #[must_use]
    pub const fn minimize_behavior(mut self, behavior: TabBarMinimizeBehavior) -> Self {
        self.minimize_behavior = behavior;
        self
    }

    /// Sets the view the platform floats above the tab bar.
    #[must_use]
    pub fn bottom_accessory(mut self, accessory: impl View) -> Self {
        self.bottom_accessory = Some(AnyView::new(accessory));
        self
    }
}

raw_view!(TabsLayout, StretchAxis::Both);

/// Adaptive native tab container keyed by the application's own tab type.
///
/// Selection is a caller-owned `Binding<T>`; each [`Tab`] is identified by the
/// id of the element of `tabs` it was built from — a `Vec`'s index, a
/// `ForEach` item's `Id`. Identity is erased to [`Id`] only when projecting
/// into [`TabsLayout`] for the native backends. The collection may be empty:
/// the container mounts with no tab content or bar items.
#[must_use]
pub struct Tabs<T: 'static> {
    selection: Binding<T>,
    items: SharedAnyViews<Tab>,
    ids: Mapping<T>,
    style: NativeTabStyle,
    minimize_behavior: TabBarMinimizeBehavior,
    bottom_accessory: Option<AnyView>,
}

impl<T> core::fmt::Debug for Tabs<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tabs")
            .field("tabs", &self.items.len().snapshot())
            .field("style", &self.style)
            .field("minimize_behavior", &self.minimize_behavior)
            .field("bottom_accessory", &self.bottom_accessory.is_some())
            .finish_non_exhaustive()
    }
}

impl<T: Ord + Clone + 'static> Tabs<T> {
    /// Creates a tab container driven by a caller-owned selection.
    ///
    /// `tabs` is any [`Views`] collection whose elements are [`Tab`]s: a `Vec`
    /// for a fixed set, `ForEach` over a reactive [`nami::List`] for a set
    /// that changes at runtime. The collection's element id is the tab's
    /// identity — it is also the type `selection` binds.
    pub fn new(selection: &Binding<T>, tabs: impl Views<View = Tab, Id = T> + 'static) -> Self {
        let (items, ids) = AnyViews::new_with_ids(tabs);
        Self {
            selection: selection.clone(),
            items: SharedAnyViews::from(items),
            ids,
            style: NativeTabStyle::Automatic,
            minimize_behavior: TabBarMinimizeBehavior::Automatic,
            bottom_accessory: None,
        }
    }

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
    pub fn bottom_accessory(mut self, accessory: impl View) -> Self {
        self.bottom_accessory = Some(AnyView::new(accessory));
        self
    }

    /// Erases tab identity into the container the native backends consume.
    #[doc(hidden)]
    #[must_use]
    pub fn into_layout(self) -> TabsLayout {
        TabsLayout {
            selection: self.ids.binding(&self.selection),
            tabs: self.items,
            style: self.style,
            minimize_behavior: self.minimize_behavior,
            bottom_accessory: self.bottom_accessory,
        }
    }
}

impl<T: Ord + Clone + 'static> View for Tabs<T> {
    fn body(self, _env: &Environment) -> impl View {
        self.into_layout()
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use nami::collection::List as SignalList;
    use nami::id::SelfId;
    use waterui_core::views::{ForEach, ViewSnapshot as _};

    /// The label-only `Tab` fixture — content is irrelevant to id erasure.
    fn tab(label: &'static str) -> Tab {
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
        let layout = tabs.into_layout();
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
        let layout = tabs.into_layout();
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
        let layout = tabs.into_layout();
        let snapshot = layout.tabs.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(layout.selection.snapshot(), snapshot.get_id(0).expect("id"),);
    }

    #[test]
    fn an_empty_collection_is_a_valid_layout() {
        let selection = Binding::container(0usize);
        let tabs = Tabs::new(&selection, Vec::<Tab>::new());
        let layout = tabs.into_layout();
        assert_eq!(layout.tabs.snapshot().len(), 0);
    }
}
