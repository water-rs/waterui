//! Material Design 3 tabs composed from `WaterUI` navigation tabs.

use core::fmt::{self, Debug};

use waterui::id::Id;
use waterui::navigation::NavigationView;
use waterui::navigation::tab::{Tab as WaterTab, TabsLayout as WaterTabsLayout, tab_style};
use waterui::{Binding, Environment, View};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{AnyViewBuilder, ViewBuilder};

/// A Material Design 3 tab item.
pub struct MaterialTab {
    id: Id,
    label: Label,
    content: AnyViewBuilder<NavigationView>,
}

impl Debug for MaterialTab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterialTab")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl MaterialTab {
    /// Creates a Material tab with a stable id, semantic label, and content.
    #[must_use]
    pub fn new(
        id: Id,
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
pub struct MaterialTabs {
    selection: Binding<Id>,
    tabs: Vec<MaterialTab>,
}

impl Debug for MaterialTabs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterialTabs")
            .field("tabs", &self.tabs)
            .finish_non_exhaustive()
    }
}

impl MaterialTabs {
    /// Creates an adaptive Material tabs container.
    #[must_use]
    pub fn new(selection: &Binding<Id>, tabs: Vec<MaterialTab>) -> Self {
        Self {
            selection: selection.clone(),
            tabs,
        }
    }
}

impl View for MaterialTabs {
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
pub fn material_tabs(selection: &Binding<Id>, tabs: Vec<MaterialTab>) -> MaterialTabs {
    MaterialTabs::new(selection, tabs)
}

/// Creates a Material Design 3 tab.
#[must_use]
pub fn material_tab(
    id: Id,
    label: impl IntoLabel,
    content: impl ViewBuilder<Output = NavigationView>,
) -> MaterialTab {
    MaterialTab::new(id, label, content)
}

#[cfg(test)]
mod tests {
    use crate::dimensions::{
        TABS_ACTIVE_INDICATOR_HEIGHT, TABS_ACTIVE_INDICATOR_RADIUS, TABS_BAR_HEIGHT,
        TABS_BUTTON_HORIZONTAL_INSET, TABS_BUTTON_MIN_WIDTH,
    };

    #[test]
    fn material_tabs_tokens_match_material_web_v0_192_primary_tabs() {
        assert_eq!(TABS_BAR_HEIGHT, 48.0);
        assert_eq!(TABS_BUTTON_MIN_WIDTH, 48.0);
        assert_eq!(TABS_BUTTON_HORIZONTAL_INSET, 16.0);
        assert_eq!(TABS_ACTIVE_INDICATOR_HEIGHT, 3.0);
        assert_eq!(TABS_ACTIVE_INDICATOR_RADIUS, 3.0);
    }
}
