//! The `UIKit` button: a `UIButton` of `.system` type driven by a
//! `UIButtonConfiguration` for its chrome.
//!
//! The configuration sets the button's chrome (bordered / glass / plain);
//! `Button` adds what a configuration cannot express: the label's content
//! insets, tint, enabled state and accessibility hooks.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSAttributedString, NSString};
use objc2_ui_kit::{
    NSDirectionalEdgeInsets, UIButton, UIButtonConfiguration, UIColor, UIControl, UIControlState,
};

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

/// An interactive button backed by `UIButton` (`.system` type).
///
/// Derefs to `UIButton` so callers can reach platform APIs the wrapper does
/// not expose; prefer the wrapper's methods where they exist.
#[derive(Debug, Clone)]
pub struct Button {
    inner: Retained<UIButton>,
}

impl Button {
    /// A `.system`-type button with no title.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            inner: UIButton::buttonWithType(objc2_ui_kit::UIButtonType::System, mtm),
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
