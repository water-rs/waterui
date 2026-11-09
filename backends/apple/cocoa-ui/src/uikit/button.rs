//! The `UIKit` button: a `UIButton` of `.system` type driven by a
//! `UIButtonConfiguration` for its chrome.
//!
//! The configuration sets the button's chrome (bordered / glass / plain);
//! `Button` adds what a configuration cannot express: the label's content
//! insets, tint, enabled state and accessibility hooks.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIButton` subclass, forwards to
//! `UIButton`'s own `setEnabled:` implementation, and calls `objc2`
//! bindings marked unsafe because `UIKit` view APIs are main-thread only —
//! which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

#[cfg(target_abi = "macabi")]
use std::cell::Cell;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSArray, NSAttributedString, NSObjectProtocol, NSString};
#[cfg(target_abi = "macabi")]
use objc2_ui_kit::UIWindow;
use objc2_ui_kit::{
    NSDirectionalEdgeInsets, UIButton, UIButtonConfiguration, UIButtonType, UIColor, UICommand,
    UIControl, UIControlState, UIKeyCommand,
};

use super::key_commands::{KeyCommandRegistry, KeyCommands};
use crate::callback::{emit, guarded};
use crate::geometry::EdgeInsets;

/// The chrome a `UIButtonConfiguration` draws around the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chrome {
    /// No drawn chrome; the label sits on the surrounding surface.
    Plain,
    /// A filled, low-emphasis capsule.
    Gray,
    /// A filled, accent-tinted capsule.
    Filled,
    /// A liquid-glass capsule.
    Glass,
    /// A liquid-glass capsule in its prominent variant.
    ProminentGlass,
}

/// Runs after each `setEnabled:`, with the new value.
type EnabledHandler = Rc<dyn Fn(bool)>;

/// The ivars of [`CocoaUiButton`].
pub struct CocoaUiButtonIvars {
    /// Called with the new value after each `setEnabled:`.
    enabled_handler: RefCell<Option<EnabledHandler>>,
    key_commands: Rc<KeyCommandRegistry>,
    /// `UIKit` has presented this button's menu and not yet ended it.
    #[cfg(target_abi = "macabi")]
    menu_open: Cell<bool>,
}

impl fmt::Debug for CocoaUiButtonIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CocoaUiButtonIvars")
            .field("enabled_handler", &self.enabled_handler.borrow().is_some())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UIButton`'s designated initializer is `initWithFrame:`;
    // both `new` and `+buttonWithType:` reach the override, and the class does
    // not implement `Drop`.
    #[unsafe(super(UIButton))]
    #[name = "CocoaUiButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = CocoaUiButtonIvars]
    #[derive(Debug)]
    /// A `UIButton` reporting enabled-state changes to a handler.
    ///
    /// The only `UIButton` cocoa-ui constructs: chrome mirroring a rendered
    /// button's enabled state — a bar item's `isEnabled` — installs a
    /// handler here, having downcast `first_button`'s result to this class.
    pub struct CocoaUiButton;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIButton`.
    unsafe impl NSObjectProtocol for CocoaUiButton {}

    impl CocoaUiButton {
        // SAFETY: the designated initializer writes ivars before calling super.
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: CGRect) -> Retained<Self> {
            let this = this.set_ivars(CocoaUiButtonIvars {
                enabled_handler: RefCell::new(None),
                key_commands: Rc::new(KeyCommandRegistry::default()),
                #[cfg(target_abi = "macabi")]
                menu_open: Cell::new(false),
            });
            // SAFETY: the inherited designated initializer has this signature.
            unsafe { msg_send![super(this), initWithFrame: frame] }
        }

        // SAFETY: armed commands send this selector with themselves as sender.
        #[unsafe(method(cocoaUiButtonMenuCommandFired:))]
        fn menu_command_fired(&self, command: &UICommand) {
            guarded("CocoaUiButton cocoaUiButtonMenuCommandFired:", || {
                self.ivars().key_commands.fire(command);
            });
        }

        // SAFETY: UIResponder's declared signature, forwarding other selectors to super.
        #[unsafe(method(canPerformAction:withSender:))]
        fn can_perform_action_with_sender(&self, action: Sel, sender: Option<&AnyObject>) -> Bool {
            if action != sel!(cocoaUiButtonMenuCommandFired:) {
                // SAFETY: the inherited responder query has this signature.
                return unsafe { msg_send![super(self), canPerformAction: action, withSender: sender] };
            }
            Bool::from(sender.and_then(|sender| sender.downcast_ref::<UICommand>()).map_or_else(
                || self.ivars().key_commands.any_armed(),
                |command| self.ivars().key_commands.owns(command),
            ))
        }

        // SAFETY: the responder property returns retained commands from the menu.
        #[unsafe(method_id(keyCommands))]
        fn key_commands_override(&self) -> Option<Retained<NSArray<UIKeyCommand>>> {
            guarded("CocoaUiButton keyCommands", || {
                // SAFETY: `UIResponder.keyCommands`' declared signature.
                let inherited: Option<Retained<NSArray<UIKeyCommand>>> =
                    unsafe { msg_send![super(self), keyCommands] };
                let Some(menu) = self.menu() else {
                    return inherited;
                };
                self.ivars().key_commands.commands(
                    &super::menu::key_commands(&menu),
                    inherited,
                    sel!(cocoaUiButtonMenuCommandFired:),
                )
            })
        }

        // `UIControl` extends `UIContextMenuInteractionDelegate` with these
        // two methods, which it requires subclasses to forward to `super`.
        // SAFETY: the declared signatures, forwarding every argument.
        #[cfg(target_abi = "macabi")]
        #[unsafe(method(contextMenuInteraction:willDisplayMenuForConfiguration:animator:))]
        fn will_display_menu(
            &self,
            interaction: &AnyObject,
            configuration: &AnyObject,
            animator: Option<&AnyObject>,
        ) {
            guarded("CocoaUiButton willDisplayMenu", || {
                // SAFETY: see above.
                unsafe {
                    let () = msg_send![
                        super(self),
                        contextMenuInteraction: interaction,
                        willDisplayMenuForConfiguration: configuration,
                        animator: animator
                    ];
                }
                self.ivars().menu_open.set(true);
            });
        }

        // SAFETY: see `will_display_menu`.
        #[cfg(target_abi = "macabi")]
        #[unsafe(method(contextMenuInteraction:willEndForConfiguration:animator:))]
        fn will_end_menu(
            &self,
            interaction: &AnyObject,
            configuration: &AnyObject,
            animator: Option<&AnyObject>,
        ) {
            guarded("CocoaUiButton willEndMenu", || {
                // SAFETY: see `will_display_menu`.
                unsafe {
                    let () = msg_send![
                        super(self),
                        contextMenuInteraction: interaction,
                        willEndForConfiguration: configuration,
                        animator: animator
                    ];
                }
                self.ivars().menu_open.set(false);
            });
        }

        // A menu open on a button that leaves its window is ended first:
        // `removeFromSuperview` does not close an `NSMenu` popup.
        // SAFETY: `UIView.willMoveToWindow:`'s declared signature.
        #[cfg(target_abi = "macabi")]
        #[unsafe(method(willMoveToWindow:))]
        fn will_move_to_window(&self, window: Option<&UIWindow>) {
            guarded("CocoaUiButton willMoveToWindow", || {
                if window.is_none() {
                    self.end_open_menu();
                }
                // SAFETY: the inherited method has this signature.
                let () = unsafe { msg_send![super(self), willMoveToWindow: window] };
            });
        }

        // SAFETY: `setEnabled:` is `UIControl`'s setter; the override
        // forwards to `super` unchanged.
        #[unsafe(method(setEnabled:))]
        fn set_enabled(&self, enabled: bool) {
            guarded("CocoaUiButton setEnabled", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setEnabled: enabled] };
                emit(&self.ivars().enabled_handler, enabled);
            });
        }
    }
);

impl CocoaUiButton {
    /// A button of `button_type` — `+[UIButton buttonWithType:]` invoked
    /// on this subclass.
    #[must_use]
    pub fn with_type(mtm: MainThreadMarker, button_type: UIButtonType) -> Retained<Self> {
        let _ = mtm;
        // SAFETY: invoked on the subclass itself, `buttonWithType:`
        // allocates through `self`, so the result is a `CocoaUiButton`.
        let this: Retained<Self> = unsafe { msg_send![Self::class(), buttonWithType: button_type] };
        this
    }

    /// A button through `-[UIButton initWithFrame:]` — the plain initializer the
    /// `UIButton::new` sites share.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: the designated initializer initializes this subclass's ivars.
        unsafe { msg_send![Self::alloc(mtm), initWithFrame: CGRect::ZERO] }
    }

    /// Ends the tracking session of this button's menu if it is open, so
    /// no command of the menu stays reachable. Under Catalyst the menu is
    /// an `NSMenu` popup that neither `setMenu:` nor leaving the window
    /// closes (see `catalyst_tracking`).
    ///
    /// # Panics
    ///
    /// When `UIKit` reported the menu open and no `NSMenu` is being
    /// tracked.
    #[cfg(target_abi = "macabi")]
    pub fn end_open_menu(&self) {
        if self.ivars().menu_open.replace(false) {
            super::application::menu_tracking(self.mtm()).cancel();
        }
    }

    /// A scope routing menu commands through this button's own registry.
    #[must_use]
    pub fn menu_commands(&self) -> KeyCommands {
        KeyCommands::in_registry_with_action(
            Rc::clone(&self.ivars().key_commands),
            sel!(cocoaUiButtonMenuCommandFired:),
        )
    }

    /// How many commands this button's menu has armed.
    #[cfg(feature = "native-test")]
    #[must_use]
    pub fn armed_command_count(&self) -> usize {
        self.ivars().key_commands.armed()
    }

    /// Runs `handler` with the new value after each `setEnabled:` —
    /// whichever path changed enabled, not only [`Button::set_enabled`].
    pub fn set_enabled_handler(&self, handler: impl Fn(bool) + 'static) {
        self.ivars().enabled_handler.replace(Some(Rc::new(handler)));
    }
}

/// An interactive button backed by `UIButton` (`.system` type).
///
/// Derefs to `UIButton` so callers can reach platform APIs the wrapper does
/// not expose; prefer the wrapper's methods where they exist.
#[derive(Debug, Clone)]
pub struct Button {
    inner: Retained<CocoaUiButton>,
}

impl Button {
    /// Ends the tracking session of this button's menu if it is open; see
    /// [`CocoaUiButton::end_open_menu`].
    #[cfg(target_abi = "macabi")]
    pub fn end_open_menu(&self) {
        self.inner.end_open_menu();
    }

    /// A scope routing menu commands through this button's own registry.
    #[must_use]
    pub fn menu_commands(&self) -> KeyCommands {
        self.inner.menu_commands()
    }

    /// A `.system`-type button with no title.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            inner: CocoaUiButton::with_type(mtm, UIButtonType::System),
        }
    }

    /// The control, for wiring `ActionTarget`.
    #[must_use]
    pub fn control(&self) -> &UIControl {
        &self.inner
    }

    /// Enables or disables user interaction.
    pub fn set_enabled(&self, enabled: bool) {
        self.inner.setEnabled(enabled);
    }

    /// Whether the button currently accepts interaction.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.inner.isEnabled()
    }

    /// The plain-text title.
    pub fn set_title(&self, title: &str) {
        let title = NSString::from_str(title);
        self.inner
            .setTitle_forState(Some(&title), UIControlState::Normal);
    }

    /// The styled title, as a single attributed run.
    pub fn set_attributed_title(&self, title: &NSAttributedString) {
        self.inner
            .setAttributedTitle_forState(Some(title), UIControlState::Normal);
    }

    /// The color the button's chrome and title tint derive from.
    ///
    /// # Safety
    ///
    /// `setTintColor:` is marked unsafe in the bindings; it is an ordinary
    /// main-thread `UIKit` setter.
    pub fn set_tint_color(&self, color: &UIColor) {
        // SAFETY: an ordinary main-thread `UIKit` setter.
        unsafe { self.inner.setTintColor(Some(color)) };
    }

    /// Presents `menu` on interaction, replacing the button's own action.
    pub fn set_menu(&self, menu: Option<&objc2_ui_kit::UIMenu>) {
        self.inner.setMenu(menu);
    }

    /// Whether a touch-down opens the menu instead of firing the action —
    /// `showsMenuAsPrimaryAction`.
    pub fn set_shows_menu_as_primary_action(&self, shows: bool) {
        self.inner.setShowsMenuAsPrimaryAction(shows);
    }

    /// Draws `chrome` around the label with zero added content padding: the
    /// owning layout supplies the padding it measured with
    /// [`Self::chrome_content_insets`].
    ///
    /// Keeping the padding outside the configuration means a button laid out
    /// by hand does not double-count the insets.
    pub fn set_chrome(&self, chrome: Chrome, mtm: MainThreadMarker) {
        let configuration = chrome_configuration(chrome, mtm);
        configuration.setContentInsets(NSDirectionalEdgeInsets {
            top: 0.0,
            leading: 0.0,
            bottom: 0.0,
            trailing: 0.0,
        });
        self.inner.setConfiguration(Some(&configuration));
    }

    /// The insets `chrome`'s configuration applies by default — how far the
    /// chrome keeps the label from its edge — for the owning layout to mirror
    /// in its own padding.
    #[must_use]
    pub fn chrome_content_insets(chrome: Chrome, mtm: MainThreadMarker) -> EdgeInsets {
        let configuration = chrome_configuration(chrome, mtm);
        let NSDirectionalEdgeInsets {
            top,
            leading,
            bottom,
            trailing,
        } = configuration.contentInsets();
        EdgeInsets::new(top, leading, bottom, trailing)
    }
}

impl AsRef<UIButton> for Button {
    fn as_ref(&self) -> &UIButton {
        &self.inner
    }
}

impl core::ops::Deref for Button {
    type Target = UIButton;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

fn chrome_configuration(chrome: Chrome, mtm: MainThreadMarker) -> Retained<UIButtonConfiguration> {
    match chrome {
        Chrome::Plain => UIButtonConfiguration::plainButtonConfiguration(mtm),
        Chrome::Gray => UIButtonConfiguration::grayButtonConfiguration(mtm),
        Chrome::Filled => UIButtonConfiguration::filledButtonConfiguration(mtm),
        Chrome::Glass => UIButtonConfiguration::glassButtonConfiguration(mtm),
        Chrome::ProminentGlass => UIButtonConfiguration::prominentGlassButtonConfiguration(mtm),
    }
}
