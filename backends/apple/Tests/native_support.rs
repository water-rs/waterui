//! Harness-only reach into the embedding contract.
//!
//! Compiled into the crate only behind the non-default
//! `native-test` feature and pulled in through
//! `#[path]` in `lib.rs`, so `native` test binaries can assert the
//! private window lifecycle without widening the public API. Each entry
//! takes the harness's real `MainThreadMarker` — the cases build real
//! `NSWindow`s, which only the process's true main thread may.

use cocoa_ui::MainThreadMarker;
use waterui::window::WindowManager;
use waterui_backend_core::Environment;
use waterui_backend_core::scroll::ANIMATED_ROW_SCROLL_APPROACH;

pub use cocoa_ui::native_test::pump_main_until;

/// The bound a case gives deferred main-queue work before it fails.
///
/// Reached only when the awaited work never arrives; a healthy queue
/// answers within a few run-loop turns.
pub const MAIN_QUEUE_DEADLINE: f64 = 5.0;

/// The least wall-clock time a platform `Animation::Default` scroll
/// takes from its request to its landing — well under `AppKit`'s 0.25 s
/// group and `UIKit`'s own scroll animation. A jump lands at once; a
/// starved main thread only lengthens a native animation.
pub const NATIVE_SCROLL_MIN_DURATION: std::time::Duration = std::time::Duration::from_millis(150);

/// Rows in the list the approach cases scroll — well over the
/// [`ANIMATED_ROW_SCROLL_APPROACH`] bound.
pub const APPROACH_LIST_ROWS: usize = 400;

/// The row the approach cases animate to from the top — further than
/// the bound, so the request first jumps to [`APPROACH_ROW`].
pub const APPROACH_TARGET_ROW: usize = 180;

/// The row an animated request toward [`APPROACH_TARGET_ROW`] from the
/// top jumps to before it animates.
pub const APPROACH_ROW: usize = APPROACH_TARGET_ROW - ANIMATED_ROW_SCROLL_APPROACH;

/// Asserts that `request` starts a platform-timed scroll, not a jump.
///
/// [`assert_native_scroll_from`] with the offset before the request as
/// the start: the request itself must not move it.
pub fn assert_native_scroll(
    what: &str,
    request: impl FnOnce(),
    offset: impl Fn() -> cocoa_ui::Point,
    landing: impl Fn() -> cocoa_ui::Point,
) {
    let start = offset();
    assert_native_scroll_from(what, request, || start, offset, landing);
}

/// Asserts that `request` starts a platform-timed scroll from `from`.
///
/// Right after `request` returns, before any run-loop turn, `offset`
/// reads `from` exactly — the start itself, or where a list's approach
/// jump put it. Pumping the main run loop then lands it within one device
/// pixel of `landing` — read on every pass, so a target the layout
/// resolves during the scroll is compared as it stands when the offset
/// lands — no sooner than [`NATIVE_SCROLL_MIN_DURATION`] after the
/// request. The platform runs the animation on its wall clock, so no
/// intermediate sample is required: a starved main thread may see none,
/// and can only lengthen the measured duration.
pub fn assert_native_scroll_from(
    what: &str,
    request: impl FnOnce(),
    from: impl Fn() -> cocoa_ui::Point,
    offset: impl Fn() -> cocoa_ui::Point,
    landing: impl Fn() -> cocoa_ui::Point,
) {
    #[cfg(target_os = "macos")]
    let pixel = 1.0 / cocoa_ui::appkit::main_screen_scale();
    #[cfg(target_os = "ios")]
    let pixel = 1.0 / cocoa_ui::uikit::main_screen_scale();
    let distance =
        |a: cocoa_ui::Point, b: cocoa_ui::Point| (a.x - b.x).abs().max((a.y - b.y).abs());
    let requested = std::time::Instant::now();
    request();
    let after_request = offset();
    let start = from();
    assert!(
        after_request.x.to_bits() == start.x.to_bits()
            && after_request.y.to_bits() == start.y.to_bits(),
        "{what}: the request left the offset at {after_request:?} before any run-loop turn, not at the start {start:?}"
    );
    assert!(
        distance(start, landing()) >= pixel,
        "{what}: the start {start:?} is already at the landing {:?}",
        landing()
    );
    let landed = pump_main_until(MAIN_QUEUE_DEADLINE, || {
        distance(offset(), landing()) < pixel
    });
    let took = requested.elapsed();
    assert!(
        landed,
        "{what}: the offset never reached {:?} from {start:?}; it stayed at {:?}",
        landing(),
        offset()
    );
    assert!(
        took >= NATIVE_SCROLL_MIN_DURATION,
        "{what}: landed {took:?} after the request, under the {NATIVE_SCROLL_MIN_DURATION:?} a native animation takes"
    );
}

/// Pumps the main run loop until every block already on the main queue has run.
///
/// The backend applies a list emission as a block on the main dispatch
/// queue, so a case asserting that the newest state is never overwritten
/// cannot stop the moment that state first appears: an older emission
/// still queued would land in a later turn. This enqueues a sentinel
/// block and pumps until it runs; the main queue is FIFO, so every block
/// enqueued before it has run too. Answers whether the sentinel ran
/// within [`MAIN_QUEUE_DEADLINE`].
#[must_use = "a queue that never drains must fail the case"]
pub fn drain_main_queue(mtm: MainThreadMarker) -> bool {
    let drained = alloc::rc::Rc::new(core::cell::Cell::new(false));
    cocoa_ui::main_queue::enqueue_local(mtm, {
        let drained = alloc::rc::Rc::clone(&drained);
        move |_mtm| drained.set(true)
    });
    pump_main_until(MAIN_QUEUE_DEADLINE, || drained.get())
}

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
    let controller = cocoa_ui::uikit::ViewController::new(mtm, cocoa_ui::uikit::window_root(mtm));
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

/// Re-exports the GPU-surface mounted-scene fixtures.
///
/// The `native_test` module inside `components::gpu_surface` builds a real
/// `SceneView` mount and drives the production failure drain and
/// completion settlement paths on it.
#[cfg(all(target_os = "macos", feature = "gpu_surface"))]
pub mod gpu_surface {
    pub use crate::components::gpu_surface::native_test::{MountedSceneSurface, WakeProbe};

    /// Performs the once-per-process `startup::initialize` — called
    /// once from the `Tests/native.rs` harness's true main thread
    /// before any trial runs; fixture mounts rely on that explicit
    /// harness setup.
    pub fn initialize_process() {
        let _ = crate::startup::initialize();
    }
}

/// The minimum environment a real render needs: `dispatch::install`
/// performs the backend's half of the embedding contract (dispatcher,
/// window manager, realizations); the theme slots text resolves
/// through are the framework's. Shared by the `native` and `native_app`
/// harnesses.
#[must_use]
pub fn render_environment() -> Environment {
    use waterui::graphics::color::WorkingColor;
    use waterui::reactive::{SignalExt, binding};
    use waterui::text::font::{Body, Caption, FontSlot, Subheadline};

    let mut env = Environment::new();
    crate::dispatch::install(&mut env);
    waterui::theme::install_color_scheme(
        &mut env,
        binding(waterui::theme::ColorScheme::Light).computed(),
    );
    let black = || binding(WorkingColor::BLACK).computed();
    waterui::theme::install_color_signal::<waterui::theme::color::Foreground>(&mut env, black());
    // The richer fixtures (list rows, stacked text) resolve muted and
    // accent roles plus the caption/subheadline slots — install them so
    // a theme miss can't masquerade as a render failure.
    waterui::theme::install_color_signal::<waterui::theme::color::MutedForeground>(
        &mut env,
        black(),
    );
    waterui::theme::install_color_signal::<waterui::theme::color::Accent>(&mut env, black());
    waterui::theme::install_font_signal::<Body>(&mut env, binding(Body::DEFAULT).computed());
    waterui::theme::install_font_signal::<Caption>(&mut env, binding(Caption::DEFAULT).computed());
    waterui::theme::install_font_signal::<Subheadline>(
        &mut env,
        binding(Subheadline::DEFAULT).computed(),
    );
    env
}

/// Renders `view` against [`render_environment`] through the typed
/// dispatch entry point.
#[must_use]
pub fn render(view: impl waterui_backend_core::View) -> crate::contract::NativeLeaf {
    crate::dispatch::render(
        waterui_backend_core::AnyView::new(view),
        &render_environment(),
    )
}

/// The kit view a `ScrollView` renders into.
#[cfg(target_os = "macos")]
pub type ScrollSurface = cocoa_ui::appkit::ScrollView;
/// The kit view a `ScrollView` renders into.
#[cfg(target_os = "ios")]
pub type ScrollSurface = cocoa_ui::uikit::ScrollView;
/// The kit view a `List` renders into.
#[cfg(target_os = "macos")]
pub type ListSurface = cocoa_ui::appkit::ListTableView;
/// The kit view a `List` renders into.
#[cfg(target_os = "ios")]
pub type ListSurface = cocoa_ui::uikit::TableView;

/// The offset that puts `row`'s top edge at the viewport's top, before
/// any clamp — `rectOfRow`'s origin on `AppKit`; `rectForRowAtIndexPath`'s
/// origin in section 0 minus the adjusted top inset on `UIKit`, read as
/// the table stands now, since `UIKit` sizes unseen rows by estimate.
#[must_use]
pub fn list_row_top(table: &ListSurface, row: usize) -> cocoa_ui::Point {
    #[cfg(target_os = "macos")]
    {
        cocoa_ui::Point::new(0.0, table.rect_of_row(row).origin.y)
    }
    #[cfg(target_os = "ios")]
    {
        use cocoa_ui::objc2_ui_kit::NSIndexPathUIKitAdditions;
        let index = cocoa_ui::objc2_foundation::NSIndexPath::indexPathForRow_inSection(
            isize::try_from(row).expect("the row fits an NSInteger"),
            0,
        );
        cocoa_ui::Point::new(
            0.0,
            table.rectForRowAtIndexPath(&index).origin.y - table.adjustedContentInset().top,
        )
    }
}

/// The scroll suites' surface, rendered but not mounted: a vertical
/// scroll over a 2000pt document, driven by `controller` and reporting its
/// offset into `offset` — with the leaf that owns its watchers.
#[must_use]
pub fn scroll_surface(
    controller: &waterui::layout::scroll::ScrollController<waterui_core::layout::Point>,
    offset: &waterui::reactive::Binding<waterui_core::layout::Point>,
) -> (
    crate::contract::NativeLeaf,
    cocoa_ui::Retained<ScrollSurface>,
) {
    use objc2::Message;
    use waterui::layout::frame::Frame;
    use waterui::layout::scroll::scroll;
    use waterui::prelude::text;

    let leaf = render(
        scroll(Frame::new(text("document")).height(2000.0))
            .scroll_controller(controller)
            .report_offset(offset),
    );
    let surface = leaf
        .view()
        .downcast_ref::<ScrollSurface>()
        .expect("a ScrollView renders the kit's scroll surface")
        .retain();
    (leaf, surface)
}

/// The list suites' row body — identical one-line rows; the cases assert
/// positions, not labels.
#[must_use]
pub fn row_item() -> waterui::component::list::ListItem {
    waterui::component::list::ListItem::new(waterui::prelude::text("row"))
}

/// The list suites' table, rendered but not mounted: `rows` in a list
/// driven by `controller`, with the leaf that owns its wiring.
#[must_use]
pub fn row_list(
    rows: Vec<fn() -> waterui::component::list::ListItem>,
    controller: &waterui::layout::scroll::ScrollController<usize>,
) -> (crate::contract::NativeLeaf, cocoa_ui::Retained<ListSurface>) {
    use objc2::Message;

    let leaf = render(waterui::component::list::List::content(rows).scroll_controller(controller));
    let table = leaf
        .view()
        .downcast_ref::<ListSurface>()
        .expect("a List renders the kit's table surface")
        .retain();
    (leaf, table)
}
