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

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSAttributedString, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    NSDirectionalEdgeInsets, UIButton, UIButtonConfiguration, UIButtonType, UIColor, UIControl,
    UIControlState,
};

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
}

impl fmt::Debug for CocoaUiButtonIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CocoaUiButtonIvars")
            .field("enabled_handler", &self.enabled_handler.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `UIButton`'s designated initializer is `initWithFrame:`;
    // `CocoaUiButton::new` reaches it through `init` and `with_type`
    // through `+buttonWithType:`'s own initialization, and the class does
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
        // SAFETY: `setEnabled:` is `UIControl`'s setter; the override
        // forwards to `super` unchanged.
        #[unsafe(method(setEnabled:))]
        fn set_enabled(&self, enabled: bool) {
            guarded("CocoaUiButton setEnabled", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setEnabled: enabled] };
                // `setEnabled:` may run inside `buttonWithType:`'s own
                // initialization, before `mark_ivars_initialized` could
                // have run — read the storage without the initialized
                // check.
                // SAFETY: the storage is `set_ivars`-written, marked, or
                // still zero-filled, all valid `CocoaUiButtonIvars` states.
                let ivars = unsafe { crate::defined_class::raw_ivars(self) };
                emit(&ivars.enabled_handler, enabled);
            });
        }
    }
);

impl CocoaUiButton {
    /// A button of `button_type` — `+[UIButton buttonWithType:]` invoked
    /// on this subclass.
    ///
    /// The factory's own `alloc` zero-fills the ivar storage, which is
    /// exactly the state `CocoaUiButtonIvars` starts in, so no `set_ivars`
    /// step is needed — only `objc2`'s initialized marker.
    #[must_use]
    pub fn with_type(mtm: MainThreadMarker, button_type: UIButtonType) -> Retained<Self> {
        let _ = mtm;
        // SAFETY: invoked on the subclass itself, `buttonWithType:`
        // allocates through `self`, so the result is a `CocoaUiButton`.
        let this: Retained<Self> = unsafe { msg_send![Self::class(), buttonWithType: button_type] };
        crate::defined_class::mark_ivars_initialized(&*this);
        this
    }

    /// A button through `-[UIButton init]` — the plain initializer the
    /// `UIButton::new` sites share.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(CocoaUiButtonIvars {
            enabled_handler: RefCell::new(None),
        });
        // SAFETY: `init` is `UIView`'s designated-initializer chain.
        unsafe { msg_send![super(this), init] }
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
