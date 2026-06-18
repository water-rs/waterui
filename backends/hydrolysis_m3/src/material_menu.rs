//! Material Design 3 menu convenience API.

use waterui::component::{IntoLabel, Label};
use waterui::widget::Divider;
use waterui_controls::menu::{Command, CommandBuilder, Menu, MenuItem, MenuView};

/// A Material Design 3 menu trigger and popup menu.
pub type MaterialMenu = Menu;

/// A Material Design 3 menu item.
pub type MaterialMenuItem = MenuItem;

/// A Material Design 3 menu command.
pub type MaterialMenuCommand = Command;

/// Builder for a Material Design 3 menu command.
pub type MaterialMenuItemBuilder = CommandBuilder;

/// Creates a Material Design 3 menu.
#[must_use]
pub fn material_menu(label: impl IntoLabel, items: impl MenuView) -> MaterialMenu {
    Menu::new(label, items)
}

/// Creates a Material Design 3 menu command builder.
#[must_use]
pub fn material_menu_item(label: impl IntoLabel) -> MaterialMenuItemBuilder {
    Command::builder(label)
}

/// Creates a Material Design 3 submenu.
#[must_use]
pub fn material_sub_menu(label: impl IntoLabel, items: impl MenuView) -> MaterialMenu {
    Menu::new(label, items)
}

/// Creates a Material Design 3 menu divider.
pub const fn material_menu_divider() -> Divider {
    Divider
}

/// Creates an icon-only Material Design 3 menu trigger label.
#[must_use]
pub fn material_menu_icon_label(label: impl Into<Label>) -> Label {
    label.into().icon_only()
}

#[cfg(test)]
mod tests {
    use crate::layout::menu::text_context_metrics;

    #[test]
    fn material_menu_tokens_match_material_web_v0_192() {
        let metrics = text_context_metrics();

        assert_eq!(metrics.row_height, 56.0);
        assert_eq!(metrics.min_width, 112.0);
        assert_eq!(metrics.max_width, 320.0);
        assert_eq!(metrics.corner_radius, 4.0);
        assert_eq!(metrics.separator_thickness, 1.0);
    }
}
