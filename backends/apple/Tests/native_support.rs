//! Harness-only reach into the embedding contract.
//!
//! Compiled into the crate only behind the non-default
//! `native-test-support` feature and pulled in through
//! `#[path]` in `lib.rs`, so `native` test binaries can assert the
//! private window lifecycle without widening the public API. Each entry
//! takes the harness's real `MainThreadMarker` — the cases build real
//! `NSWindow`s, which only the process's true main thread may.

use cocoa_ui::MainThreadMarker;
use waterui::window::WindowManager;
use waterui_backend_core::Environment;

/// A real controller/window lifetime around the production mounting path.
#[cfg(target_os = "ios")]
#[derive(Debug)]
pub struct UIKitMount {
    pub host: cocoa_ui::Retained<cocoa_ui::uikit::HostView>,
    pub content: alloc::rc::Rc<crate::contract::Mounted>,
    pub window: cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIWindow>,
    _controller: cocoa_ui::Retained<cocoa_ui::uikit::ViewController>,
    _keepalive: crate::contract::KeepAlive,
}

#[cfg(target_os = "ios")]
impl Drop for UIKitMount {
    fn drop(&mut self) {
        self.window.setHidden(true);
        self.window.setRootViewController(None);
    }
}

/// Hosts real content through embedding's shared mount/primary-content/layout path.
#[cfg(target_os = "ios")]
#[expect(
    deprecated,
    reason = "the native test process supplies its own window without a scene"
)]
#[must_use]
pub fn mount_uikit(
    mtm: MainThreadMarker,
    view: waterui::AnyView,
    env: &Environment,
    frame: cocoa_ui::Rect,
) -> UIKitMount {
    use objc2::{MainThreadOnly, Message};
    let controller = cocoa_ui::uikit::ViewController::new(mtm);
    let host = controller.host_view().retain();
    let window = cocoa_ui::objc2_ui_kit::UIWindow::initWithFrame(
        cocoa_ui::objc2_ui_kit::UIWindow::alloc(mtm),
        frame.into(),
    );
    window.setRootViewController(Some(&controller));
    cocoa_ui::view::set_frame(&host, frame);
    let mut keepalive = crate::contract::KeepAlive::default();
    let mut env = env.clone();
    crate::theme::install_controller(&mut env, &controller, &mut keepalive);
    let content = crate::embedding::mount_content(&host, view, &env, &mut keepalive);
    window.makeKeyAndVisible();
    window.layoutIfNeeded();
    UIKitMount {
        host,
        content,
        window,
        _controller: controller,
        _keepalive: keepalive,
    }
}

#[cfg(target_os = "macos")]
use cocoa_ui::Retained;
#[cfg(target_os = "macos")]
use waterui::Signal;
#[cfg(target_os = "macos")]
use waterui::Str;
#[cfg(target_os = "macos")]
use waterui::color::Color;
#[cfg(target_os = "macos")]
use waterui::reactive::{Binding, Computed, SignalExt, binding};
#[cfg(target_os = "macos")]
use waterui::window::{UserAttention, WindowBackground, WindowLevel, WindowState, WindowStyle};
#[cfg(target_os = "macos")]
use waterui_core::layout::{Point, Rect, Size};

/// Checks that native service installation provides a window manager.
///
/// Uses the embedding runtime's installer after installing the dispatcher.
///
/// # Panics
/// Panics if service installation does not provide a window manager.
pub fn manager_installs_into_the_environment(_mtm: MainThreadMarker) {
    let mut env = Environment::new();
    crate::dispatch::install(&mut env);
    crate::embedding::install_services(&mut env);
    assert!(env.get::<WindowManager>().is_some());
}

/// Checks two-way bindings on a real native root window.
///
/// `bind_root_window` adopts a window the host already created: the
/// declared style and title land on it, the real frame publishes into
/// the binding, a declared frame drives the window back, and platform
/// close publishes `Closed`. The window is never ordered in — the whole
/// lifecycle stays offscreen.
///
/// # Panics
/// Panics if the native window cannot be retained or a binding assertion fails.
#[cfg(target_os = "macos")]
pub fn bind_root_window_wires_a_live_window(mtm: MainThreadMarker) {
    let window = cocoa_ui::appkit::Window::new(
        mtm,
        cocoa_ui::Rect::new(100.0, 100.0, 640.0, 480.0),
        cocoa_ui::appkit::WindowStyle::TITLED,
    );

    let mut env = Environment::new();
    crate::dispatch::install(&mut env);
    crate::embedding::install_services(&mut env);
    let title: Computed<Str> = binding(Str::from("Bind Root")).computed();
    let frame: Binding<Rect> = binding(Rect::new(Point::new(0.0, 0.0), Size::new(0.0, 0.0)));
    let state: Binding<WindowState> = binding(WindowState::Normal);
    let style: Computed<WindowStyle> = binding(WindowStyle::Titled).computed();
    let level: Computed<WindowLevel> = binding(WindowLevel::Normal).computed();
    let attention: Binding<Option<UserAttention>> = binding(None);
    let background: Computed<WindowBackground> =
        binding(WindowBackground::Color(Color::srgb(255, 255, 255))).computed();

    // SAFETY: `window.native()` is a live +0 object; the binding retains it
    // for the declaration's lifetime on the main thread.
    let native = unsafe {
        Retained::retain(std::ptr::from_ref(window.native()).cast_mut())
            .expect("a live NSWindow retains")
    };
    let binding = crate::windows::bind_root_window(
        native,
        &env,
        &title,
        &frame,
        &state,
        None,
        &style,
        &level,
        &attention,
        None,
        &background,
        true,
        true,
        mtm,
    );

    // The declared style was adopted on top of the kit window's mask.
    let mask = window.style_mask();
    assert!(mask.contains(
        cocoa_ui::appkit::WindowStyle::TITLED
            | cocoa_ui::appkit::WindowStyle::CLOSABLE
            | cocoa_ui::appkit::WindowStyle::MINIATURIZABLE
            | cocoa_ui::appkit::WindowStyle::RESIZABLE
    ));

    // The declared title landed on the window.
    assert_eq!(window.native().title().to_string(), "Bind Root");

    // The window's real frame seeded the frame binding (outer-frame
    // convention — the host's position on screen wins over the declared
    // origin).
    let snapshot = frame.snapshot();
    let current = window.frame();
    let expected = cocoa_ui::Rect::new(
        f64::from(snapshot.origin().x),
        f64::from(snapshot.origin().y),
        f64::from(snapshot.size().width),
        f64::from(snapshot.size().height),
    );
    assert_eq!(current, expected);

    // A declared frame write drives the real window.
    frame.set(Rect::new(Point::new(20.0, 30.0), Size::new(320.0, 240.0)));
    let moved = window.frame();
    assert_eq!(moved, cocoa_ui::Rect::new(20.0, 30.0, 320.0, 240.0));

    // Platform close publishes `Closed` through the binding — the
    // window's own lifecycle event driving the declared state.
    window.close();
    assert_eq!(state.snapshot(), WindowState::Closed);
    window.close();
    assert_eq!(state.snapshot(), WindowState::Closed);

    // Dropping the binding releases the window's declaration — the
    // owned-ABI replacement for a free function over a raw handle.
    drop(binding);
}
