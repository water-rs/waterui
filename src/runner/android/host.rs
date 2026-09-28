//! The Android host's window services, executor and frame transaction.
//!
//! `AndroidHostWindow` is the [`PlatformWindow`] the Kotlin `HydrolysisHostView`
//! stands behind: window metrics live here (one coherent snapshot pushed by
//! the host, never read off the GPU attachment), input arrives as pushed
//! events, and `request_redraw` crosses JNI once to post a Choreographer
//! frame. `AndroidSession` is the mounted app: `RuntimeWindow` + environment
//! + the main-Looper executor, driven by one frame transaction per vsync
//! callback — the plan's "one coordinated frame transaction", never an
//! unconditional tick-then-render pair.
//!
//! The executor keeps the shared channel-queue shape, but its wake writes an
//! `eventfd` that the main `ALooper` is watching instead of a Java callback —
//! wakeups coalesce in the fd and never pay a JNI round-trip per task.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use executor_core::LocalExecutor;
use executor_core::async_task::{AsyncTask, Runnable};
use jni::JavaVM;
use jni::objects::{GlobalRef, JValue};
use nami::Signal;
use ndk::looper::{FdEvent, ForeignLooper, ThreadLooper};
use waterui::Environment;
use waterui::cursor::CursorStyle;
use waterui::window::WindowState;
use waterui_text::FontCollection;

use super::accessibility::AccessibilitySnapshot;
use super::fonts::android_fonts;
use super::gpu::{AndroidGpuContext, AndroidSurface};
use super::ime::ImeBridge;
use super::jni::JniError;
use super::platform_views::PlatformViewTable;
use crate::engine::WidgetTheme;
use crate::platform::{
    GpuSurfaceWindow, InputEvent, PlatformWindow, SurfaceProvider, TextInputState,
    validated_window_frame,
};
use crate::renderer::{HydrolysisRenderer, HydrolysisTextContextMenuMode, MenuShortcutRegistry};
use crate::runner::window::{RuntimeWindow, advance_runtime, handle_input_events, render_window};
use crate::runner::{
    RenderDiagnosticsConfig, init_main_thread_executors, install_headless_window_managers,
    install_native_component_hooks, menu_bar,
};
use crate::time::Instant;

/// One coherent metrics snapshot the host pushes — size, density, font scale,
/// display refresh and the system-bar insets arrive together, never piecemeal.
#[derive(Clone, Debug)]
pub(crate) struct MetricsSnapshot {
    pub(crate) width_px: u32,
    pub(crate) height_px: u32,
    /// Physical pixels per logical (dp) unit — the platform scale factor.
    pub(crate) density: f64,
    /// The user's fontScale (`Configuration.fontScale`), applied on top of
    /// density for text metrics.
    #[expect(
        dead_code,
        reason = "carried in the coherent metrics snapshot; hydrolysis has no font-scale sink yet"
    )]
    pub(crate) font_scale: f64,
    /// The display's refresh rate in Hz, when the host reports one.
    pub(crate) refresh_hz: Option<f64>,
    /// Window-inset edges in physical px: `[left, top, right, bottom]` —
    /// the combined system-bar/cutout/IME insets the safe-area contract reads.
    pub(crate) insets_px: [i32; 4],
}

/// The session's JNI handle back into the Kotlin host — a cached `JavaVM`
/// plus a global reference to the `HydrolysisHostView` that owns this
/// session. Every call lands on the UI thread (the only thread these
/// callbacks ever run on) through `get_env`/`attach_current_thread`.
pub(crate) struct HostBridge {
    vm: JavaVM,
    host_view: GlobalRef,
}

impl HostBridge {
    fn call(&self, name: &'static str, sig: &'static str, args: &[JValue]) {
        match self.vm.get_env() {
            Ok(mut env) => self.call_with_env(&mut env, name, sig, args),
            Err(_) => match self.vm.attach_current_thread() {
                Ok(mut guard) => self.call_with_env(&mut guard, name, sig, args),
                Err(error) => {
                    tracing::error!(
                        target: "waterui::hydrolysis::android",
                        method = name,
                        %error,
                        "host callback could not attach a JNI env"
                    );
                }
            },
        }
    }

    fn call_with_env(
        &self,
        env: &mut jni::JNIEnv,
        name: &'static str,
        sig: &'static str,
        args: &[JValue],
    ) {
        if let Err(error) = env.call_method(&self.host_view, name, sig, args) {
            // A pending Java exception (the host's deliberate throw for a
            // fatal GPU error) surfaces to the Kotlin caller as-is; other
            // JNI failures are logged, never silently dropped.
            if env.exception_check().unwrap_or(false) {
                return;
            }
            tracing::error!(
                target: "waterui::hydrolysis::android",
                method = name,
                %error,
                "host callback failed"
            );
        }
    }

    /// Posts a Choreographer frame request on the host view's scheduler.
    pub(crate) fn request_frame(&self) {
        self.call("onNativeRequestRedraw", "()V", &[]);
    }

    /// Pushes the focused text-input rect (physical px) and purpose to the
    /// host's IME controller; a negative purpose clears it (keyboard hides).
    fn sync_text_input_state(&self, state: Option<TextInputState>, density: f64) {
        let args: &[JValue] = match state {
            Some(state) => &[
                JValue::Float((f64::from(state.x) * density) as f32),
                JValue::Float((f64::from(state.y) * density) as f32),
                JValue::Float((f64::from(state.width) * density) as f32),
                JValue::Float((f64::from(state.height) * density) as f32),
                JValue::Int(match state.purpose {
                    crate::platform::TextInputPurpose::Normal => 0,
                    crate::platform::TextInputPurpose::Password => 1,
                }),
            ],
            None => &[
                JValue::Float(0.0),
                JValue::Float(0.0),
                JValue::Float(0.0),
                JValue::Float(0.0),
                JValue::Int(-1),
            ],
        };
        self.call("onNativeTextInputState", "(FFFFI)V", args);
    }

    /// Marks the published accessibility snapshot dirty on the host side.
    #[cfg(feature = "accessibility")]
    pub(crate) fn accessibility_tree_changed(&self) {
        self.call("onNativeAccessibilityTreeChanged", "()V", &[]);
    }

    /// Delivers a fatal error (GPU loss, unrecoverable renderer failure) —
    /// the host raises it as an exception on the UI thread.
    fn fatal_error(&self, message: &str) {
        let Ok(mut env) = self.vm.get_env() else {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                %message,
                "fatal error raised with no attached JNIEnv"
            );
            return;
        };
        let Ok(message) = env.new_string(message) else {
            return;
        };
        let _ = env.call_method(
            &self.host_view,
            "onNativeFatalError",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&message)],
        );
    }

    /// The window's content asked to close — the host finishes the activity.
    fn close_requested(&self) {
        self.call("onNativeCloseRequested", "()V", &[]);
    }
}

/// The Kotlin host's window on the runner's side: host services only. The
/// GPU attachment is a separate object — this type's [`PlatformWindow`] impl
/// never touches it, and [`GpuSurfaceWindow::surface`] is the only path that
/// hands it to the painter.
pub(crate) struct AndroidHostWindow {
    metrics: MetricsSnapshot,
    events: Vec<InputEvent>,
    pub(crate) surface: AndroidSurface,
    pub(crate) bridge: HostBridge,
    /// A redraw the engine asked for that has not yet reached the scheduler;
    /// consumed at the end of the frame transaction as scheduling demand.
    redraw_pending: Cell<bool>,
    cursor_style: CursorStyle,
}

impl AndroidHostWindow {
    /// Queues an input event for the next frame transaction's dispatch.
    ///
    /// Host-side `MotionEvent` coordinates arrive in physical pixels while the
    /// retained scene is laid out in logical units — the boundary converts,
    /// exactly as the winit runner's `PhysicalPosition::to_logical` does.
    pub(crate) fn push_event(&mut self, mut event: InputEvent) {
        let density = self.metrics.density as f32;
        match &mut event {
            InputEvent::PointerDown { x, y, .. }
            | InputEvent::PointerUp { x, y, .. }
            | InputEvent::PointerMove { x, y, .. }
            | InputEvent::Moved { x, y }
            | InputEvent::Scroll { x, y, .. }
            | InputEvent::TrackpadPan { x, y, .. }
            | InputEvent::Magnification { x, y, .. }
            | InputEvent::Rotation { x, y, .. } => {
                *x /= density;
                *y /= density;
            }
            _ => {}
        }
        if let InputEvent::Scroll {
            dx,
            dy,
            is_line_delta,
            ..
        } = &mut event
        {
            if !*is_line_delta {
                *dx /= density;
                *dy /= density;
            }
        }
        // Input is a wake, not a pump: the first queued event posts one
        // Choreographer frame that dispatches the batch; the engine's `Next`
        // decides whether anything follows.
        let wakes = self.events.is_empty();
        self.events.push(event);
        if wakes {
            tracing::debug!(
                target: "waterui::hydrolysis::android",
                "wake posted: input event queued"
            );
            self.bridge.request_frame();
        }
    }

    pub(crate) fn take_redraw_pending(&self) -> bool {
        self.redraw_pending.replace(false)
    }
}

impl PlatformWindow for AndroidHostWindow {
    /// The drawable content area in physical pixels, from the metrics
    /// snapshot — valid before any surface exists and while between surface
    /// generations.
    fn content_size(&self) -> (u32, u32) {
        (self.metrics.width_px, self.metrics.height_px)
    }

    fn apply_properties(&mut self, window: &waterui::window::Window) {
        if window.state.snapshot() == WindowState::Closed {
            return;
        }
        // The host sizes the window; the app's `Window::frame` binding is a
        // mount-time request — validated (never NaN) and then driven by the
        // host's metrics, which is the size the frame must fill. The write is
        // value-gated: `window.frame` is subscribed by the render pass itself,
        // so an unconditional `set` re-arms a refresh every frame and the
        // pump never idles.
        let current = validated_window_frame(window.frame.snapshot());
        let logical_width = f64::from(self.metrics.width_px) / self.metrics.density;
        let logical_height = f64::from(self.metrics.height_px) / self.metrics.density;
        let target = waterui_core::layout::Rect::new(
            waterui_core::layout::Point::new(0.0, 0.0),
            waterui_core::layout::Size::new(logical_width as f32, logical_height as f32),
        );
        if current != target {
            window.frame.set(target);
        }
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut self.events)
    }

    fn request_redraw(&self) {
        self.redraw_pending.set(true);
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            "wake posted: redraw requested"
        );
        self.bridge.request_frame();
    }

    fn scale_factor(&self) -> f64 {
        self.metrics.density
    }

    fn refresh_rate_hz(&self) -> Option<f64> {
        self.metrics.refresh_hz
    }

    fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
        self.bridge
            .sync_text_input_state(state, self.metrics.density);
    }

    fn set_cursor_style(&mut self, style: CursorStyle) {
        // Touch has no cursor chrome; stylus/mouse sessions on desktop-mode
        // displays read it when the pointer hovers the band.
        self.cursor_style = style;
    }
}

impl GpuSurfaceWindow for AndroidHostWindow {
    fn surface(&mut self) -> &mut dyn SurfaceProvider {
        &mut self.surface
    }
}

/// The UI-local executor adapted to the main `ALooper`: the same channel
/// queue the headless runner drains, but a waker's send also writes a
/// coalescing `eventfd` the looper's fd callback watches — work scheduled
/// from any thread lands on the UI thread without a JNI call per wake.
#[derive(Clone, Debug)]
pub(crate) struct AndroidMainThreadExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    runnable_rx: Rc<mpsc::Receiver<Runnable>>,
    pending: Arc<AtomicUsize>,
    /// The `eventfd` every schedule edge writes; the main `ALooper` reads it.
    wake_fd: Arc<OwnedFd>,
}

impl AndroidMainThreadExecutor {
    fn new(wake_fd: OwnedFd) -> Self {
        let (runnable_tx, runnable_rx) = mpsc::channel();
        Self {
            runnable_tx,
            runnable_rx: Rc::new(runnable_rx),
            pending: Arc::new(AtomicUsize::new(0)),
            wake_fd: Arc::new(wake_fd),
        }
    }

    /// Runs every runnable currently queued, returning whether any ran.
    pub(crate) fn drain(&self) -> bool {
        let mut ran = false;
        loop {
            let Ok(runnable) = self.runnable_rx.try_recv() else {
                return ran;
            };
            ran = true;
            runnable.run();
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl LocalExecutor for AndroidMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let pending = Arc::clone(&self.pending);
        let wake_fd = self.wake_fd.as_raw_fd();
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            pending.fetch_add(1, Ordering::SeqCst);
            match runnable_tx.send(runnable) {
                Ok(()) => {
                    // Coalesced wake: one counter increment regardless of how
                    // many runnables are already queued.
                    unsafe {
                        libc::eventfd_write(wake_fd, 1);
                    }
                }
                Err(unsent) => {
                    pending.fetch_sub(1, Ordering::SeqCst);
                    // Same teardown race as the headless executor: dropping a
                    // spawn_local runnable off-thread panics — leak it.
                    std::mem::forget(unsent);
                }
            }
        });
        runnable.schedule();
        task
    }
}

/// Owns the `eventfd` an `AndroidMainThreadExecutor` wakes on, and the
/// `ALooper` registration that drains it. Dropping unregisters the fd before
/// it is closed, so the looper can never fire into a dead session.
struct ExecutorWake {
    looper: ForeignLooper,
    fd: OwnedFd,
}

impl ExecutorWake {
    /// Creates the eventfd and registers its drain callback on this thread's
    /// looper — must be the UI thread (its main looper exists already).
    fn register(executor: AndroidMainThreadExecutor) -> Result<Self, JniError> {
        // SAFETY: zero flags would let a saturated counter block the looper —
        // NONBLOCK + CLOEXEC it is.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(JniError(format!(
                "hydrolysis android: eventfd failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: fd >= 0 checked above.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let wake_fd = fd.as_fd();
        let looper = ThreadLooper::for_thread().ok_or_else(|| {
            JniError("hydrolysis android: no ALooper on the UI thread".to_owned())
        })?;
        looper
            .add_fd_with_callback(wake_fd, FdEvent::INPUT, move |fd, _events| {
                let mut count: u64 = 0;
                // SAFETY: fd is the registered eventfd; eventfd_read consumes
                // every coalesced wake at once.
                unsafe {
                    libc::eventfd_read(fd.as_raw_fd(), &mut count);
                }
                let drained = executor.drain();
                tracing::debug!(
                    target: "waterui::hydrolysis::android",
                    count,
                    drained,
                    "executor wake: local task scheduled"
                );
                true
            })
            .map_err(|error| {
                JniError(format!("hydrolysis android: ALooper_addFd failed: {error}"))
            })?;
        Ok(Self {
            looper: looper.into_foreign(),
            fd,
        })
    }
}

impl Drop for ExecutorWake {
    fn drop(&mut self) {
        let _ = self.looper.remove_fd(self.fd.as_fd());
    }
}

/// Outcome of one frame transaction, decoded by the Kotlin scheduler.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameOutcome {
    /// The engine wants another vsync-aligned frame (animations, pending
    /// refresh, a redraw request consumed this pass). False means the window
    /// goes fully idle — no callbacks, no CPU or GPU work.
    pub(crate) wants_next_frame: bool,
    /// Nanoseconds until the next gesture deadline, if the engine reported
    /// one — the scheduler posts a fallback frame then in case no other wake
    /// fires first.
    pub(crate) deadline_in_nanos: Option<i64>,
    /// The window's content asked to close (its `Window::state` is Closed).
    pub(crate) should_close: bool,
}

/// The mounted app session: one runtime window, its environment, the
/// executor and the GPU context. Owned by the Kotlin side through a raw
/// pointer as `jlong`; every entry point runs on the UI thread.
pub(crate) struct AndroidSession {
    pub(crate) env: Environment,
    pub(crate) runtime: RuntimeWindow<AndroidHostWindow>,
    executor: AndroidMainThreadExecutor,
    /// Keeps the eventfd registered with the main looper for the session's
    /// life; drop order unregisters it before the fd closes.
    #[expect(
        dead_code,
        reason = "held for its Drop side effect — unregister + close"
    )]
    wake: ExecutorWake,
    /// The GPU context outlives the surface and renderer inside `runtime`
    /// (they drop first — field order is teardown order).
    #[expect(
        dead_code,
        reason = "held so the wgpu device outlives the runtime's GPU objects"
    )]
    gpu: AndroidGpuContext,
    /// Extra windows the app asks for while mounted: on Android one session
    /// mounts exactly one host view, so a second window is an explicit
    /// unsupported-feature error, not a silently dropped request.
    pending_window_queue: Rc<RefCell<Vec<waterui::window::Window>>>,
    /// The published accessibility snapshot — read only by the
    /// `accessibility` feature's publish/query paths.
    #[cfg_attr(
        not(feature = "accessibility"),
        expect(dead_code, reason = "read only under the accessibility feature")
    )]
    pub(crate) a11y: AccessibilitySnapshot,
    pub(crate) platform_views: PlatformViewTable,
    pub(crate) ime: ImeBridge,
    /// The live surface generation, as last reported by the host.
    surface_generation: u64,
    /// The deadline the last frame transaction reported, read out by
    /// `nativeFrameDeadlineInNanos`.
    frame_deadline_in_nanos: Option<i64>,
    /// The live window safe-area binding the `WindowSafeArea` env value wraps:
    /// `set_metrics` writes it on every host insets change and the windowed
    /// pipeline re-lays out through the subscription it read.
    safe_area: nami::Binding<waterui_layout::padding::EdgeInsets>,
    /// A frame reached the surface at least once; together with an idle
    /// `wants_next_frame` it arms the one-shot readiness log the device's
    /// idle-CPU sampling waits on.
    presented_once: Cell<bool>,
    /// The readiness line was already logged — it fires exactly once per
    /// session so a later busy window cannot re-arm it.
    ready_logged: Cell<bool>,
    /// Sessions are owned and driven on the UI thread only.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl AndroidSession {
    /// Mounts the registered app on the host view: builds the environment,
    /// executor wake bridge, GPU context and the runtime window.
    ///
    /// `metrics` is the host's snapshot at creation time — content size and
    /// scale exist from the start, before the surface band attaches.
    pub(crate) fn create(
        vm: JavaVM,
        host_view: GlobalRef,
        metrics: MetricsSnapshot,
        sdk_int: i32,
    ) -> Result<Box<Self>, JniError> {
        let inspector = init_main_thread_executors();
        let inspector_probe = inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe);

        let bridge = HostBridge { vm, host_view };

        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(JniError(format!(
                "hydrolysis android: eventfd failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: fd >= 0 checked above.
        let executor = AndroidMainThreadExecutor::new(unsafe { OwnedFd::from_raw_fd(fd) });
        let wake = ExecutorWake::register(executor.clone())?;
        let _ = executor_core::try_init_local_executor(
            waterui::task::monitored_local_executor_with_probes(
                executor.clone(),
                waterui::task::RefreshRate::HEADLESS,
                inspector_probe,
            ),
        );

        waterui_locale::start_system_locale_listener();

        let (app, style) = super::instantiate_app();
        let waterui::app::AppParts {
            windows,
            menu_bar,
            env,
            // A phone activity already embodies both policies: when the last
            // window asks to close the host finishes the activity, and the OS
            // itself decides whether the process stays resident.
            last_window: _,
        } = app.into_parts();
        let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
        waterui::inspector::install(&mut env, inspector);
        let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
        install_native_component_hooks(&mut env);
        install_headless_window_managers(&mut env, Rc::clone(&pending_window_queue));
        menu_bar::register_menu_bar(&menu_bar, &env);
        env.insert(HydrolysisTextContextMenuMode::Overlay);
        crate::theme::install_theme_tokens(&mut env, Some(&*style));
        let theme: Rc<dyn WidgetTheme> = style;
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::new(Rc::clone(&theme)),
        ));
        let fonts = FontCollection::new(android_fonts());
        fonts.clone().install(&mut env);
        let shortcuts = env
            .get::<MenuShortcutRegistry>()
            .expect("install_headless_window_managers seeds MenuShortcutRegistry")
            .clone();
        let safe_area = nami::binding(waterui_layout::padding::EdgeInsets::default());
        env.insert(crate::platform::WindowSafeArea(safe_area.clone()));

        let mut windows = VecDeque::from(windows);
        let window = windows
            .pop_front()
            .unwrap_or_else(|| panic!("hydrolysis android: the app registered no window to mount"));
        assert!(
            windows.is_empty(),
            "hydrolysis android: multiple windows are unsupported — the host mounts exactly one window per session"
        );

        let gpu = AndroidGpuContext::request().map_err(JniError::from)?;
        let mut platform = AndroidHostWindow {
            metrics,
            events: Vec::new(),
            surface: AndroidSurface::new(gpu.clone(), sdk_int),
            bridge,
            redraw_pending: Cell::new(false),
            cursor_style: CursorStyle::default(),
        };
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = &platform.surface;
            HydrolysisRenderer::new(surface.adapter(), surface.device(), theme)
        };
        crate::runner::fonts::seed_core(&mut renderer, &fonts);
        renderer.set_window_id(shortcuts.mint_window_id());
        let runtime = RuntimeWindow::new(
            window,
            platform,
            renderer,
            RenderDiagnosticsConfig::from_env(),
        );

        Ok(Box::new(Self {
            env,
            runtime,
            executor,
            wake,
            gpu,
            pending_window_queue,
            a11y: AccessibilitySnapshot::default(),
            platform_views: PlatformViewTable::default(),
            ime: ImeBridge::default(),
            surface_generation: 0,
            frame_deadline_in_nanos: None,
            safe_area,
            presented_once: Cell::new(false),
            ready_logged: Cell::new(false),
            _not_send: std::marker::PhantomData,
        }))
    }

    /// Pushes a metrics snapshot from the host: size/density/refresh/insets
    /// arrive as one coherent unit. A size change becomes a `Resize` event so
    /// the window's frame binding tracks the host.
    pub(crate) fn set_metrics(&mut self, metrics: MetricsSnapshot) {
        let insets_px = metrics.insets_px;
        let density = metrics.density;
        let (size_changed, insets_changed) = {
            let platform = &mut self.runtime.platform;
            let size_changed = platform.metrics.width_px != metrics.width_px
                || platform.metrics.height_px != metrics.height_px
                || platform.metrics.density != metrics.density;
            let insets_changed = platform.metrics.insets_px != insets_px;
            platform.metrics = metrics;
            if size_changed {
                let (w, h) = platform.content_size();
                platform.push_event(InputEvent::Resize {
                    width: w,
                    height: h,
                });
            }
            if insets_changed {
                // The binding is the environment value the window pipeline
                // reads; the write re-lays out through the subscription, and
                // the explicit requests cover the frames before the first read
                // landed one.
                let [leading, top, trailing, bottom] = insets_px;
                let density = density as f32;
                self.safe_area.set(waterui_layout::padding::EdgeInsets::new(
                    top as f32 / density,
                    bottom as f32 / density,
                    leading as f32 / density,
                    trailing as f32 / density,
                ));
                platform.request_redraw();
            }
            (size_changed, insets_changed)
        };
        if size_changed || insets_changed {
            let metrics = &self.runtime.platform.metrics;
            tracing::debug!(
                target: "waterui::hydrolysis::android",
                width = metrics.width_px,
                height = metrics.height_px,
                density = metrics.density,
                insets_px = ?insets_px,
                "wake posted: metrics changed"
            );
        }
        if insets_changed {
            self.runtime.request_refresh();
        }
    }

    /// The one coordinated frame transaction — the scheduler's
    /// Choreographer callback lands here. Executor wakes and input dispatch
    /// run against the last presented geometry; the pump renders only when
    /// the engine's `Next` says there is work, and the outcome tells the
    /// scheduler whether another frame is owed.
    pub(crate) fn on_frame(&mut self) -> FrameOutcome {
        tracing::debug!(target: "waterui::hydrolysis::android", "frame wake");
        self.executor.drain();
        let should_close = handle_input_events(&mut self.runtime, &self.env) || self.should_close();
        let now = Instant::now();
        let deadline = advance_runtime(&mut self.runtime, &self.env, now);
        if self.runtime.mode.is_pending() && self.runtime.platform.surface.is_attached() {
            let executor = self.executor.clone();
            let presented = render_window(&mut self.runtime, &self.env, &mut || executor.drain());
            if presented {
                self.presented_once.set(true);
            }
        }
        // A lost device is fatal for the session — surface it through the
        // host's explicit error callback, not a silently black band.
        if self.runtime.platform.surface.device_loss().is_lost() {
            self.runtime
                .platform
                .bridge
                .fatal_error("hydrolysis android: wgpu device lost");
        }
        if should_close {
            self.runtime.platform.bridge.close_requested();
        }
        super::accessibility::publish_if_pending(self);
        super::platform_views::publish_if_pending(self);

        // Popup windows mounting mid-frame land on the pending queue: the
        // host has no second band to put one on, so this is the explicit
        // unsupported-component error the repository contract wants.
        if !self.pending_window_queue.borrow().is_empty() {
            panic!(
                "hydrolysis android: secondary windows are unsupported — the host mounts exactly one window per session"
            );
        }

        let wants_next_frame =
            self.runtime.mode.is_pending() || self.runtime.platform.take_redraw_pending();
        if self.presented_once.get() && !wants_next_frame && !self.ready_logged.get() {
            tracing::info!(
                target: "waterui::hydrolysis::android",
                "hydrolysis android: first frame presented; ui idle"
            );
            self.ready_logged.set(true);
        }
        // A deadline already elapsed wakes the frame once: its tick then
        // runs at or after the deadline, which is when the recognizer that
        // armed it clears itself.
        let deadline_in_nanos = deadline.map(|at| {
            i64::try_from(
                at.checked_duration_since(now)
                    .unwrap_or_default()
                    .as_nanos(),
            )
            .unwrap_or_default()
        });
        self.frame_deadline_in_nanos = deadline_in_nanos;
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            wants_next_frame,
            deadline_in_nanos,
            should_close,
            "frame outcome"
        );
        FrameOutcome {
            wants_next_frame,
            deadline_in_nanos,
            should_close,
        }
    }

    /// The Kotlin GPU band created or replaced its surface; `generation`
    /// comes from the host's own counter, so a stale `surfaceDestroyed` can
    /// never tear down a newer attachment.
    pub(crate) fn surface_attached_with_generation(
        &mut self,
        native_window: ndk::native_window::NativeWindow,
        width: u32,
        height: u32,
        generation: u64,
    ) -> Result<(), String> {
        self.surface_generation = generation;
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            generation,
            width,
            height,
            "wake posted: surface attached"
        );
        self.runtime
            .platform
            .surface
            .attach(native_window, width, height, generation)
            .map_err(|error| error.to_string())?;
        // A new surface never inherits the old one's presented frame — the
        // next transaction must re-encode and present.
        self.runtime.request_refresh();
        self.runtime.platform.request_redraw();
        Ok(())
    }

    /// The band's size changed on the live surface generation.
    pub(crate) fn surface_resized(
        &mut self,
        width: u32,
        height: u32,
        generation: u64,
    ) -> Result<(), String> {
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            generation,
            width,
            height,
            "wake posted: surface resized"
        );
        self.runtime
            .platform
            .surface
            .resize_for(width, height, generation)
            .map_err(|error| error.to_string())
    }

    /// The band's surface is going away; `generation` names which one, so a
    /// stale destroy does not tear down a newer attachment.
    pub(crate) fn surface_detached(&mut self, generation: u64) {
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            generation,
            "wake posted: surface detached"
        );
        self.runtime.platform.surface.detach_for(generation);
    }

    /// The scheduler's interaction/animation high-refresh demand changed —
    /// routed onto the native window (API 30+).
    pub(crate) fn set_high_refresh_demand(&mut self, fps: Option<f32>) {
        self.runtime.platform.surface.set_high_refresh_demand(fps);
    }

    /// Whether the window's content asked the host to close.
    pub(crate) fn should_close(&self) -> bool {
        self.runtime.window.state.snapshot() == WindowState::Closed
    }

    /// The deadline (ns from now) the last frame transaction left behind.
    pub(crate) fn frame_deadline_in_nanos(&self) -> i64 {
        self.frame_deadline_in_nanos.unwrap_or(-1)
    }
}
