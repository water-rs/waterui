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
//! handler block, while a `UIKeyCommand` has no block and is routed as an
//! action message through `cocoaUiMenuCommandFired:` on the responder
//! chain, which resolves the [`MenuCommandTarget`] the command retains as
//! an associated object and fires the callback.
//!
//! # Safety
//!
//! The `unsafe` here builds `UIAction`s from `RcBlock`s and `UIKeyCommand`s
//! carrying `MenuCommandTarget`s, and calls `objc2`/`UIKit` bindings marked
//! unsafe because `UIKit` objects are main-thread only, which
//! [`MainThreadMarker`] guarantees at construction. `UIAction`'s handler
//! block is `copy`ed by the call that consumes it, so the stack block it
//! is built from may be dropped once construction returns; and a
//! `UIKeyCommand`'s `objc_setAssociatedObject` retains the target for the
//! command's whole lifetime.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::ffi::{
    OBJC_ASSOCIATION_RETAIN_NONATOMIC, objc_getAssociatedObject, objc_setAssociatedObject,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSObject, NSString};
use objc2_ui_kit::{
    UIAction, UIColor, UIImage, UIKeyCommand, UIMenu, UIMenuElement, UIMenuElementAttributes,
    UIMenuElementState, UIMenuOptions, UIView,
};

use crate::menu::{Command, MenuTreeNode};
use crate::uikit::button::{Button, Chrome};

/// `MenuCommandTarget` instance variables: the command's Rust callback.
pub struct MenuCommandTargetIvars {
    handler: Rc<dyn Fn()>,
}

/// The `objc_setAssociatedObject` key under which a `command_element`-built
/// `UIKeyCommand` retains its `MenuCommandTarget`.
static TARGET_KEY: u8 = 0;

/// The `MenuCommandTarget` `command_element` attached to `element`, if any.
pub fn menu_command_target(element: &UIMenuElement) -> Option<&MenuCommandTarget> {
    // SAFETY: `element` is a live `UIMenuElement` and `TARGET_KEY` is the
    // association key `command_element` installs under.
    let target = unsafe {
        objc_getAssociatedObject(
            NonNull::from(element).cast::<AnyObject>().as_ptr(),
            std::ptr::from_ref(&TARGET_KEY).cast::<c_void>(),
        )
    };
    // SAFETY: the only association under `TARGET_KEY` is the retained
    // `MenuCommandTarget` `command_element` installs, so a non-null result
    // is a live `AnyObject` reference of that class.
    unsafe { target.as_ref() }.and_then(AnyObject::downcast_ref::<MenuCommandTarget>)
}

define_class!(
    // SAFETY: `NSObject`'s only initializer contract is `init`, which the
    // allocation below calls, and the class does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiMenuCommandTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuCommandTargetIvars]
    /// The callback owner a `UIKeyCommand` carries: `UIKit` routes a key
    /// command as an action message, so the command's Rust callback cannot
    /// live behind a block the way `UIAction`'s does. The target rides on
    /// the element as an associated object and `AppDelegate`'s
    /// `cocoaUiMenuCommandFired:` — the end of the responder chain — reads
    /// it back out of the sender to fire the callback.
    pub struct MenuCommandTarget;

    impl MenuCommandTarget {}
);

impl MenuCommandTarget {
    /// A target owning `handler`.
    fn new(mtm: MainThreadMarker, handler: Rc<dyn Fn()>) -> Retained<Self> {
        let this = mtm
            .alloc::<Self>()
            .set_ivars(MenuCommandTargetIvars { handler });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// Runs the command's callback.
    pub(crate) fn fire(&self) {
        let handler = self.ivars().handler.clone();
        handler();
    }
}

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
    let title = NSString::from_str(&command.label);
    let image = menu_image(command.symbol.as_deref());
    if command.key_equivalent.is_empty() {
        let block = RcBlock::new(move |_action: NonNull<UIAction>| {
            action();
        });
        // SAFETY: `actionWithTitle:image:identifier:handler:` retains the block.
        let ui_action = unsafe {
            UIAction::actionWithTitle_image_identifier_handler(
                &title,
                image.as_deref(),
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
        ui_action.into_super()
    } else {
        let target = MenuCommandTarget::new(mtm, action);
        // SAFETY: `commandWithTitle:...` is `UIKeyCommand`'s designated
        // constructor and `cocoaUiMenuCommandFired:` is a selector
        // `AppDelegate` implements. `propertyList` stays `nil`: `UIKit`
        // asserts it is a property-list type, which `MenuCommandTarget` is
        // not, so the target attaches as an associated object below.
        let key_command = unsafe {
            UIKeyCommand::commandWithTitle_image_action_input_modifierFlags_propertyList(
                &title,
                image.as_deref(),
                sel!(cocoaUiMenuCommandFired:),
                &NSString::from_str(&command.key_equivalent),
                command.modifiers.native(),
                None,
                mtm,
            )
        };
        // SAFETY: `key_command` is a live `UIMenuElement` and `target` an
        // `NSObject`; `OBJC_ASSOCIATION_RETAIN_NONATOMIC` makes the command
        // retain its callback's owner for the command's whole lifetime.
        unsafe {
            objc_setAssociatedObject(
                NonNull::from(&*key_command).cast::<AnyObject>().as_ptr(),
                std::ptr::from_ref(&TARGET_KEY).cast::<c_void>(),
                NonNull::from(&*target).cast::<AnyObject>().as_ptr(),
                OBJC_ASSOCIATION_RETAIN_NONATOMIC,
            );
        }
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
    /// the same shape [`menu`] gives the same command.
    #[must_use]
    pub fn command(mtm: MainThreadMarker, command: &Command, handler: impl Fn() + 'static) -> Self {
        Self {
            action: command_element(mtm, command, Rc::new(handler)),
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
