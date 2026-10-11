//! `UIMenu` built from the shared menu tree, plus the button that presents
//! one and the `Menu`/`MenuElement`/`MenuAction` tree a handler builds one
//! from imperatively.
//!
//! `elements` turns [`MenuTreeNode`]s into `UIMenuElement`s — dividers
//! become inline groups, commands become `UIAction`s — or `UIKeyCommand`s
//! when the command declares a shortcut, the only `UIMenuElement` that
//! arms a chord — carrying the command's attributes (disabled,
//! destructive, checked) — and `menu` wraps them into a `UIMenu`. Both
//! element kinds run the command's Rust callback: a `UIAction` calls its
//! handler block, while a `UIKeyCommand` has no block and names its
//! callback through the [`KeyCommands`] scope it is armed in (see
//! `key_commands`).
//!
//! # Safety
//!
//! The `unsafe` here builds `UIAction`s from `RcBlock`s and `UIKeyCommand`s
//! from a selector `AppDelegate` implements, and calls `objc2`/`UIKit`
//! bindings marked unsafe because `UIKit` objects are main-thread only,
//! which [`MainThreadMarker`] guarantees at construction. `UIAction`'s
//! handler block is `copy`ed by the call that consumes it, so the stack
//! block it is built from may be dropped once construction returns.

use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, sel};
use objc2_foundation::{NSArray, NSString};
use objc2_ui_kit::{
    UIAction, UIColor, UIImage, UIKeyCommand, UIMenu, UIMenuElement, UIMenuElementAttributes,
    UIMenuElementState, UIMenuOptions, UIView,
};

use super::key_commands::KeyCommands;
use crate::menu::{Command, MenuTreeNode};
use crate::uikit::button::{Button, Chrome};

/// A `UIMenu` from a menu tree, its shortcut commands armed in
/// `key_commands`.
#[must_use]
pub fn menu(
    mtm: MainThreadMarker,
    key_commands: &KeyCommands,
    title: &Command,
    nodes: &[MenuTreeNode],
) -> Retained<UIMenu> {
    menu_with_identifier(mtm, key_commands, title, None, nodes)
}

/// A `UIMenu` from a menu tree, named `identifier` for `UIMenuBuilder`
/// lookups — the application menu bar relies on that stability — its
/// shortcut commands armed in `key_commands`.
#[must_use]
pub fn menu_with_identifier(
    mtm: MainThreadMarker,
    key_commands: &KeyCommands,
    title: &Command,
    identifier: Option<&str>,
    nodes: &[MenuTreeNode],
) -> Retained<UIMenu> {
    let image = menu_image(title.symbol.as_deref());
    UIMenu::menuWithTitle_image_identifier_options_children(
        &NSString::from_str(&title.label),
        image.as_deref(),
        identifier.map(NSString::from_str).as_deref(),
        UIMenuOptions::empty(),
        &objc2_foundation::NSArray::from_slice(
            &elements(mtm, key_commands, nodes)
                .iter()
                .map(|e| &**e)
                .collect::<Vec<_>>(),
        ),
        mtm,
    )
}

/// `UIMenuElement`s from a menu tree — dividers split the list into inline
/// groups — their shortcut commands armed in `key_commands`.
#[must_use]
pub fn elements(
    mtm: MainThreadMarker,
    key_commands: &KeyCommands,
    nodes: &[MenuTreeNode],
) -> Vec<Retained<UIMenuElement>> {
    // `MenuTreeNode` is not `Clone`; walk references.
    let mut current: Vec<&MenuTreeNode> = Vec::new();
    let mut group_refs: Vec<Vec<&MenuTreeNode>> = Vec::new();
    for node in nodes {
        if matches!(node, MenuTreeNode::Divider) {
            if !current.is_empty() {
                group_refs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(node);
        }
    }
    if !current.is_empty() {
        group_refs.push(current);
    }
    let mut out: Vec<Retained<UIMenuElement>> = Vec::new();
    let single = group_refs.len() == 1;
    for group in group_refs {
        let children: Vec<Retained<UIMenuElement>> = group
            .iter()
            .filter_map(|node| element(mtm, key_commands, node))
            .collect();
        if single {
            out.extend(children);
            continue;
        }
        out.push(
            UIMenu::menuWithTitle_image_identifier_options_children(
                &NSString::new(),
                None,
                None,
                UIMenuOptions::DisplayInline,
                &objc2_foundation::NSArray::from_slice(
                    &children.iter().map(|e| &**e).collect::<Vec<_>>(),
                ),
                mtm,
            )
            .into_super(),
        );
    }
    out
}

fn element(
    mtm: MainThreadMarker,
    key_commands: &KeyCommands,
    node: &MenuTreeNode,
) -> Option<Retained<UIMenuElement>> {
    match node {
        MenuTreeNode::Divider => None,
        MenuTreeNode::Submenu(command, children) => {
            Some(menu(mtm, key_commands, command, children).into_super())
        }
        MenuTreeNode::Command(command, action) => {
            Some(command_element(mtm, key_commands, command, action.clone()))
        }
    }
}

fn command_element(
    mtm: MainThreadMarker,
    key_commands: &KeyCommands,
    command: &Command,
    action: Rc<dyn Fn()>,
) -> Retained<UIMenuElement> {
    let mut attributes = UIMenuElementAttributes::empty();
    if !command.enabled {
        attributes |= UIMenuElementAttributes::Disabled;
    }
    if command.destructive {
        attributes |= UIMenuElementAttributes::Destructive;
    }
    let state = if command.selected {
        UIMenuElementState::On
    } else {
        UIMenuElementState::Off
    };
    let title = NSString::from_str(&command.label);
    let image = menu_image(command.symbol.as_deref());
    if command.key_equivalent.is_empty() {
        let block = RcBlock::new(move |_action: NonNull<UIAction>| {
            action();
        });
        // SAFETY: `actionWithTitle:image:identifier:handler:` copies the
        // block, so `block` may be dropped once the call returns.
        let ui_action = unsafe {
            UIAction::actionWithTitle_image_identifier_handler(
                &title,
                image.as_deref(),
                None,
                RcBlock::as_ptr(&block).cast(),
                mtm,
            )
        };
        ui_action.setAttributes(attributes);
        ui_action.setState(state);
        if let Some(subtitle) = &command.subtitle {
            ui_action.setSubtitle(Some(&NSString::from_str(subtitle)));
        }
        ui_action.into_super()
    } else {
        let property_list = key_commands.arm(action);
        // SAFETY: `commandWithTitle:...` is `UIKeyCommand`'s designated
        // constructor, `cocoaUiMenuCommandFired:` is a selector
        // `AppDelegate` implements, and an `NSNumber` is a property-list
        // object.
        let key_command = unsafe {
            UIKeyCommand::commandWithTitle_image_action_input_modifierFlags_propertyList(
                &title,
                image.as_deref(),
                sel!(cocoaUiMenuCommandFired:),
                &NSString::from_str(&command.key_equivalent),
                command.modifiers.native(),
                Some(AsRef::<AnyObject>::as_ref(&*property_list)),
                mtm,
            )
        };
        key_command.setAttributes(attributes);
        key_command.setState(state);
        if let Some(subtitle) = &command.subtitle {
            key_command.setSubtitle(Some(&NSString::from_str(subtitle)));
        }
        key_command.into_super().into_super()
    }
}

fn menu_image(symbol: Option<&str>) -> Option<Retained<UIImage>> {
    symbol.and_then(|symbol| UIImage::systemImageNamed(&NSString::from_str(symbol)))
}

/// One child of a [`Menu`]: a triggerable action or a nested menu.
#[derive(Debug, Clone)]
pub enum MenuElement {
    /// An action the user picks.
    Action(MenuAction),
    /// A nested menu shown hierarchically.
    Submenu(Menu),
}

impl MenuElement {
    fn native(&self) -> Retained<UIMenuElement> {
        match self {
            Self::Action(action) => action.action.clone(),
            Self::Submenu(menu) => menu.menu.clone().into_super(),
        }
    }
}

/// A `UIMenu`: an immutable list of [`MenuElement`] children.
///
/// A `Menu` is a handle: clones refer to the same menu.
#[derive(Debug, Clone)]
pub struct Menu {
    menu: Retained<UIMenu>,
}

impl Menu {
    /// A menu titled `title` with `children`; `icon`, when given, is an
    /// `SF Symbols` name drawn beside the title in a submenu row.
    ///
    /// `inline` marks the menu `.displayInline`: its children flatten into
    /// their parent as a labelled group rather than nesting.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        title: &str,
        icon: Option<&str>,
        inline: bool,
        children: &[MenuElement],
    ) -> Self {
        let image = icon.and_then(|name| UIImage::systemImageNamed(&NSString::from_str(name)));
        let options = if inline {
            UIMenuOptions::DisplayInline
        } else {
            UIMenuOptions::empty()
        };
        let elements: Vec<Retained<UIMenuElement>> =
            children.iter().map(MenuElement::native).collect();
        let menu = UIMenu::menuWithTitle_image_identifier_options_children(
            &NSString::from_str(title),
            image.as_deref(),
            None,
            options,
            &NSArray::from_retained_slice(&elements),
            mtm,
        );
        Self { menu }
    }
}

/// A leaf element of a [`Menu`]: a `UIAction`, or a `UIKeyCommand` when
/// the command declares a shortcut.
///
/// A `MenuAction` is a handle: clones refer to the same element.
#[derive(Debug, Clone)]
pub struct MenuAction {
    action: Retained<UIMenuElement>,
}

impl MenuAction {
    /// The element `command` describes, running `handler` when chosen —
    /// the same shape [`menu`] gives the same command — its shortcut, when
    /// it declares one, armed in `key_commands`.
    #[must_use]
    pub fn command(
        mtm: MainThreadMarker,
        key_commands: &KeyCommands,
        command: &Command,
        handler: impl Fn() + 'static,
    ) -> Self {
        Self {
            action: command_element(mtm, key_commands, command, Rc::new(handler)),
        }
    }

    /// The wrapped `UIMenuElement`.
    #[must_use]
    pub fn element(&self) -> &UIMenuElement {
        &self.action
    }
}

/// A button whose primary action opens a `UIMenu`.
///
/// Built on [`Button`] with plain chrome and no content padding: the owning
/// layout supplies both the label view and the padding it measured with.
///
/// A `MenuButton` is a handle: clones refer to the same control.
#[derive(Debug, Clone)]
pub struct MenuButton {
    button: Button,
}

impl MenuButton {
    /// A plain-chrome button whose primary action opens its menu.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let button = Button::new(mtm);
        button.set_chrome(Chrome::Plain, mtm);
        button.setShowsMenuAsPrimaryAction(true);
        Self { button }
    }

    /// Replaces the menu the button opens.
    pub fn set_menu(&self, menu: &Menu) {
        self.button.setMenu(Some(&menu.menu));
    }

    /// The color the button's chrome derives from.
    pub fn set_tint_color(&self, color: &UIColor) {
        self.button.set_tint_color(color);
    }

    /// The button, for adding to a view hierarchy and framing.
    #[must_use]
    pub fn view(&self) -> &UIView {
        &self.button
    }
}

impl AsRef<UIView> for MenuButton {
    fn as_ref(&self) -> &UIView {
        &self.button
    }
}
