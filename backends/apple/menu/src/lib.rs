//! The projection of `WaterUI` menus onto the native menu tree.
//!
//! Both Apple-platform hosts — the native `waterui-apple` backend and the
//! Hydrolysis `UIKit` host — turn a declared `App::menu_bar` into
//! `cocoa_ui::menu::MenuTreeNode`s the same way: [`menu_tree`] binds each
//! command's action to the application environment, and [`kit_command_fields`]
//! carries its label, symbol and shortcut. On iOS the [`ios`] module builds
//! the main menu from that tree through the main menu system's build
//! handler, so a shortcut becomes a `UIKeyCommand` `UIKit` matches before
//! the key reaches a responder.

#![cfg(any(target_os = "macos", target_os = "ios"))]

#[cfg(target_os = "macos")]
mod close_window;
#[cfg(target_os = "ios")]
pub mod ios;
mod key;

use std::rc::Rc;

use nami::Signal;
#[cfg(target_os = "macos")]
use waterui_controls::menu::CloseWindowChord;
use waterui_controls::menu::{
    CommandRole, ResolvedCommand, ResolvedMenuItem, ResolvedNestedMenu, Shortcut,
};
use waterui_core::Environment;

#[cfg(target_os = "macos")]
pub use close_window::{standard_close_window_command, standard_close_window_node};
pub use key::key_equivalent_for;

/// The key equivalent and modifier set a `Shortcut` describes.
#[must_use]
pub fn shortcut_parts(shortcut: &Shortcut) -> (String, cocoa_ui::menu::KeyModifiers) {
    use cocoa_ui::menu::KeyModifiers;
    let modifiers = [
        (shortcut.modifiers.command(), KeyModifiers::COMMAND),
        (shortcut.modifiers.option(), KeyModifiers::OPTION),
        (shortcut.modifiers.shift(), KeyModifiers::SHIFT),
        (shortcut.modifiers.control(), KeyModifiers::CONTROL),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(KeyModifiers::empty(), |flags, (_, native)| flags | native);
    (key_equivalent_for(&shortcut.key), modifiers)
}

/// A resolved command as the kit's shared `Command` payload — label text,
/// subtitle, symbol name, destructive flag and shortcut.
fn kit_command(command: &ResolvedCommand) -> cocoa_ui::menu::Command {
    kit_command_fields(
        command.label.content.snapshot().to_plain().to_string(),
        command.subtitle.as_ref().map(ToString::to_string),
        command.icon.as_ref().map(|icon| icon.name.to_string()),
        command.role,
        command.shortcut.as_ref(),
    )
}

/// A resolved nested menu's header as the kit's shared `Command` payload.
#[must_use]
pub fn kit_command_for_menu(menu: &ResolvedNestedMenu) -> cocoa_ui::menu::Command {
    kit_command_fields(
        menu.label.content.snapshot().to_plain().to_string(),
        None,
        menu.icon.as_ref().map(|icon| icon.name.to_string()),
        CommandRole::Standard,
        None,
    )
}

/// The kit `Command` payload of one item's presentation fields.
#[must_use]
pub fn kit_command_fields(
    label: String,
    subtitle: Option<String>,
    symbol: Option<String>,
    role: CommandRole,
    shortcut: Option<&Shortcut>,
) -> cocoa_ui::menu::Command {
    let (key_equivalent, modifiers) = shortcut.map_or_else(
        || (String::new(), cocoa_ui::menu::KeyModifiers::empty()),
        shortcut_parts,
    );
    cocoa_ui::menu::Command {
        label,
        subtitle,
        symbol,
        destructive: matches!(role, CommandRole::Destructive),
        enabled: true,
        selected: false,
        key_equivalent,
        modifiers,
    }
}

/// The declared menu bar's resolved items as the kit's shared tree.
///
/// It is a `MenuTreeNode` list, with each command's action bound to fire under
/// `env` — the content both the macOS and the iOS menu bar build from.
/// On macOS each command is checked against the chords the standard
/// application menu reserves.
#[must_use]
pub fn menu_tree(
    items: &[ResolvedMenuItem],
    env: &Environment,
) -> Vec<cocoa_ui::menu::MenuTreeNode> {
    items
        .iter()
        .filter_map(|item| match item {
            ResolvedMenuItem::Divider => Some(cocoa_ui::menu::MenuTreeNode::Divider),
            ResolvedMenuItem::Command(command) => {
                #[cfg(target_os = "macos")]
                command.assert_allowed_in_macos_menu_bar();
                let action = command.action.clone();
                let env = env.clone();
                Some(cocoa_ui::menu::MenuTreeNode::Command(
                    kit_command(command),
                    Rc::new(move || {
                        action.call(&env);
                    }),
                ))
            }
            ResolvedMenuItem::Menu(menu) => Some(cocoa_ui::menu::MenuTreeNode::Submenu(
                kit_command_for_menu(menu),
                menu_tree(&menu.items.snapshot(), env),
            )),
            // macOS: the standard application menu already ends with Quit
            // (`build_default`), so a declared one is dropped rather than
            // shown twice. iOS has no application quit, so it is omitted.
            ResolvedMenuItem::Quit => None,
            // macOS: the standard Close item where it was declared, with the
            // application's decided chord — the Window menu then leaves its
            // own out (`CloseWindowPlacement`).
            #[cfg(target_os = "macos")]
            ResolvedMenuItem::CloseWindow => Some(standard_close_window_node(
                CloseWindowChord::of(env).as_ref(),
            )),
            // iOS: the system owns every scene's window, so a declared Close
            // Window is omitted.
            #[cfg(not(target_os = "macos"))]
            ResolvedMenuItem::CloseWindow => None,
        })
        .collect()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use cocoa_ui::appkit::{KeyModifiers, MenuAction};
    use cocoa_ui::menu::MenuTreeNode;
    use cocoa_ui::objc2_app_kit::{NSDeleteFunctionKey, NSF5FunctionKey};
    use nami::{Computed, Signal};
    use waterui_controls::menu::{
        CloseWindowChord, Menu, MenuItem, NamedKey, ResolvedMenuItem, ShortcutKey,
        resolve_menu_bar_items,
    };
    use waterui_core::Environment;

    use super::{key_equivalent_for, menu_tree};

    fn function_key(code: u32) -> String {
        String::from(char::from_u32(code).expect("function keys are scalar values"))
    }

    #[test]
    fn named_keys_map_to_appkit_function_key_equivalents() {
        assert_eq!(
            key_equivalent_for(&ShortcutKey::from(NamedKey::Delete)),
            function_key(NSDeleteFunctionKey)
        );
        assert_eq!(
            key_equivalent_for(&ShortcutKey::from(NamedKey::F5)),
            function_key(NSF5FunctionKey)
        );
        assert_eq!(key_equivalent_for(&ShortcutKey::from('q')), "q");
    }

    #[test]
    fn an_uppercase_character_key_equivalent_is_lowercased() {
        assert_eq!(key_equivalent_for(&ShortcutKey::from('Q')), "q");
    }

    #[test]
    #[should_panic(expected = "has no AppKit key equivalent")]
    fn a_named_key_without_an_appkit_equivalent_panics() {
        let _ = key_equivalent_for(&ShortcutKey::from(NamedKey::AudioVolumeUp));
    }

    /// `menus` resolved as the declared menu bar.
    fn declared(menus: Vec<Menu>) -> Vec<ResolvedMenuItem> {
        resolve_menu_bar_items(&Computed::constant(menus), &Environment::new()).snapshot()
    }

    /// A bare environment with the application's decided chord installed,
    /// as `App::into_parts` installs it — here over an empty bar, so the
    /// chord is ⌘W.
    fn chord_env() -> Environment {
        let mut env = Environment::new();
        env.insert(CloseWindowChord::new(&Computed::constant(Vec::new()), &env));
        env
    }

    /// A declared Close Window is the standard item, armed with the chord
    /// the menu bar leaves free — ⌘W here, since nothing binds it.
    #[test]
    fn a_declared_close_window_emits_the_standard_item() {
        let env = chord_env();
        let nodes = menu_tree(&[ResolvedMenuItem::CloseWindow], &env);
        let [MenuTreeNode::Standard(command, MenuAction::CloseWindow)] = nodes.as_slice() else {
            panic!("a declared Close Window emits the standard Close node: {nodes:?}");
        };
        assert_eq!(command.label, "Close");
        assert_eq!(command.key_equivalent, "w");
        assert_eq!(command.modifiers, KeyModifiers::COMMAND);
    }

    #[test]
    fn a_nested_declared_close_window_emits_the_standard_item() {
        let env = chord_env();
        let items = declared(vec![Menu::new("Window", (MenuItem::CloseWindow,))]);
        let nodes = menu_tree(&items, &env);
        let [MenuTreeNode::Submenu(_, children)] = nodes.as_slice() else {
            panic!("a declared menu nests: {nodes:?}");
        };
        assert!(
            matches!(
                children.as_slice(),
                [MenuTreeNode::Standard(_, MenuAction::CloseWindow)]
            ),
            "a nested Close Window emits the standard Close node: {children:?}"
        );
    }
}
