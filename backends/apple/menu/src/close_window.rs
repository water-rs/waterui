//! The standard Close Window item as the kit's menu-tree payload.

use cocoa_ui::appkit::MenuAction;
use waterui_controls::menu::{CommandRole, Shortcut};

use crate::kit_command_fields;

/// The standard Close Window item's title — `AppKit`'s own word for
/// it, untranslated like the other standard items' titles.
const CLOSE_WINDOW_TITLE: &str = "Close";

/// The standard Close Window item as a kit [`Command`] payload.
///
/// It carries the title and the application's decided chord — ⌘W while the
/// application leaves it free, ⇧⌘W when a declared command holds ⌘W,
/// no key equivalent when it binds both.
///
/// [`Command`]: cocoa_ui::menu::Command
#[must_use]
pub fn standard_close_window_command(chord: Option<&Shortcut>) -> cocoa_ui::menu::Command {
    kit_command_fields(
        String::from(CLOSE_WINDOW_TITLE),
        None,
        None,
        CommandRole::Standard,
        chord,
    )
}

/// The standard Close Window item as a kit menu-tree node.
///
/// For the menus built from `MenuTreeNode`s (the declared menu bar, context
/// menus): the same title and chord, its action the same untargeted
/// `performClose:` sent up the responder chain — `AppKit` greys the
/// row out while no window can close.
#[must_use]
pub fn standard_close_window_node(chord: Option<&Shortcut>) -> cocoa_ui::menu::MenuTreeNode {
    cocoa_ui::menu::MenuTreeNode::Standard(
        standard_close_window_command(chord),
        MenuAction::CloseWindow,
    )
}
