//! Native adaptive tab navigation.

use alloc::vec::Vec;

use nami::{Binding, Computed, SignalExt as _};
use waterui_controls::IntoLabel;
use waterui_core::{
    AnyView, Environment, IntoSignal, Str, View,
    handler::{AnyViewBuilder, ViewBuilder},
    id::{Id, Mapping},
    impl_debug,
    layout::StretchAxis,
    raw_view,
};

use super::NavigationView;

/// Native tab presentation selected by a public style.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
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

/// One stable native tab.
pub struct Tab<T> {
    /// Stable tab identifier.
    pub id: T,
    /// Semantic tab label.
    pub label: AnyView,
    /// Stable lazily built tab root.
    pub content: AnyViewBuilder<NavigationView>,
    /// Optional reactive badge count.
    pub badge: Option<Computed<i32>>,
    /// Whether the native tab item is enabled.
    pub enabled: Computed<bool>,
}

impl_debug!(Tab<Id>);

impl<T> Tab<T> {
    /// Creates a stable tab with a semantic label and navigation root.
    pub fn new(
        id: T,
        label: impl IntoLabel,
        content: impl ViewBuilder<Output = NavigationView>,
    ) -> Self {
        Self {
            id,
            label: AnyView::new(label.into_label()),
            content: AnyViewBuilder::new(content),
            badge: None,
            enabled: Computed::constant(true),
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
    pub fn container<B>(id: T, label: impl IntoLabel, content: B) -> Self
    where
        B: ViewBuilder,
    {
        Self::new(id, label, move || {
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

    /// Rewrites the tab identity, keeping every other field.
    fn with_id<U>(self, id: U) -> Tab<U> {
        Tab {
            id,
            label: self.label,
            content: self.content,
            badge: self.badge,
            enabled: self.enabled,
        }
    }
}

/// Native tab container consumed by platform backends.
///
/// The C ABI cannot carry generics, so tab identity is erased to [`Id`] here.
/// Application code uses [`Tabs`], which keeps the app's own identifier type.
#[derive(Debug)]
#[non_exhaustive]
pub struct TabsLayout {
    /// Currently selected tab identifier.
    pub selection: Binding<Id>,
    /// Stable tabs.
    pub tabs: Vec<Tab<Id>>,
    /// Native adaptive style.
    pub style: NativeTabStyle,
}

impl TabsLayout {
    /// Creates an erased tab container.
    #[must_use]
    pub const fn new(selection: Binding<Id>, tabs: Vec<Tab<Id>>) -> Self {
        Self {
            selection,
            tabs,
            style: NativeTabStyle::Automatic,
        }
    }

    /// Sets native adaptive tab presentation.
    #[must_use]
    pub fn style(mut self, style: impl TabStyle) -> Self {
        self.style = style.into_native();
        self
    }
}

raw_view!(TabsLayout, StretchAxis::Both);

/// Adaptive native tab container keyed by the application's own tab type.
///
/// Selection is a caller-owned `Binding<T>`; each [`Tab`] carries a value of the
/// same type. Identity is erased to [`Id`] only when projecting into
/// [`TabsLayout`] for the native backends.
#[must_use]
pub struct Tabs<T: 'static> {
    selection: Binding<T>,
    items: Vec<Tab<T>>,
    style: NativeTabStyle,
}

impl<T> core::fmt::Debug for Tabs<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tabs")
            .field("tabs", &self.items.len())
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

impl<T: Ord + Clone + 'static> Tabs<T> {
    /// Creates a tab container driven by a caller-owned selection.
    pub fn new(selection: &Binding<T>, tabs: Vec<Tab<T>>) -> Self {
        Self {
            selection: selection.clone(),
            items: tabs,
            style: NativeTabStyle::Automatic,
        }
    }

    /// Sets native adaptive tab presentation.
    pub fn style(mut self, style: impl TabStyle) -> Self {
        self.style = style.into_native();
        self
    }
}

impl<T: Ord + Clone + 'static> View for Tabs<T> {
    fn body(self, _env: &Environment) -> impl View {
        let mapping = Mapping::new();
        let tabs = self
            .items
            .into_iter()
            .map(|tab| {
                let id = mapping.to_id(tab.id.clone());
                tab.with_id(id)
            })
            .collect();
        TabsLayout {
            selection: mapping.binding(&self.selection),
            tabs,
            style: self.style,
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}
