//! Native adaptive tab navigation.

use alloc::vec::Vec;

use nami::{Binding, Computed, SignalExt as _};
use waterui_controls::IntoLabel;
use waterui_core::{
    AnyView, IntoSignal,
    handler::{AnyViewBuilder, ViewBuilder},
    id::Id,
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
}

/// Stable native tab container.
#[derive(Debug)]
#[non_exhaustive]
pub struct Tabs {
    /// Currently selected tab identifier.
    pub selection: Binding<Id>,
    /// Stable tabs.
    pub tabs: Vec<Tab<Id>>,
    /// Native adaptive style.
    pub style: NativeTabStyle,
}

impl Tabs {
    /// Creates a tab container.
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

raw_view!(Tabs, StretchAxis::Both);
