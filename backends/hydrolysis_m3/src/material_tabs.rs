//! Material Design 3 tabs composed from `WaterUI` navigation tabs.

use core::fmt::{self, Debug};

use waterui::navigation::NavigationView;
use waterui::navigation::tab::{Tab as WaterTab, TabsLayout as WaterTabsLayout, tab_style};
use waterui::{Binding, Environment, View};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{AnyViewBuilder, ViewBuilder};

/// A Material Design 3 tab item, named by the application's own tab type.
pub struct MaterialTab<T> {
    id: T,
    label: Label,
    content: AnyViewBuilder<NavigationView>,
}

impl<T: Debug> Debug for MaterialTab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterialTab")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<T> MaterialTab<T> {
    /// Creates a Material tab with a stable id, semantic label, and content.
    #[must_use]
    pub fn new(
        id: T,
        label: impl IntoLabel,
        content: impl ViewBuilder<Output = NavigationView>,
    ) -> Self {
        Self {
            id,
            label: label.into_label(),
            content: AnyViewBuilder::new(content),
        }
    }
}

/// A Material Design 3 primary tabs container.
pub struct MaterialTabs<T: 'static> {
    selection: Binding<T>,
    tabs: Vec<MaterialTab<T>>,
}

impl<T: Debug + 'static> Debug for MaterialTabs<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterialTabs")
            .field("tabs", &self.tabs)
            .finish_non_exhaustive()
    }
}

impl<T: Ord + Clone + 'static> MaterialTabs<T> {
    /// Creates an adaptive Material tabs container.
    #[must_use]
    pub fn new(selection: &Binding<T>, tabs: Vec<MaterialTab<T>>) -> Self {
        Self {
            selection: selection.clone(),
            tabs,
        }
    }
}

impl<T: Ord + Clone + 'static> View for MaterialTabs<T> {
    fn body(self, _env: &Environment) -> impl View {
        let selection = self.selection;
        let tabs = self
            .tabs
            .into_iter()
            .map(|tab| {
                let content = tab.content;
                WaterTab::new(tab.id, tab.label, move || content.build())
            })
            .collect();
        WaterTabsLayout::new(selection, tabs).style(tab_style::tab_bar())
    }
}

/// Creates a Material Design 3 primary tabs container.
#[must_use]
pub fn material_tabs<T: Ord + Clone + 'static>(
    selection: &Binding<T>,
    tabs: Vec<MaterialTab<T>>,
) -> MaterialTabs<T> {
    MaterialTabs::new(selection, tabs)
}

/// Creates a Material Design 3 tab.
#[must_use]
pub fn material_tab<T>(
    id: T,
    label: impl IntoLabel,
    content: impl ViewBuilder<Output = NavigationView>,
) -> MaterialTab<T> {
    MaterialTab::new(id, label, content)
}

#[cfg(test)]
mod tests {
    use crate::dimensions::{
        TABS_ACTIVE_INDICATOR_HEIGHT, TABS_ACTIVE_INDICATOR_RADIUS, TABS_BAR_HEIGHT,
        TABS_BUTTON_HORIZONTAL_INSET, TABS_BUTTON_MIN_WIDTH,
    };

    #[test]
    fn material_tabs_tokens_match_compose_primary_navigation_tab_tokens() {
        assert_eq!(TABS_BAR_HEIGHT, 48.0);
        assert_eq!(TABS_BUTTON_MIN_WIDTH, 48.0);
        assert_eq!(TABS_BUTTON_HORIZONTAL_INSET, 16.0);
        assert_eq!(TABS_ACTIVE_INDICATOR_HEIGHT, 3.0);
        assert_eq!(TABS_ACTIVE_INDICATOR_RADIUS, 3.0);
    }
}
