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

pub use cocoa_ui::native_test::pump_main_until;

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
