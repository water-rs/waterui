//! Material Design 3 tabs composed from `WaterUI` navigation tabs.

use core::fmt::{self, Debug};

use waterui::id::{Id, TaggedView};
use waterui::navigation::NavigationView;
use waterui::navigation::tab::{Tab as WaterTab, TabPosition, Tabs as WaterTabs};
use waterui::reactive::SignalExt as _;
use waterui::text::Text;
use waterui::{AnyView, Binding, Environment, View};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::Native;
use waterui_core::handler::{AnyViewBuilder, ViewBuilder};

use crate::color::{OnSurfaceVariant, Primary};
use crate::semantics::label_plain_text;
use crate::theme::typography;

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
    position: TabPosition,
}

impl Debug for MaterialTabs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterialTabs")
            .field("tabs", &self.tabs)
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

impl MaterialTabs {
    /// Creates a top-positioned Material tabs container.
    #[must_use]
    pub fn new(selection: &Binding<Id>, tabs: Vec<MaterialTab>) -> Self {
        Self {
            selection: selection.clone(),
            tabs,
            position: TabPosition::Top,
        }
    }

    /// Places the tab bar at the bottom edge.
    #[must_use]
    pub const fn bottom(mut self) -> Self {
        self.position = TabPosition::Bottom;
        self
    }
}

impl View for MaterialTabs {
    fn body(self, _env: &Environment) -> impl View {
        let selection = self.selection;
        let tabs = self
            .tabs
            .into_iter()
            .map(|tab| {
                let selected = selection.clone().map(move |selection| -> waterui::Color {
                    if selection == tab.id {
                        Primary.into()
                    } else {
                        OnSurfaceVariant.into()
                    }
                });
                let content = tab.content;
                let label = Text::verbatim(label_plain_text(&tab.label))
                    .font(typography::title_small())
                    .color(selected)
                    .into_config_without_env();
                WaterTab::new(
                    TaggedView::new(tab.id, AnyView::new(Native::new(label))),
                    move || content.build(),
                )
            })
            .collect();
        WaterTabs::new(selection, tabs).position(self.position)
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
