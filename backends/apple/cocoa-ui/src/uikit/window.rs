//! Windows shown in a window scene.

use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_ui_kit::{UIView, UIViewController, UIWindow};

use super::application::WindowScene;

/// A window, shown in the scene it was created for.
#[derive(Debug)]
pub struct Window {
    window: Retained<UIWindow>,
}

impl Window {
    /// A hidden window filling `scene`.
    #[must_use]
    pub fn new(scene: &WindowScene) -> Self {
        let scene = scene.native();
        Self {
            window: UIWindow::initWithWindowScene(UIWindow::alloc(scene.mtm()), scene),
        }
    }

    /// Makes `controller` the window's root, whose view fills the window.
    pub fn set_root_view_controller(&self, controller: &UIViewController) {
        self.window.setRootViewController(Some(controller));
    }

    /// Shows the window and makes it the key window of its scene.
    pub fn make_key_and_visible(&self) {
        self.window.makeKeyAndVisible();
    }

    /// Runs any pending layout pass of the window's views now.
    pub fn layout_if_needed(&self) {
        self.window.layoutIfNeeded();
    }

    pub(super) fn into_native(self) -> Retained<UIWindow> {
        self.window
    }
}

/// The window `view` currently lives in, if any.
#[must_use]
pub fn window_of(view: &UIView) -> Option<Retained<UIWindow>> {
    view.window()
}

/// The main screen's scale — the display scale an off-window view
/// rasterizes at.
#[must_use]
/// # Panics
///
/// On a failure to confirm the main thread.
#[allow(deprecated)]
pub fn main_screen_scale() -> f64 {
    objc2_ui_kit::UIScreen::mainScreen(objc2::MainThreadMarker::new().expect("main thread")).scale()
}

/// Whether the application is in the active state —
/// `UIApplication.shared.applicationState == .active`.
#[must_use]
/// # Panics
///
/// On a failure to reach `UIApplication.sharedApplication`.
pub fn application_is_active() -> bool {
    let mtm = objc2::MainThreadMarker::new().expect("main thread");
    objc2_ui_kit::UIApplication::sharedApplication(mtm).applicationState()
        == objc2_ui_kit::UIApplicationState::Active
}
