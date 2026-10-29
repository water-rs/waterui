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

/// `pump_main_until` for the `AppKit` harnesses.
#[cfg(target_os = "macos")]
pub use cocoa_ui::native_test::pump_main_until;
/// The pump a case inside the harness application waits on: on iOS it is
/// `uikit::native_test`'s — under Catalyst that is a real `AppKit`
/// `nextEvent:`/`sendEvent:` turn, the only dispatch route `UIKit` has
/// (water-rs/waterui#3358) — while `AppKit` keeps the bare run-loop turn.
#[cfg(target_os = "ios")]
pub use cocoa_ui::uikit::native_test::pump_main_until;

/// The bound a case gives deferred main-queue work before it fails.
///
/// Reached only when the awaited work never arrives; a healthy queue
/// answers within a few run-loop turns.
pub const MAIN_QUEUE_DEADLINE: f64 = 5.0;

/// Drives a `!Send` future to completion on the main thread while
/// pumping the run loop.
///
/// A bare `block_on` parks the thread it polls on; the central
/// capture's completions and wakes land on `DispatchQueue::main()`,
/// which only a turning run loop services — so the future's polls
/// interleave with short `pump_main_until` turns, re-polling on the
/// wake flag as soon as a callback delivers. Bounded at `seconds`
/// overall: a future that never settles panics naming the bound
/// instead of hanging the case.
///
/// # Panics
///
/// When `future` is not ready within `seconds`.
pub fn block_on_main<F: core::future::Future>(seconds: f64, future: F) -> F::Output {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::Instant;

    struct Flag(Arc<AtomicBool>);
    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::Release);
        }
    }

    let woken = Arc::new(AtomicBool::new(false));
    let waker = Waker::from(Arc::new(Flag(Arc::clone(&woken))));
    let mut cx = Context::from_waker(&waker);
    let mut future = core::pin::pin!(future);
    let start = Instant::now();
    loop {
        woken.store(false, Ordering::Release);
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        assert!(
            start.elapsed().as_secs_f64() < seconds,
            "a main-thread future did not settle within {seconds}s"
        );
        pump_main_until(0.05, || woken.load(Ordering::Acquire));
    }
}

/// The least wall-clock time a native `Animation::Default` scroll takes.
///
/// Measured from the request to the landing, it is well under `AppKit`'s
/// 0.25 s group and `UIKit`'s own scroll animation. A jump lands at once;
/// a starved main thread only lengthens a native animation.
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
    controller: cocoa_ui::Retained<cocoa_ui::uikit::ViewController>,
    _keepalive: crate::contract::KeepAlive,
}

#[cfg(target_os = "ios")]
impl Drop for UIKitMount {
    fn drop(&mut self) {
        self.host.clear_handlers();
        // Whether the window shows in a window scene — the same read
        // `mount_uikit_window` gated its appearance wait on, taken before
        // the window hides. A scene-less window releases its root at once,
        // as it always has.
        let scene_bound = self.window.windowScene().is_some();
        self.window.setHidden(true);
        // Hiding alone delivers no disappearance; releasing the root does,
        // ending on a later run-loop turn. The mount still owns the
        // controller, so it lives until that disappearance has ended.
        self.window.setRootViewController(None);
        if scene_bound {
            assert!(
                pump_main_until(MAIN_QUEUE_DEADLINE, || {
                    let counts = self.controller.appearance_counts();
                    counts.did_disappear >= counts.did_appear
                }),
                "the root controller's viewDidDisappear: did not run within \
                 {MAIN_QUEUE_DEADLINE}s of releasing it: {:?}",
                self.controller.appearance_counts()
            );
        }
    }
}

/// The scene-less `UIWindow` the spawned test process mounts into.
///
/// `initWithFrame:` is the only construction outside a window scene —
/// the spawned test binary owns no scene — so this is the one site the
/// deprecation is expected at. The window takes the screen's bounds, so
/// its ambient `safeAreaInsets` — the notch band above, the
/// home-indicator band below — are the real ones a `WaterUI` app sees.
#[cfg(target_os = "ios")]
#[expect(
    deprecated,
    reason = "the native test process supplies its own window without a scene"
)]
#[must_use]
pub fn spawned_window(
    mtm: MainThreadMarker,
) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIWindow> {
    use objc2::MainThreadOnly;
    let window = cocoa_ui::objc2_ui_kit::UIWindow::initWithFrame(
        cocoa_ui::objc2_ui_kit::UIWindow::alloc(mtm),
        cocoa_ui::objc2_core_foundation::CGRect::ZERO,
    );
    window.setFrame(window.screen().bounds());
    window
}

/// Hosts real content through embedding's shared mount/primary-content/layout path.
#[cfg(target_os = "ios")]
#[must_use]
pub fn mount_uikit(mtm: MainThreadMarker, view: waterui::AnyView, env: &Environment) -> UIKitMount {
    mount_uikit_window(mtm, view, env, spawned_window(mtm))
}

/// `mount_uikit` into a window the caller already owns — a trial that
/// must reach the window before the first mount (the keyboard can be
/// up first) uses this.
#[cfg(target_os = "ios")]
#[must_use]
pub fn mount_uikit_window(
    mtm: MainThreadMarker,
    view: waterui::AnyView,
    env: &Environment,
    window: cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIWindow>,
) -> UIKitMount {
    use objc2::Message;
    let controller = cocoa_ui::uikit::ViewController::new(mtm, cocoa_ui::uikit::window_root(mtm));
    let host = controller.host_view().retain();
    window.setRootViewController(Some(&controller));
    cocoa_ui::view::set_frame(&host, cocoa_ui::view::bounds(&window));
    let mut keepalive = crate::contract::KeepAlive::default();
    let mut env = env.clone();
    crate::theme::install_controller(&mut env, &controller, &mut keepalive);
    let content = crate::embedding::mount_content(&host, view, &env, &mut keepalive);
    window.makeKeyAndVisible();
    window.layoutIfNeeded();
    // A scene window opens an appearance transition only the run loop
    // closes: `viewWillAppear:` lands inside `makeKeyAndVisible`, but
    // `viewDidAppear:` waits for a later turn. Returning before it runs
    // hands the case a mid-transition controller whose drop would cut
    // the transition short. A scene-less window — the bare harness's
    // spawned window — keeps the immediate return.
    if window.windowScene().is_some() {
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                controller.appearance_counts().did_appear >= 1
            }),
            "the root controller's viewDidAppear: did not run within \
             {MAIN_QUEUE_DEADLINE}s of showing its scene window"
        );
    }
    UIKitMount {
        host,
        content,
        window,
        controller,
        _keepalive: keepalive,
    }
}

/// `mount_uikit` at a fixed window frame — trials that assert geometry
/// inside a window sized for the case rather than the screen's.
#[cfg(target_os = "ios")]
#[must_use]
pub fn mount_uikit_in(
    mtm: MainThreadMarker,
    view: waterui::AnyView,
    env: &Environment,
    frame: cocoa_ui::Rect,
) -> UIKitMount {
    let mount = mount_uikit(mtm, view, env);
    mount.window.setFrame(frame.into());
    cocoa_ui::view::set_frame(&mount.host, frame);
    mount.window.layoutIfNeeded();
    mount
}

/// A kit subtree embedded under a foreign window root.
///
/// The window's root view controller is a plain `UIViewController` and
/// the `HostView` sits inside a plain `UIView` container — every host
/// in the window shares the window's own keyboard region regardless.
#[cfg(target_os = "ios")]
#[derive(Debug)]
pub struct EmbeddedMount {
    /// The embedded host.
    pub host: cocoa_ui::Retained<cocoa_ui::uikit::HostView>,
    /// The foreign-rooted window holding the subtree.
    pub window: cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIWindow>,
    /// The mounted content inside `host`.
    pub content: alloc::rc::Rc<crate::contract::Mounted>,
    _keepalive: crate::contract::KeepAlive,
}

#[cfg(target_os = "ios")]
impl Drop for EmbeddedMount {
    fn drop(&mut self) {
        self.window.setHidden(true);
        self.window.setRootViewController(None);
    }
}

/// Mounts `view` into an embedded `HostView` under a foreign window
/// root in either content-mount order.
///
/// `content_first` mounts the content while the host is still detached
/// and only then reaches the window; otherwise the host joins the
/// already-windowed container first and mounts after — proving the
/// window's keyboard region answers the same either way.
#[cfg(target_os = "ios")]
#[must_use]
pub fn mount_uikit_embedded(
    mtm: MainThreadMarker,
    view: waterui::AnyView,
    env: &Environment,
    content_first: bool,
) -> EmbeddedMount {
    use cocoa_ui::objc2_ui_kit::{UIView, UIViewController};
    use cocoa_ui::uikit::HostView;
    use objc2::{MainThreadOnly, msg_send};

    // SAFETY: `init` is `UIViewController`'s plain designated initializer.
    let foreign: cocoa_ui::Retained<UIViewController> =
        unsafe { msg_send![UIViewController::alloc(mtm), init] };
    let window = spawned_window(mtm);
    let container = UIView::initWithFrame(UIView::alloc(mtm), window.screen().bounds());
    let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
    let mut view = Some(view);
    let mut keepalive = crate::contract::KeepAlive::default();
    let mut mount = |host: &_, env: &_, keepalive: &mut _| {
        crate::embedding::mount_content(
            host,
            view.take().expect("the view mounts once"),
            env,
            keepalive,
        )
    };
    let content = content_first.then(|| mount(&host, env, &mut keepalive));
    window.setRootViewController(Some(&foreign));
    foreign
        .view()
        .expect("a root view controller creates its view")
        .addSubview(&container);
    container.addSubview(&host);
    cocoa_ui::view::set_frame(&host, cocoa_ui::view::bounds(&container));
    let content = content.unwrap_or_else(|| mount(&host, env, &mut keepalive));
    window.makeKeyAndVisible();
    window.layoutIfNeeded();
    EmbeddedMount {
        host,
        window,
        content,
        _keepalive: keepalive,
    }
}

#[cfg(target_os = "macos")]
use waterui::Signal;
#[cfg(target_os = "macos")]
use waterui::color::Color;
#[cfg(target_os = "macos")]
use waterui::reactive::{Binding, binding};
#[cfg(target_os = "macos")]
use waterui::window::{WindowBackground, WindowState, WindowStyle};
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

/// The environment `embedding` prepares for a window's mount on macOS.
///
/// `create_root` installs the dispatcher and the embedding services and
/// then the platform theme, resolved from the shared application's
/// current scheme — a window background resolves the theme's
/// `Background` token through the environment, so a bare
/// `Environment::new()` cannot drive a window binding. Answers the
/// environment plus the theme signals: keep them alive for the window's
/// lifetime, the way a real mount's keepalive does.
#[cfg(target_os = "macos")]
#[must_use]
pub fn window_environment(mtm: MainThreadMarker) -> (Environment, crate::theme::ThemeSignals) {
    let mut env = Environment::new();
    crate::dispatch::install(&mut env);
    crate::embedding::install_services(&mut env);
    let theme = crate::theme::install(
        &mut env,
        cocoa_ui::appkit::Application::shared(mtm).color_scheme(),
    );
    (env, theme)
}

/// Re-reads every theme slot for `scheme`, as an appearance change does.
#[cfg(target_os = "macos")]
pub fn refresh_theme(theme: &crate::theme::ThemeSignals, scheme: cocoa_ui::ColorScheme) {
    crate::theme::refresh(theme, scheme);
}

/// Checks two-way bindings on a real native root window.
///
/// `windows::realize` turns a declaration into a live window: the
/// declared style and title land on it, the declared frame seeds the
/// window's content rect, platform close publishes `Closed` through the
/// state binding, and the armed `on_close_request` gate fires.
///
/// `realize` orders the window in — this trial opens and closes a real
/// window, so it runs only on a host where a window may appear on a
/// display.
///
/// # Panics
/// Panics if the window cannot be realized or an assertion fails.
#[cfg(target_os = "macos")]
pub fn a_declared_window_realizes_its_declaration(mtm: MainThreadMarker) {
    let (mut env, _theme) = window_environment(mtm);
    env.insert(crate::first_paint::FirstPaint::default());
    let state: Binding<WindowState> = binding(WindowState::Normal);
    let closed = alloc::rc::Rc::new(core::cell::RefCell::new(false));
    let declaration = waterui::window::Window::new("Bind Root", state.clone(), || {
        waterui::component::text::Text::new("x")
    })
    .style(WindowStyle::Titled)
    .background(WindowBackground::Color(Color::srgb(255, 255, 255)))
    .on_close_request({
        let closed = alloc::rc::Rc::clone(&closed);
        move || {
            let closed = alloc::rc::Rc::clone(&closed);
            async move {
                *closed.borrow_mut() = true;
                waterui::window::CloseReply::Close
            }
        }
    });

    declaration
        .frame
        .set(Rect::new(Point::new(0.0, 0.0), Size::new(640.0, 480.0)));
    let host = crate::windows::realize(declaration, &env, mtm);
    let window = host.window();
    let native = window.native();

    // The declared style lands on the window's mask.
    let mask = window.style_mask();
    assert!(mask.contains(
        cocoa_ui::appkit::WindowStyle::TITLED
            | cocoa_ui::appkit::WindowStyle::CLOSABLE
            | cocoa_ui::appkit::WindowStyle::MINIATURIZABLE
            | cocoa_ui::appkit::WindowStyle::RESIZABLE
    ));

    // The declared title lands on the window.
    assert_eq!(native.title().to_string(), "Bind Root");

    // The declared frame seeds the window's content size — the platform
    // positions the chrome around it.
    let content = native.contentRectForFrameRect(native.frame());
    assert_eq!(
        cocoa_ui::Size::new(content.size.width, content.size.height),
        cocoa_ui::Size::new(640.0, 480.0)
    );

    // The armed close gate intercepts the platform close — `AppKit` does
    // not close outright — and the handler's `Close` reply publishes
    // `Closed` through the state binding.
    window.native().performClose(None);
    assert!(drain_main_queue(mtm));
    assert!(*closed.borrow());
    assert_eq!(state.snapshot(), WindowState::Closed);

    drop(host);
}

/// §7.1 on a real window root: a navigation page's scroll surface runs
/// under the window toolbar, its other content stays below the titlebar.
///
/// A navigation page touches the edge the window-drawn bar sits on, so on
/// a titled window — unified toolbar, full-size content view — a scroll
/// surface on it reaches the window's top edge, while non-scrolling
/// content in the same position stays clear of the titlebar.
///
/// Every case realizes its content through `windows::realize`, the
/// production window root, so each link the safe-area answer passes —
/// the root's `content_frame`, the tab host, the tab's `NavigationView`
/// pane, the stack, the page — is the one an application gets. Each case
/// orders a window in.
#[cfg(target_os = "macos")]
pub mod toolbar_reach {
    use cocoa_ui::objc2_app_kit::{NSTextField, NSView, NSWindowStyleMask, NSWindowToolbarStyle};
    use cocoa_ui::{MainThreadMarker, Retained};
    use waterui::Str;
    use waterui::ViewExt as _;
    use waterui::component::list::{List, ListItem};
    use waterui::id::SelfId;
    use waterui::navigation::{
        NavigationLink, NavigationPath, NavigationStack, NavigationView, Tab, Tabs, tab_style,
    };
    use waterui::prelude::text;
    use waterui::reactive::binding;
    use waterui::window::{WindowState, WindowStyle};
    use waterui_core::layout::{Point, Rect, Size};

    use super::{ListSurface, MAIN_QUEUE_DEADLINE, pump_main_until, row_item, window_environment};

    /// The text the non-scrolling fixtures show — the label the case
    /// finds in the window.
    const MARKER: &str = "toolbar-reach-marker";

    /// Half a point: the distance at which an edge counts as reaching a
    /// boundary at any backing scale.
    const TOLERANCE: f64 = 0.5;

    /// The gap `SwiftUI` leaves between the safe-area top and a list's
    /// first row.
    const LIST_GAP: f64 = 10.0;

    /// How far the first row may stray from [`LIST_GAP`] below the
    /// titlebar.
    const LIST_GAP_TOLERANCE: f64 = 2.0;

    /// The window-coordinate tops a case measures against.
    struct Edges {
        /// The top of the window, which a full-size content view reaches.
        window: f64,
        /// The top of the content view's safe-area rect — the bottom of
        /// the titlebar, its toolbar and any accessory row.
        safe: f64,
    }

    /// The top edge of `rect`, a rect in `view`'s coordinates, in window
    /// coordinates.
    fn top_in_window(view: &NSView, rect: cocoa_ui::objc2_foundation::NSRect) -> f64 {
        let frame = view.convertRect_toView(rect, None);
        frame.origin.y + frame.size.height
    }

    /// The first view under `root`, depth-first, that `matches` accepts.
    fn find<T: objc2::DowncastTarget>(
        root: &NSView,
        matches: &impl Fn(&T) -> bool,
    ) -> Option<Retained<T>> {
        for sub in root.subviews() {
            if let Ok(found) = sub.clone().downcast::<T>()
                && matches(&found)
            {
                return Some(found);
            }
            if let Some(found) = find(&sub, matches) {
                return Some(found);
            }
        }
        None
    }

    /// Realizes `content` as a titled window through the production root,
    /// waits until the window carries its unified toolbar over a full-size
    /// content view — and the search row when `searchable` — then lays the
    /// tree out and hands `check` the content view and its edges.
    fn realized<V: waterui::View>(
        mtm: MainThreadMarker,
        content: impl Fn() -> V + 'static,
        searchable: bool,
        check: impl FnOnce(&NSView, &Edges),
    ) {
        let (mut env, _theme) = window_environment(mtm);
        env.insert(crate::first_paint::FirstPaint::default());
        let declaration =
            waterui::window::Window::new("Toolbar Reach", binding(WindowState::Normal), content)
                .style(WindowStyle::Titled);
        declaration
            .frame
            .set(Rect::new(Point::new(0.0, 0.0), Size::new(700.0, 520.0)));
        let host = crate::windows::realize(declaration, &env, mtm);
        let native = host.window().native();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                native.toolbar().is_some()
                    && native.toolbarStyle() == NSWindowToolbarStyle::Unified
                    && native.styleMask().contains(
                        NSWindowStyleMask::Titled | NSWindowStyleMask::FullSizeContentView,
                    )
                    && (!searchable || native.titlebarAccessoryViewControllers().count() == 1)
            }),
            "the window must carry a unified toolbar over a full-size content view{}",
            if searchable {
                " and the search row"
            } else {
                ""
            }
        );
        let root = native
            .contentView()
            .expect("the realized window has a content view");
        root.layoutSubtreeIfNeeded();
        let edges = Edges {
            window: native.frame().size.height,
            safe: top_in_window(&root, root.safeAreaRect()),
        };
        let root_top = top_in_window(&root, root.bounds());
        assert!(
            (root_top - edges.window).abs() <= TOLERANCE,
            "a full-size content view reaches the window top {}, got {root_top}",
            edges.window
        );
        assert!(
            edges.safe < edges.window - 1.0,
            "the titlebar must cover a band of the content view: safe-area top {}, window top {}",
            edges.safe,
            edges.window
        );
        check(&root, &edges);
        native.close();
        drop(host);
    }

    /// Asserts the list's scroll view runs under the toolbar to the
    /// window's top edge, and its first row rests [`LIST_GAP`] below the
    /// titlebar's bottom.
    fn assert_list_reaches_the_top(root: &NSView, edges: &Edges) {
        let list = find::<ListSurface>(root, &|_| true).expect("the window hosts the list");
        let top = top_in_window(&list, list.bounds());
        assert!(
            top >= edges.window - TOLERANCE,
            "the list's scroll view must reach the window top {} under the toolbar \
             (safe-area top {}), but its top is {top}",
            edges.window,
            edges.safe
        );
        let first_row = top_in_window(&list.table_view(), list.rect_of_row(0));
        assert!(
            first_row <= edges.safe + TOLERANCE,
            "the list's first row must sit below the titlebar's bottom {}, but its top is \
             {first_row}",
            edges.safe
        );
        let expected = edges.safe - LIST_GAP;
        assert!(
            (first_row - expected).abs() <= LIST_GAP_TOLERANCE,
            "the list's first row must rest {LIST_GAP}pt below the titlebar's bottom {}, at \
             {expected}, but its top is {first_row}",
            edges.safe
        );
    }

    /// Asserts the marker text stays below the titlebar.
    fn assert_text_stays_clear(root: &NSView, edges: &Edges) {
        let label = find::<NSTextField>(root, &|field| field.stringValue().to_string() == MARKER)
            .expect("the window hosts the marker text");
        let top = top_in_window(&label, label.bounds());
        assert!(
            top <= edges.safe + TOLERANCE,
            "non-scrolling content must stay below the titlebar at {}, but its top is {top}",
            edges.safe
        );
    }

    /// The fixtures' list: three plain rows.
    fn rows() -> impl waterui::View {
        List::content(vec![row_item as fn() -> ListItem; 3])
    }

    /// Window → `NavigationStack` → page whose content is a `List`.
    pub fn a_stack_page_list_runs_under_the_toolbar(mtm: MainThreadMarker) {
        realized(
            mtm,
            || NavigationStack::new(NavigationView::new("Inbox", rows())),
            false,
            assert_list_reaches_the_top,
        );
    }

    /// A row scrolled to the top sits below the titlebar.
    ///
    /// Window → `NavigationStack` → page whose content is a `List` taller
    /// than the window: the row lands at the inset content edge — where
    /// the first row rests — instead of under the toolbar.
    pub fn a_row_scrolled_to_the_top_sits_below_the_titlebar(mtm: MainThreadMarker) {
        realized(
            mtm,
            || {
                NavigationStack::new(NavigationView::new(
                    "Inbox",
                    List::content(vec![row_item as fn() -> ListItem; 60]),
                ))
            },
            false,
            |root, edges| {
                let list = find::<ListSurface>(root, &|_| true).expect("the window hosts the list");
                list.scroll_row_to_top(20);
                let row = top_in_window(&list.table_view(), list.rect_of_row(20));
                assert!(
                    row <= edges.safe + TOLERANCE,
                    "a row scrolled to the top must sit below the titlebar's bottom {}, but its \
                     top is {row}",
                    edges.safe
                );
                let expected = edges.safe - LIST_GAP;
                assert!(
                    (row - expected).abs() <= LIST_GAP_TOLERANCE,
                    "a row scrolled to the top must rest at the inset content edge {expected}, \
                     but its top is {row}"
                );
            },
        );
    }

    /// Window → `NavigationStack` → page whose content is text.
    pub fn a_stack_page_text_stays_clear_of_the_toolbar(mtm: MainThreadMarker) {
        realized(
            mtm,
            || NavigationStack::new(NavigationView::new("Inbox", text(MARKER))),
            false,
            assert_text_stays_clear,
        );
    }

    /// `Tabs` whose first tab is a `NavigationStack` over `inbox`, each tab
    /// a `Tab::container` — the picker sits in the window toolbar.
    fn tabbed(inbox: impl Fn() -> NavigationView + 'static) -> impl waterui::View {
        let inbox = std::rc::Rc::new(inbox);
        Tabs::new(
            &binding(0usize),
            vec![
                Tab::container("Inbox", move || NavigationStack::new(inbox())),
                Tab::container("Settings", || {
                    NavigationStack::new(NavigationView::new("Settings", text("settings")))
                }),
            ],
        )
    }

    /// Window → `Tabs` (its picker in the toolbar) → `NavigationStack` →
    /// page whose content is a `List`.
    pub fn a_tabbed_stack_page_list_runs_under_the_toolbar(mtm: MainThreadMarker) {
        realized(
            mtm,
            || tabbed(|| NavigationView::new("Inbox", rows())),
            false,
            assert_list_reaches_the_top,
        );
    }

    /// Window → `Tabs` (its picker in the toolbar) → `NavigationStack` →
    /// page whose content is text.
    pub fn a_tabbed_stack_page_text_stays_clear_of_the_toolbar(mtm: MainThreadMarker) {
        realized(
            mtm,
            || tabbed(|| NavigationView::new("Inbox", text(MARKER))),
            false,
            assert_text_stays_clear,
        );
    }

    /// The navigation example's structure: automatic-style `Tabs` of
    /// `Tab::container`s, the Inbox tab a path-backed `NavigationStack`
    /// whose root page carries a large title, a subtitle and a search
    /// field in the bar around `content`.
    fn inbox_tabs<V: waterui::View>(content: impl Fn() -> V + 'static) -> impl waterui::View {
        let content = std::rc::Rc::new(content);
        Tabs::new(
            &binding(0usize),
            vec![
                Tab::container("Inbox", move || {
                    NavigationStack::with_path(
                        NavigationPath::<u64>::new(),
                        content()
                            .title("Inbox")
                            .large_title()
                            .navigation_subtitle("3 unread")
                            .searchable(&binding(Str::from("")), "Search mail"),
                    )
                    .destination(|id: u64| {
                        NavigationView::new("Message", text(format!("message {id}")))
                    })
                }),
                Tab::container("Settings", || {
                    NavigationStack::new(NavigationView::new("Settings", text("settings")))
                }),
            ],
        )
        .style(tab_style::Automatic)
    }

    /// The navigation example's Inbox: its `List` of navigation-link rows
    /// runs under the toolbar and the search row.
    pub fn the_example_inbox_list_runs_under_the_toolbar(mtm: MainThreadMarker) {
        realized(
            mtm,
            || {
                inbox_tabs(|| {
                    List::for_each(
                        vec![SelfId::new(1_u64), SelfId::new(2), SelfId::new(3)],
                        |id: SelfId<u64>| {
                            ListItem::new(NavigationLink::value(text("message"), id.into_inner()))
                        },
                    )
                    .editing(binding::<bool>(false))
                })
            },
            true,
            assert_list_reaches_the_top,
        );
    }

    /// The same structure with text for content: it stays below the
    /// titlebar and the search row.
    pub fn the_example_inbox_text_stays_clear_of_the_toolbar(mtm: MainThreadMarker) {
        realized(
            mtm,
            || inbox_tabs(|| text(MARKER)),
            true,
            assert_text_stays_clear,
        );
    }
}

/// Re-exports the GPU-surface mounted-scene fixtures.
///
/// The `native_test` module inside `components::gpu_surface` builds a real
/// `SceneView` mount and drives the production failure drain and
/// completion settlement paths on it.
#[cfg(all(target_os = "macos", feature = "gpu_surface"))]
pub mod gpu_surface {
    pub use crate::components::gpu_surface::native_test::{
        MountedSceneSurface, WakeProbe, fixture_env, fixture_scene_view,
    };

    /// Performs the once-per-process `startup::initialize` — called
    /// once from the `Tests/native.rs` harness's true main thread
    /// before any trial runs; fixture mounts rely on that explicit
    /// harness setup.
    pub fn initialize_process() {
        let _ = crate::startup::initialize();
    }
}

/// Re-exports the pieces a `ViewRenderer::render` trial needs.
///
/// The service installer, the shared-runtime handles a sealed generation
/// is reached through, and the failure carriers the cause chain asserts
/// on.
#[cfg(all(target_os = "macos", feature = "gpu_surface"))]
pub mod view_renderer {
    pub use crate::components::view_renderer::install_service;
    pub use crate::gpu_runtime::{EngineGeneration, SceneEngine, scene_engine};
    pub use waterui_graphics::gpu::GpuRuntime;
    pub use waterui_graphics::gpu::runtime::HostedLayerError;
}

/// Re-exports the filtered mounted-surface fixtures.
///
/// The `native_test` module inside `components::filtered` mounts a filtered
/// leaf over a real `SceneView` GPU-surface child through the production
/// `build_filtered_parts` construction, for the settle-contract trials.
#[cfg(all(target_os = "macos", feature = "gpu_surface"))]
pub mod filtered {
    pub use crate::components::filtered::native_test::{MountedFilteredSurface, WakeProbe};
}

/// The web view leaf's harness reach.
///
/// Opens a real `WKWebView` through the same late install an application
/// gets — `install_service` fills the environment's `WebViewController`
/// slot exactly the way `embedding::mount` does — so a case can drive the
/// engine without mounting a view tree.
#[cfg(feature = "webview")]
pub mod webview {
    use cocoa_ui::MainThreadMarker;
    use waterui_backend_core::Environment;

    /// The `WebViewController` the leaf's `install_service` installs.
    ///
    /// This is the same construction `open` drives, so a case that opens
    /// several views shares the one service install. `mtm` is the proof
    /// the call happened on the real main thread, which `WKWebView`
    /// requires.
    #[must_use]
    pub fn controller(_mtm: MainThreadMarker) -> waterui_webview::WebViewController {
        let mut env = Environment::new();
        crate::components::webview::install_service(&mut env);
        env.get::<waterui_webview::WebViewController>()
            .expect("the webview leaf installs its WebViewController")
            .clone()
    }

    /// A fresh `WebView` backed by the backend's own `WKWebView`, never
    /// mounted. `mtm` is the proof the call happened on the real main
    /// thread, which `WKWebView` requires.
    #[must_use]
    pub fn open(mtm: MainThreadMarker) -> waterui_webview::WebView {
        controller(mtm).open()
    }
}

/// Counts `tracing` ERROR events one module emits — a settle contract
/// that must log exactly once per failure asserts on the count.
///
/// Used as a `tracing::Subscriber` inside `with_default`, so only the
/// events the wrapped closure raises are observed.
#[cfg(all(target_os = "macos", feature = "gpu_surface"))]
#[derive(Debug)]
pub struct ErrorLog {
    target: &'static str,
    count: alloc::sync::Arc<core::sync::atomic::AtomicU32>,
}

#[cfg(all(target_os = "macos", feature = "gpu_surface"))]
impl ErrorLog {
    /// An ERROR counter for events whose target is `target` (the
    /// emitting module path, e.g.
    /// `"waterui_apple::components::gpu_surface"`).
    #[must_use]
    pub fn new(target: &'static str) -> (Self, alloc::sync::Arc<core::sync::atomic::AtomicU32>) {
        let count = alloc::sync::Arc::new(core::sync::atomic::AtomicU32::new(0));
        (
            Self {
                target,
                count: count.clone(),
            },
            count,
        )
    }
}

#[cfg(all(target_os = "macos", feature = "gpu_surface"))]
impl tracing::Subscriber for ErrorLog {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::ERROR && metadata.target() == self.target
    }
    fn new_span(&self, _attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn register_callsite(
        &self,
        _meta: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }
    fn event(&self, _event: &tracing::Event<'_>) {
        self.count
            .fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

/// The minimum environment a real render needs.
///
/// `dispatch::install` performs the backend's half of the embedding
/// contract (dispatcher, window manager, realizations); the theme slots
/// text resolves through are the framework's. Shared by the `native` and
/// `native_app` harnesses.
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
    // A selectable list's rows re-theme onto the selection role.
    waterui::theme::install_color_signal::<waterui::theme::color::SelectionForeground>(
        &mut env,
        black(),
    );
    waterui::theme::install_font_signal::<Body>(&mut env, binding(Body::DEFAULT).computed());
    waterui::theme::install_font_signal::<Caption>(&mut env, binding(Caption::DEFAULT).computed());
    waterui::theme::install_font_signal::<Subheadline>(
        &mut env,
        binding(Subheadline::DEFAULT).computed(),
    );
    env
}
/// The view's current pixels through the production
/// [`capture_rgba`](crate::capture_image::capture_rgba) path — the same
/// capture every visual-review export drives.
#[cfg(feature = "gpu_surface")]
pub fn capture_view_rgba(view: &cocoa_ui::PlatformView) -> crate::capture_image::CapturedRgba {
    let mut env = render_environment();
    let runtime = block_on_main(30.0, waterui_graphics::gpu::GpuRuntime::new())
        .expect("the harness provides a GPU runtime");
    install_gpu(&mut env, runtime);
    let scale = cocoa_ui::view::backing_scale_factor(view).max(1.0);
    let mtm = MainThreadMarker::new().expect("captures run on the main thread");
    block_on_main(
        30.0,
        crate::capture_image::capture_rgba(view, &env, scale, mtm),
    )
    .expect("the window capture completes")
}

/// Installs the GPU owners onto an existing environment.
///
/// Runtime, scene engine, filter-pipeline cache and the
/// `PresentationTime` anchor — the set a `captures_content` transition
/// and the filtered handler resolve. `gpu_runtime::prepare` installs
/// the same beside the runtime in production; trials that render
/// `FilteredView` through the dispatcher need it on the env they hand
/// `dispatch::render`.
#[cfg(feature = "gpu_surface")]
pub fn install_gpu(
    env: &mut waterui_backend_core::Environment,
    runtime: waterui_graphics::gpu::GpuRuntime,
) {
    use std::rc::Rc;
    env.insert(runtime);
    env.insert(Rc::new(crate::gpu_runtime::SceneEngine::new()));
    env.insert(Rc::new(crate::gpu_runtime::FilterPipelines::new()));
    crate::presentation_time::PresentationTime::install(env);
    crate::components::view_renderer::install_service(env);
    // The dispatcher captures the environment at install time — re-run
    // the install so its root env sees the GPU services just inserted
    // (`handler.set`-driven remounts render through it).
    crate::dispatch::install(env);
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

/// The offset that puts `row`'s top edge at the viewport's top.
///
/// Read before any clamp: `rectOfRow`'s origin on `AppKit`;
/// `rectForRowAtIndexPath`'s origin in section 0 minus the adjusted top
/// inset on `UIKit`, read as the table stands now, since `UIKit` sizes
/// unseen rows by estimate.
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

/// Reads the standard menu bar as `menus::install` builds it — for the
/// native suite to assert the standard items' rows without an `AppKit`
/// handle of its own.
#[cfg(target_os = "macos")]
pub mod menus {
    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::appkit::Application;
    use cocoa_ui::objc2_app_kit::{NSApplication, NSEventModifierFlags};
    use cocoa_ui::objc2_foundation::NSString;
    use waterui::component::menu::{CloseWindowPlacement, Shortcut};

    /// One row of a built menu — what a case asserts without touching
    /// `NSMenuItem`.
    #[derive(Debug)]
    #[non_exhaustive]
    pub struct MenuRow {
        /// The row's title.
        pub title: String,
        /// Its key equivalent — `""` for none.
        pub key_equivalent: String,
        /// The modifiers its key equivalent takes.
        pub modifiers: NSEventModifierFlags,
        /// The name of the selector the row sends, when it sends one —
        /// `performClose:` on the standard Close item.
        pub action: Option<String>,
        /// Whether the row reports enabled after a validation pass.
        pub enabled: bool,
        /// Whether the row is a separator.
        pub separator: bool,
    }

    /// The Window menu's rows in the standard bar `menus::install` builds.
    ///
    /// Installs that bar — the default menus plus nothing declared, with
    /// `close_chord` the chord its Close item carries — then runs an
    /// `update()` validation pass on the Window menu, so `enabled`
    /// answers what the responder chain can take.
    #[must_use]
    pub fn window_menu_rows(
        mtm: MainThreadMarker,
        close_window: CloseWindowPlacement,
        close_chord: Option<&Shortcut>,
    ) -> Vec<MenuRow> {
        let application = Application::shared(mtm);
        crate::menus::install(mtm, &application, &[], close_window, close_chord);
        let main = NSApplication::sharedApplication(mtm)
            .mainMenu()
            .expect("install sets the main menu");
        let window_menu = main
            .itemWithTitle(&NSString::from_str("Window"))
            .and_then(|item| item.submenu())
            .expect("the standard bar's Window item opens a submenu");
        window_menu.update();
        window_menu
            .itemArray()
            .iter()
            .map(|item| MenuRow {
                title: item.title().to_string(),
                key_equivalent: item.keyEquivalent().to_string(),
                modifiers: item.keyEquivalentModifierMask(),
                action: item.action().map(|sel| {
                    sel.name()
                        .to_str()
                        .expect("selector names are ASCII")
                        .into()
                }),
                enabled: item.isEnabled(),
                separator: item.isSeparatorItem(),
            })
            .collect()
    }
}

/// The inspector tree `walk` produces over `root` — a trial's read of
/// the published hierarchy without driving the inspect gesture.
pub fn inspector_tree(
    root: &cocoa_ui::PlatformView,
) -> Vec<waterui::inspector::protocol::TreeNode> {
    crate::inspector::recordable_tree(root)
}

/// Navigation chrome drawn in content sits at the top of its host.
///
/// A borderless window has no toolbar to take the chrome, so a page's
/// header and a tab host's strip are drawn in content. `HostView` is
/// flipped, so both must dock at the smallest y of the host, the top, with
/// the content below them.
#[cfg(target_os = "macos")]
pub mod chrome_top {
    use cocoa_ui::objc2_app_kit::{NSSegmentedControl, NSTextField, NSView};
    use cocoa_ui::{MainThreadMarker, Retained};
    use waterui::navigation::{NavigationView, Tab, Tabs, tab_style};
    use waterui::prelude::text;
    use waterui::reactive::binding;
    use waterui::window::{WindowState, WindowStyle};
    use waterui_core::layout::{Point, Rect, Size};

    use super::{MAIN_QUEUE_DEADLINE, pump_main_until, window_environment};

    /// The text the fixtures show as their content.
    const MARKER: &str = "chrome-top-marker";

    /// The gap the tab host leaves between the page top and its strip.
    const STRIP_INSET: f64 = 8.0;

    /// The title the page's in-content header shows.
    const TITLE: &str = "chrome-top-title";

    /// Half a point: the distance at which an edge counts as reaching a
    /// boundary at any backing scale.
    const TOLERANCE: f64 = 0.5;

    /// `rect`, a rect in `view`'s coordinates, in window coordinates —
    /// where y grows upward whatever the view's own orientation.
    fn in_window(view: &NSView, rect: cocoa_ui::objc2_foundation::NSRect) -> (f64, f64) {
        let frame = view.convertRect_toView(rect, None);
        (frame.origin.y, frame.origin.y + frame.size.height)
    }

    /// The first text field under `root` showing `string`.
    fn label(root: &NSView, string: &str) -> Option<Retained<NSTextField>> {
        for sub in root.subviews() {
            if let Ok(field) = sub.clone().downcast::<NSTextField>()
                && field.stringValue().to_string() == string
            {
                return Some(field);
            }
            if let Some(found) = label(&sub, string) {
                return Some(found);
            }
        }
        None
    }

    /// The first segmented control under `root`.
    fn strip(root: &NSView) -> Option<Retained<NSSegmentedControl>> {
        for sub in root.subviews() {
            if let Ok(control) = sub.clone().downcast::<NSSegmentedControl>() {
                return Some(control);
            }
            if let Some(found) = strip(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// Realizes `content` as a borderless window through the production
    /// root, lays it out and hands `check` the content view.
    fn realized<V: waterui::View>(
        mtm: MainThreadMarker,
        content: impl Fn() -> V + 'static,
        check: impl FnOnce(&NSView),
    ) {
        let (mut env, _theme) = window_environment(mtm);
        env.insert(crate::first_paint::FirstPaint::default());
        let declaration =
            waterui::window::Window::new("Chrome Top", binding(WindowState::Normal), content)
                .style(WindowStyle::Borderless);
        declaration
            .frame
            .set(Rect::new(Point::new(0.0, 0.0), Size::new(700.0, 520.0)));
        let host = crate::windows::realize(declaration, &env, mtm);
        let native = host.window().native();
        let root = native
            .contentView()
            .expect("the realized window has a content view");
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                root.layoutSubtreeIfNeeded();
                label(&root, MARKER).is_some()
            }),
            "the window must realize the fixture"
        );
        root.layoutSubtreeIfNeeded();
        check(&root);
        native.close();
        drop(host);
    }

    /// Window → standalone `NavigationView`: its in-content header sits at
    /// the top of the page and the content below it.
    pub fn a_standalone_page_header_sits_above_its_content(mtm: MainThreadMarker) {
        realized(
            mtm,
            || NavigationView::new(TITLE, text(MARKER)),
            |root| {
                let (_, window_top) = in_window(root, root.bounds());
                let title = label(root, TITLE).expect("the in-content header shows the title");
                let header =
                    cocoa_ui::view::superview(&title).expect("the title sits in the header");
                let (header_bottom, header_top) = in_window(&header, header.bounds());
                let content = label(root, MARKER).expect("the page shows its content");
                let (_, content_top) = in_window(&content, content.bounds());
                assert!(
                    (header_top - window_top).abs() <= TOLERANCE,
                    "the header must dock at the page top {window_top}, but its top is \
                     {header_top}"
                );
                assert!(
                    content_top <= header_bottom + TOLERANCE,
                    "the content must start below the header's bottom {header_bottom}, but \
                     its top is {content_top}"
                );
            },
        );
    }

    /// Window → `Tabs` with the segmented strip: the strip sits at the top
    /// of the page and the selected pane below it.
    pub fn a_tab_strip_sits_above_its_panes(mtm: MainThreadMarker) {
        realized(
            mtm,
            || {
                Tabs::new(
                    &binding(0usize),
                    vec![
                        Tab::container("First", || text(MARKER)),
                        Tab::container("Second", || text("second")),
                    ],
                )
                .style(tab_style::Automatic)
            },
            |root| {
                let (_, window_top) = in_window(root, root.bounds());
                let strip = strip(root).expect("the host draws the segmented strip");
                let (strip_bottom, strip_top) = in_window(&strip, strip.bounds());
                let content = label(root, MARKER).expect("the selected pane shows its content");
                let (_, content_top) = in_window(&content, content.bounds());
                assert!(
                    (window_top - strip_top - STRIP_INSET).abs() <= TOLERANCE,
                    "the strip must sit {STRIP_INSET}pt below the page top {window_top}, but \
                     its top is {strip_top}"
                );
                assert!(
                    content_top <= strip_bottom + TOLERANCE,
                    "the pane must start below the strip's bottom {strip_bottom}, but its top \
                     is {content_top}"
                );
            },
        );
    }
}
