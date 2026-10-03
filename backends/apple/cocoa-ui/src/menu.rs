//! Menu content shared by the `appkit` and `uikit` menu builders: what a
//! command says and how a tree of menus is shaped. Platform-side builders
//! turn these into `NSMenu`/`UIMenu`.
//!
//! # Safety
//!
//! No unsafe code.

use std::fmt;
use std::rc::Rc;

/// A command item's content.
#[derive(Clone, Default)]
pub struct Command {
    /// The item's title.
    pub label: String,
    /// A line of help text under the title.
    pub subtitle: Option<String>,
    /// A system-symbol icon name.
    pub symbol: Option<String>,
    /// Whether the item is destructive.
    pub destructive: bool,
    /// Whether the item can be chosen.
    pub enabled: bool,
    /// Whether the item shows a checkmark.
    pub selected: bool,
    /// The key equivalent — a lowercase letter, or `""` for none.
    pub key_equivalent: String,
    /// The modifiers held with the key equivalent.
    pub modifiers: KeyModifiers,
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Command")
            .field("label", &self.label)
            .field("subtitle", &self.subtitle)
            .field("symbol", &self.symbol)
            .field("destructive", &self.destructive)
            .field("enabled", &self.enabled)
            .field("selected", &self.selected)
            .field("key_equivalent", &self.key_equivalent)
            .field("modifiers", &self.modifiers)
            .finish()
    }
}

/// One node of a menu tree.
pub enum MenuTreeNode {
    /// A command and its action.
    Command(Command, Rc<dyn Fn()>),
    /// A separator line.
    Divider,
    /// A nested menu.
    Submenu(Command, Vec<Self>),
}

impl fmt::Debug for MenuTreeNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(command, _) => f.debug_tuple("Command").field(command).finish(),
            Self::Divider => f.write_str("Divider"),
            Self::Submenu(command, children) => f
                .debug_tuple("Submenu")
                .field(command)
                .field(children)
                .finish(),
        }
    }
}

bitflags::bitflags! {
    /// Modifier keys held with a key equivalent.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct KeyModifiers: u8 {
        /// The Command key, ⌘.
        const COMMAND = 1 << 0;
        /// The Option key, ⌥.
        const OPTION = 1 << 1;
        /// The Shift key, ⇧.
        const SHIFT = 1 << 2;
        /// The Control key, ⌃.
        const CONTROL = 1 << 3;
    }
}

impl KeyModifiers {
    /// The `NSEventModifierFlags` equivalent.
    #[cfg(target_os = "macos")]
    #[must_use]
    pub(crate) fn native(self) -> objc2_app_kit::NSEventModifierFlags {
        let mut flags = objc2_app_kit::NSEventModifierFlags::empty();
        if self.contains(Self::COMMAND) {
            flags |= objc2_app_kit::NSEventModifierFlags::Command;
        }
        if self.contains(Self::OPTION) {
            flags |= objc2_app_kit::NSEventModifierFlags::Option;
        }
        if self.contains(Self::SHIFT) {
            flags |= objc2_app_kit::NSEventModifierFlags::Shift;
        }
        if self.contains(Self::CONTROL) {
            flags |= objc2_app_kit::NSEventModifierFlags::Control;
        }
        flags
    }
}
