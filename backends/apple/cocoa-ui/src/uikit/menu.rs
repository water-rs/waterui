//! `UIMenu` built from the shared menu tree, plus the button that presents
//! one and the `Menu`/`MenuElement`/`MenuAction` tree a handler builds one
//! from imperatively.
//!
//! `elements` turns [`MenuTreeNode`]s into `UIMenuElement`s — dividers
//! become inline groups, commands become `UIAction`s carrying the command's
//! attributes (disabled, destructive, checked) — and `menu` wraps them into
//! a `UIMenu`. Actions run the command's Rust callback.
//!
//! # Safety
//!
//! The `unsafe` here builds `UIAction`s from `RcBlock`s and calls `objc2`/
//! `UIKit` bindings marked unsafe because `UIKit` objects are main-thread
//! only, which [`MainThreadMarker`] guarantees at construction. `UIAction`'s
//! handler block is `copy`ed by the call that consumes it, so the stack
//! block it is built from may be dropped once construction returns.

use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};
use objc2_ui_kit::{
    UIAction, UIColor, UIImage, UIMenu, UIMenuElement, UIMenuElementAttributes, UIMenuElementState,
    UIMenuOptions, UIView,
};

use crate::menu::{Command, MenuTreeNode};
use crate::uikit::button::{Button, Chrome};

/// A `UIMenu` from a menu tree.
#[must_use]
pub fn menu(mtm: MainThreadMarker, title: &Command, nodes: &[MenuTreeNode]) -> Retained<UIMenu> {
    menu_with_identifier(mtm, title, None, nodes)
}

/// A `UIMenu` from a menu tree, named `identifier` for `UIMenuBuilder`
/// lookups — the application menu bar relies on that stability.
#[must_use]
pub fn menu_with_identifier(
    mtm: MainThreadMarker,
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
            &elements(mtm, nodes)
                .iter()
                .map(|e| &**e)
                .collect::<Vec<_>>(),
        ),
        mtm,
    )
}

/// `UIMenuElement`s from a menu tree — dividers split the list into inline
/// groups.
#[must_use]
pub fn elements(mtm: MainThreadMarker, nodes: &[MenuTreeNode]) -> Vec<Retained<UIMenuElement>> {
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
        let children: Vec<Retained<UIMenuElement>> =
            group.iter().filter_map(|node| element(mtm, node)).collect();
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

fn element(mtm: MainThreadMarker, node: &MenuTreeNode) -> Option<Retained<UIMenuElement>> {
    match node {
        MenuTreeNode::Divider => None,
        MenuTreeNode::Submenu(command, children) => Some(menu(mtm, command, children).into_super()),
        MenuTreeNode::Command(command, action) => {
            Some(command_element(mtm, command, action.clone()))
        }
    }
}

fn command_element(
    mtm: MainThreadMarker,
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
    let block = RcBlock::new(move |_action: NonNull<UIAction>| {
        action();
    });
    // SAFETY: `actionWithTitle:image:identifier:handler:` retains the block.
    let ui_action = unsafe {
        UIAction::actionWithTitle_image_identifier_handler(
            &NSString::from_str(&command.label),
            menu_image(command.symbol.as_deref()).as_deref(),
            None,
            block2::RcBlock::into_raw(block),
            mtm,
        )
    };
    ui_action.setAttributes(attributes);
    ui_action.setState(state);
    if let Some(subtitle) = &command.subtitle {
        ui_action.setSubtitle(Some(&NSString::from_str(subtitle)));
    }
    if !command.key_equivalent.is_empty() {
        ui_action.setDiscoverabilityTitle(Some(&NSString::from_str(&command.key_equivalent)));
    }
    ui_action.into_super()
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
            Self::Action(action) => action.action.clone().into_super(),
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

/// A `UIAction`: one pickable row of a [`Menu`].
///
/// A `MenuAction` is a handle: clones refer to the same action.
#[derive(Debug, Clone)]
pub struct MenuAction {
    action: Retained<UIAction>,
}

impl MenuAction {
    /// An action titled `title` that runs `handler` when picked.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, title: &str, handler: impl Fn() + 'static) -> Self {
        let block = RcBlock::new(move |_action: NonNull<UIAction>| handler());
        // SAFETY: `handler` takes a block the call copies, so the stack
        // block may be dropped once the call returns.
        let action = unsafe {
            UIAction::actionWithTitle_image_identifier_handler(
                &NSString::from_str(title),
                None,
                None,
                RcBlock::as_ptr(&block).cast(),
                mtm,
            )
        };
        Self { action }
    }

    /// The same action, subtitled `subtitle` beneath its title when given.
    #[must_use]
    pub fn with_subtitle(self, subtitle: Option<&str>) -> Self {
        self.action
            .setSubtitle(subtitle.map(NSString::from_str).as_deref());
        self
    }

    /// The same action, drawn with the `SF Symbols` image `name` when
    /// given.
    #[must_use]
    pub fn with_icon(self, name: Option<&str>) -> Self {
        if let Some(name) = name {
            self.action
                .setImage(UIImage::systemImageNamed(&NSString::from_str(name)).as_deref());
        }
        self
    }

    /// The same action, greyed out when `disabled` is true.
    #[must_use]
    pub fn with_disabled(self, disabled: bool) -> Self {
        let mut attributes = self.action.attributes();
        attributes.set(UIMenuElementAttributes::Disabled, disabled);
        self.action.setAttributes(attributes);
        self
    }

    /// The same action, drawn in the system's destructive red when
    /// `destructive` is true.
    #[must_use]
    pub fn with_destructive(self, destructive: bool) -> Self {
        let mut attributes = self.action.attributes();
        attributes.set(UIMenuElementAttributes::Destructive, destructive);
        self.action.setAttributes(attributes);
        self
    }

    /// The same action, drawn with a checkmark when `selected` is true.
    #[must_use]
    pub fn with_selected(self, selected: bool) -> Self {
        self.action.setState(if selected {
            UIMenuElementState::On
        } else {
            UIMenuElementState::Off
        });
        self
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
