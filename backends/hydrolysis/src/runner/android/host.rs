//! The Android host's window services, executor and frame transaction.
//!
//! `AndroidHostWindow` is the [`PlatformWindow`] the Kotlin `HydrolysisHostView`
//! stands behind: window metrics live here (one coherent snapshot pushed by
//! the host, never read off the GPU attachment), input arrives as pushed
//! events, and `request_redraw` crosses JNI once to post a Choreographer
//! frame. `AndroidSession` is the mounted app: `RuntimeWindow`, the
//! environment and the main-Looper executor, driven by one frame transaction
//! per vsync callback — the plan's "one coordinated frame transaction",
//! never an unconditional tick-then-render pair.
//!
//! The executor keeps the shared channel-queue shape, but its wake writes an
//! `eventfd` that the main `ALooper` is watching instead of a Java callback —
//! wakeups coalesce in the fd and never pay a JNI round-trip per task.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::Arc;

use jni::objects::{GlobalRef, JMethodID, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::{JNIEnv, JavaVM};
use nami::Signal;
use ndk::looper::{FdEvent, ForeignLooper, ThreadLooper};
use waterui::Environment;
use waterui::cursor::CursorStyle;
use waterui::window::WindowState;

use super::accessibility::AccessibilitySnapshot;
use super::gpu::{AndroidGpuContext, AndroidSurface};
use super::ime::ImeBridge;
use super::jni::JniError;
use crate::engine::WidgetTheme;
use crate::platform::{
    GpuSurfaceWindow, InputEvent, PlatformWindow, SurfaceProvider, TextInputState,
    validated_window_frame,
};
use crate::renderer::{
    FontFamilyResolution, HydrolysisRenderer, HydrolysisTextContextMenuMode, MenuShortcutRegistry,
};
use crate::runner::android_executor::AndroidMainThreadExecutor;
use crate::runner::android_methods::{HOST_METHODS, HostMethodId};
use crate::runner::window::{
    RuntimeWindow, advance_runtime, frame_wake_may_post, handle_input_events, render_window,
    reports_ui_idle, wants_next_frame,
};
use crate::runner::{
    RenderDiagnosticsConfig, init_main_thread_executors, install_headless_window_managers,
    install_native_component_hooks, menu_bar,
};
use crate::text::SessionTextEngine;
use crate::time::Instant;

/// One coherent metrics snapshot the host pushes — size, density, font scale,
/// display refresh and the system-bar insets arrive together, never piecemeal.
#[derive(Clone, Debug)]
pub struct MetricsSnapshot {
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
    /// Container-region inset edges in physical px: `[left, top, right,
    /// bottom]` — the system-bar/cutout/window-chrome band of §7.1's
    /// safe-area contract, never the IME.
    pub(crate) container_insets_px: [i32; 4],
    /// Keyboard-region inset edges in physical px — the IME band of §7.1's
    /// contract, reported every animation frame the host's
    /// `WindowInsetsAnimationCompat` callback produces.
    pub(crate) keyboard_insets_px: [i32; 4],
    /// `ViewConfiguration.getScaledTouchSlop()`, in physical px.
    pub(crate) touch_slop_px: f32,
    /// `ViewConfiguration.getScaledMinimumFlingVelocity()`, in physical px/s.
    pub(crate) min_fling_velocity_px: f32,
    /// `ViewConfiguration.getScaledMaximumFlingVelocity()`, in physical px/s.
    pub(crate) max_fling_velocity_px: f32,
    /// `ViewConfiguration.getScrollFriction()` — always positive on a real
    /// host, so the creation-time snapshot's `0.0` reads as "not yet
    /// populated" until the first `nativeSetMetrics` lands.
    pub(crate) scroll_friction: f64,
}

/// The session's JNI handle back into the Kotlin host — a cached `JavaVM`,
/// a global reference to the `HydrolysisSession` object that owns this
/// session, and the `jmethodID`s of every [`HOST_METHODS`] entry, resolved
/// once here. Every call lands on the UI thread (the only thread these
/// callbacks ever run on) through `get_env`/`attach_current_thread`.
pub struct HostBridge {
    vm: JavaVM,
    host_view: GlobalRef,
    /// Cached ids in [`HOST_METHODS`] order — call sites index it by
    /// [`HostMethodId`], so no name or signature literal appears elsewhere.
    methods: Vec<JMethodID>,
}

impl HostBridge {
    /// Resolves every [`HOST_METHODS`] entry with `GetMethodID` on the
    /// session object's class. A host method that went missing — stripped
    /// or renamed by R8 — fails the session's creation naming the method
    /// and its signature, rather than the first call that reaches for it.
    pub(crate) fn new(
        env: &mut JNIEnv,
        vm: JavaVM,
        host_view: GlobalRef,
    ) -> Result<Self, JniError> {
        let class = env.get_object_class(&host_view)?;
        let mut methods = Vec::with_capacity(HOST_METHODS.len());
        for method in HOST_METHODS {
            match env.get_method_id(&class, method.name, method.signature) {
                Ok(id) => methods.push(id),
                Err(error) => {
                    // GetMethodID leaves a pending NoSuchMethodError; the
                    // guard's IllegalStateException is the report that must
                    // reach Kotlin, so the pending one is cleared first.
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_clear();
                    }
                    return Err(JniError(format!(
                        "hydrolysis android: HydrolysisSession is missing \
                         {} {}: {error}",
                        method.name, method.signature
                    )));
                }
            }
        }
        Ok(Self {
            vm,
            host_view,
            methods,
        })
    }

    fn call(&self, method: HostMethodId, args: &[JValue]) {
        match self.vm.get_env() {
            Ok(mut env) => self.call_with_env(&mut env, method, args),
            Err(_) => match self.vm.attach_current_thread() {
                Ok(mut guard) => self.call_with_env(&mut guard, method, args),
                Err(error) => {
                    tracing::error!(
                        target: "waterui::hydrolysis::android",
                        method = HOST_METHODS[method as usize].name,
                        %error,
                        "host callback could not attach a JNI env"
                    );
                }
            },
        }
    }

    fn call_with_env(&self, env: &mut JNIEnv, method: HostMethodId, args: &[JValue]) {
        let jni_args: Vec<jni::sys::jvalue> = args.iter().map(JValue::as_jni).collect();
        // SAFETY: `methods` holds the ids `GetMethodID` resolved on this
        // object's class, in table order; `method` indexes the matching id
        // and every table entry is a void method whose argument list the
        // caller matches to the declared signature.
        if let Err(error) = unsafe {
            env.call_method_unchecked(
                &self.host_view,
                self.methods[method as usize],
                ReturnType::Primitive(Primitive::Void),
                &jni_args,
            )
        } {
            // A pending Java exception (the host's deliberate throw for a
            // fatal GPU error) surfaces to the Kotlin caller as-is — but
            // describe it first, or the next JNI call aborts the process on
            // "called with pending exception" and the real trace never
            // reaches logcat. Other JNI failures are logged, never silently
            // dropped.
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                return;
            }
            tracing::error!(
                target: "waterui::hydrolysis::android",
                method = HOST_METHODS[method as usize].name,
                %error,
                "host callback failed"
            );
        }
    }

    /// Posts a Choreographer frame request on the host view's scheduler.
    pub(crate) fn request_frame(&self) {
        self.call(HostMethodId::RequestRedraw, &[]);
    }

    /// `call` for a single `String` argument — the JSON pushes serialize
    /// into a `jstring` inside the env first.
    fn call_str(&self, method: HostMethodId, json: &str) {
        let Ok(mut env) = self.vm.get_env() else {
            return;
        };
        let Ok(value) = env.new_string(json) else {
            return;
        };
        let jni_args = [JValue::Object(&value).as_jni()];
        // SAFETY: as in `call_with_env` — the cached id matches this
        // method's `(Ljava/lang/String;)V` signature exactly.
        if unsafe {
            env.call_method_unchecked(
                &self.host_view,
                self.methods[method as usize],
                ReturnType::Primitive(Primitive::Void),
                &jni_args,
            )
        }
        .is_err()
            && env.exception_check().unwrap_or(false)
        {
            let _ = env.exception_describe();
        }
    }

    /// `session.onNativeEditingState(json)` — the authoritative editing
    /// state for the connection's `Editable` mirror.
    pub(crate) fn editing_state_changed(&self, json: &str) {
        self.call_str(HostMethodId::EditingState, json);
    }

    /// `session.onNativeCursorAnchorInfo(json)` — the subscribed cursor
    /// anchor info, in logical units.
    pub(crate) fn cursor_anchor_changed(&self, json: &str) {
        self.call_str(HostMethodId::CursorAnchorInfo, json);
    }

    /// `session.onNativeSoftInput(visible)` — shows or hides the soft
    /// keyboard. Candidate geometry and the field's input contract travel on
    /// the editing-state and cursor-anchor pushes instead.
    fn set_soft_input_visible(&self, visible: bool) {
        self.call(HostMethodId::SoftInput, &[JValue::Bool(visible.into())]);
    }

    /// `session.onNativeAccessibilityTreeChanged(json)` — the JSON event
    /// list the semantic diff produced for this publish; the host replays
    /// each entry as the scoped accessibility event it describes.
    #[cfg(feature = "accessibility")]
    pub fn accessibility_tree_changed(&self, events_json: &str) {
        self.call_str(HostMethodId::AccessibilityTreeChanged, events_json);
    }

    /// Marks the published platform-view placement set dirty on the host
    /// side — the registry re-reads `nativePlatformViewFrames` and re-lays
    /// out its slots.
    pub(crate) fn platform_views_changed(&self) {
        self.call(HostMethodId::PlatformViewsChanged, &[]);
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
        let jni_args = [JValue::Object(&message).as_jni()];
        // SAFETY: as in `call_with_env` — the cached id matches
        // `onNativeFatalError`'s `(Ljava/lang/String;)V` signature exactly.
        let _ = unsafe {
            env.call_method_unchecked(
                &self.host_view,
                self.methods[HostMethodId::FatalError as usize],
                ReturnType::Primitive(Primitive::Void),
                &jni_args,
            )
        };
    }

    /// The window's content asked to close — the host finishes the activity.
    fn close_requested(&self) {
        self.call(HostMethodId::CloseRequested, &[]);
    }

    /// `session.onNativeBackAvailable(available)` — the host enables its back
    /// callback only while a navigation stack can accept the pop. A disabled
    /// callback leaves back to the system, which finishes the activity.
    fn set_back_navigation_available(&self, available: bool) {
        self.call(
            HostMethodId::BackAvailable,
            &[JValue::Bool(available.into())],
        );
    }
}

/// The Kotlin host's window on the runner's side: host services only. The
/// GPU attachment is a separate object — this type's [`PlatformWindow`] impl
/// never touches it, and [`GpuSurfaceWindow::surface`] is the only path that
/// hands it to the painter.
pub struct AndroidHostWindow {
    metrics: MetricsSnapshot,
    events: Vec<InputEvent>,
    pub(crate) surface: AndroidSurface,
    /// Shared with the installed frame wake, which posts its own
    /// Choreographer request through it.
    pub(crate) bridge: Rc<HostBridge>,
    /// A redraw the engine asked for that has not yet reached the scheduler;
    /// consumed at the end of the frame transaction as scheduling demand.
    redraw_pending: Cell<bool>,
    /// The Activity is between `onStart` and `onStop` — half of the Android
    /// visibility report; the other half is a live surface below.
    started: bool,
    cursor_style: CursorStyle,
    /// The [`TextInputState::activation`] the soft keyboard was last shown
    /// for; `None` while no field holds focus. The runner syncs text-input
    /// state on every frame, and only a change here reaches the IME.
    soft_input: Option<u64>,
    /// Shared with the installed frame wake: `true` while a wake may post.
    /// Closed inside the frame transaction — a request raised there is
    /// counted into `wants_next_frame` instead — and while occluded, where
    /// it stays armed for the frame visibility restores. Re-derived by
    /// [`Self::sync_frame_wake_gate`]; never written anywhere else, so
    /// [`Self::is_occluded`] stays the one visibility report.
    frame_wake_gate: Rc<Cell<bool>>,
    /// `on_frame` holds this while its frame transaction runs — the gate's
    /// other input is the occlusion report.
    frame_transaction_open: Cell<bool>,
}

impl AndroidHostWindow {
    /// Queues an input event for the next frame transaction's dispatch.
    ///
    /// Host-side `MotionEvent` coordinates arrive in physical pixels while the
    /// retained scene is laid out in logical units — the boundary converts,
    /// exactly as the winit runner's `PhysicalPosition::to_logical` does.
    pub(crate) fn push_event(&mut self, mut event: InputEvent) {
        let density = crate::num_cast::f64_as_f32(self.metrics.density);
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
            is_line_delta: false,
            ..
        } = &mut event
        {
            *dx /= density;
            *dy /= density;
        }
        // Input is a wake, not a pump: the first queued event posts one
        // Choreographer frame that dispatches the batch; the engine's `Next`
        // decides whether anything follows. An occluded session queues the
        // event without the post — it dispatches on the frame visibility
        // restores.
        let wakes = self.events.is_empty() && !self.is_occluded();
        self.events.push(event);
        if wakes {
            tracing::debug!(
                target: "waterui::hydrolysis::android",
                "wake posted: input event queued"
            );
            self.bridge.request_frame();
        }
    }

    pub const fn take_redraw_pending(&self) -> bool {
        self.redraw_pending.replace(false)
    }

    /// Re-derives the installed frame wake's gate: closed while `on_frame`
    /// runs (a request raised inside the transaction is counted into
    /// `wants_next_frame` instead of posting) and while the window is
    /// occluded (the request stays armed for the restore frame). Called
    /// wherever either input can move — the transaction's edges and every
    /// occlusion sync — so the wake never carries a second visibility
    /// report of its own.
    pub(crate) fn sync_frame_wake_gate(&self) {
        self.frame_wake_gate.set(frame_wake_may_post(
            self.frame_transaction_open.get(),
            self.is_occluded(),
        ));
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
            waterui_core::layout::Size::new(
                crate::num_cast::f64_as_f32(logical_width),
                crate::num_cast::f64_as_f32(logical_height),
            ),
        );
        if current != target {
            window.frame.set(target);
        }
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut self.events)
    }

    /// The Android visibility report, from the contract's public APIs: the
    /// Activity's `onStart`/`onStop` state and the `SurfaceHolder`
    /// attachment. A stopped activity or a destroyed surface means nothing
    /// the band could draw reaches the user.
    fn is_occluded(&self) -> bool {
        !self.started || !self.surface.is_attached()
    }

    fn request_redraw(&self) {
        self.redraw_pending.set(true);
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            "wake posted: redraw requested"
        );
        self.bridge.request_frame();
    }

    /// The wake installed on the window's frame signals: posts the
    /// Choreographer frame a request raised outside a transaction needs —
    /// an executor drain, a platform-view callback, an accessibility
    /// action. The gate suppresses it inside `on_frame` and while
    /// occluded; Kotlin's `posted` flag coalesces repeated posts, so the
    /// wake itself posts unconditionally while the gate is open.
    fn frame_wake(&self) -> Rc<dyn Fn()> {
        let gate = Rc::clone(&self.frame_wake_gate);
        let bridge = Rc::clone(&self.bridge);
        Rc::new(move || {
            if gate.get() {
                tracing::debug!(
                    target: "waterui::hydrolysis::android",
                    "wake posted: frame request pending"
                );
                bridge.request_frame();
            }
        })
    }

    fn scale_factor(&self) -> f64 {
        self.metrics.density
    }

    /// The touch-drag scroll gesture's parameters, converted from the
    /// metrics snapshot's physical px to logical units — the fling's spline
    /// model is AOSP `frameworks/base/core/java/android/widget/OverScroller.java`
    /// (`SplineScroller`): `mPhysicalCoeff = GRAVITY_EARTH * 39.37 * ppi *
    /// 0.84` in physical px/s², carried here per logical unit so the
    /// renderer's logical-unit math reproduces the same run.
    fn touch_scroll_config(&self) -> Option<crate::platform::TouchScrollConfig> {
        let metrics = &self.metrics;
        // The session's creation-time snapshot carries no `ViewConfiguration`
        // values — friction is always positive on a real push — and a
        // window still waiting on its first metrics dispatches no input
        // these would apply to.
        if metrics.scroll_friction <= 0.0 {
            return None;
        }
        let density = metrics.density;
        let ppi = density * 160.0;
        let physical_coeff_px = 9.806_65 * 39.37 * ppi * 0.84;
        Some(crate::platform::TouchScrollConfig {
            touch_slop: crate::num_cast::f64_as_f32(f64::from(metrics.touch_slop_px) / density),
            min_fling_velocity: crate::num_cast::f64_as_f32(
                f64::from(metrics.min_fling_velocity_px) / density,
            ),
            max_fling_velocity: crate::num_cast::f64_as_f32(
                f64::from(metrics.max_fling_velocity_px) / density,
            ),
            fling: crate::platform::FlingDeceleration {
                physical_coeff: physical_coeff_px / density,
                friction: metrics.scroll_friction,
            },
        })
    }

    fn refresh_rate_hz(&self) -> Option<f64> {
        self.metrics.refresh_hz
    }

    fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
        let soft_input = state.map(|state| state.activation);
        if soft_input == self.soft_input {
            return;
        }
        self.soft_input = soft_input;
        // Focus gained, or a press on the focused field: show. Focus lost:
        // hide.
        self.bridge.set_soft_input_visible(soft_input.is_some());
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

/// Owns the `ALooper` registration that drains an
/// [`AndroidMainThreadExecutor`]'s wake fd. Dropping unregisters the fd, so
/// the looper can never fire into a dead executor.
struct ExecutorWake {
    looper: ForeignLooper,
    /// A share of the executor's wake fd, keeping it open for the looper for
    /// exactly as long as the registration lives.
    fd: Arc<OwnedFd>,
}

/// The local-reference capacity the wake drain's frame reserves. The
/// `PushLocalFrame` contract is "at least this many", not a bound, so the
/// usual JNI default covers a drain's bridge calls.
const WAKE_DRAIN_LOCALS: i32 = 32;

impl ExecutorWake {
    /// Registers the executor's own wake fd's drain callback on this thread's
    /// looper — must be the UI thread (its main looper exists already). The
    /// executor has no other wake fd, so a wake written while the looper
    /// idles is always observed on the registered fd.
    fn register(executor: &AndroidMainThreadExecutor) -> Result<Self, JniError> {
        let fd = Arc::clone(executor.wake_fd());
        let looper = ThreadLooper::for_thread().ok_or_else(|| {
            JniError("hydrolysis android: no ALooper on the UI thread".to_owned())
        })?;
        let drain = executor.clone();
        looper
            .add_fd_with_callback(fd.as_fd(), FdEvent::INPUT, move |fd, _events| {
                let mut count: u64 = 0;
                // SAFETY: fd is the registered eventfd; eventfd_read consumes
                // every coalesced wake at once.
                unsafe {
                    libc::eventfd_read(fd.as_raw_fd(), &raw mut count);
                }
                // The looper's fd callback is not a Java-to-native entry
                // point (`MessageQueue.nativePollOnce` is `@CriticalNative`
                // and pushes no local frame), so locals the drained tasks'
                // JNI calls create would never be freed. One frame around the
                // drain gives these tasks the scope a JNI entry gives the
                // frame callback's drain.
                let drained = super::jni::java_vm()
                    .get_env()
                    .expect(
                        "hydrolysis android: the looper callback runs on the attached UI thread",
                    )
                    .with_local_frame(WAKE_DRAIN_LOCALS, |_env| Ok::<_, JniError>(drain.drain()))
                    .expect("hydrolysis android: PushLocalFrame around the executor drain failed");
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
pub struct FrameOutcome {
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
pub struct AndroidSession {
    pub(crate) env: Environment,
    pub(crate) runtime: RuntimeWindow<AndroidHostWindow>,
    executor: AndroidMainThreadExecutor,
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
    /// The platform-view sink the window's `PlatformView` leaves record into;
    /// the published table is serialized for the Kotlin registry.
    pub(crate) platform_views: crate::platform_view::PlatformViewSink,
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
    /// The live window keyboard-area binding `WindowKeyboardArea` wraps —
    /// written on the same host pushes, IME animation frames included.
    keyboard_area: nami::Binding<waterui_layout::padding::EdgeInsets>,
    /// A frame reached the surface at least once; together with an idle
    /// `wants_next_frame` it arms the one-shot readiness log the device's
    /// idle-CPU sampling waits on.
    presented_once: Cell<bool>,
    /// The readiness line was already logged — it fires exactly once per
    /// session so a later busy window cannot re-arm it.
    ready_logged: Cell<bool>,
    /// The last value reported to the host's back callback. Starts disabled,
    /// matching the callback the activity registers.
    back_navigation_available: bool,
    /// Sessions are owned and driven on the UI thread only.
    _not_send: std::marker::PhantomData<*const ()>,
}

/// What the load hook creates once per UI thread: the executor every
/// session shares, its looper registration, and the process environment
/// carrying the inspector's recorders. The `Kotlin` side holds the pointer
/// for the life of the process and hands it to each new session, so a
/// second session never strands its tasks on a dead executor and the
/// inspector is started once per process.
pub struct UiThreadServices {
    /// The one executor per UI thread — sessions drain clones of it; the
    /// queue itself is shared.
    executor: AndroidMainThreadExecutor,
    /// Keeps the wake fd registered with the main looper for the process's
    /// life; the looper owns no session state.
    #[expect(
        dead_code,
        reason = "held for its Drop side effect — unregister the wake fd"
    )]
    wake: ExecutorWake,
    /// An environment that carries the inspector's runtime and recorders.
    /// Each session layers its own environment on it, so the one inspector
    /// the process started is visible from every session's `env` without a
    /// second runtime.
    env: Environment,
}

impl UiThreadServices {
    /// Runs at `nativeUiThreadServices` — the load hook, once per process on
    /// the UI thread (its main looper must already exist).
    pub(crate) fn init() -> Result<Self, JniError> {
        let inspector = init_main_thread_executors();
        let inspector_probe = inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe);
        // SAFETY: NONBLOCK + CLOEXEC keep a saturated counter from stalling
        // the looper and the fd out of child processes.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(JniError(format!(
                "hydrolysis android: eventfd failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: fd >= 0 checked above.
        let executor = AndroidMainThreadExecutor::new(unsafe { OwnedFd::from_raw_fd(fd) });
        let wake = ExecutorWake::register(&executor)?;
        let _ = executor_core::try_init_local_executor(
            waterui::task::monitored_local_executor_with_probes(
                executor.clone(),
                waterui::task::RefreshRate::HEADLESS,
                inspector_probe,
            ),
        );
        let mut env = Environment::new();
        waterui::inspector::install(&mut env, inspector);
        Ok(Self {
            executor,
            wake,
            env,
        })
    }
}

impl AndroidSession {
    /// Mounts the registered app on the host view: builds the environment,
    /// GPU context and the runtime window. `services` is the UI-thread
    /// bundle the load hook created — the session clones its executor and
    /// layers its environment over the process's.
    ///
    /// `metrics` is the host's snapshot at creation time — content size and
    /// scale exist from the start, before the surface band attaches.
    pub(crate) fn create(
        env: &mut JNIEnv,
        vm: JavaVM,
        host_view: GlobalRef,
        metrics: MetricsSnapshot,
        services: &'static UiThreadServices,
    ) -> Result<Box<Self>, JniError> {
        let bridge = HostBridge::new(env, vm, host_view)?;
        let executor = services.executor.clone();

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
            // Android kills the process without notice, so the termination
            // hooks are never called and the machine is never started.
            termination: _,
        } = app.into_parts();
        let mut env = env
            .extending(waterui_graphics::SceneViewMergeToParent)
            .layered_on(&services.env);
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
        let fonts = crate::text::fonts::platform_collection(&env);
        fonts.clone().install(&mut env);
        let shortcuts = env
            .get::<MenuShortcutRegistry>()
            .expect("install_headless_window_managers seeds MenuShortcutRegistry")
            .clone();
        let safe_area = nami::binding(waterui_layout::padding::EdgeInsets::default());
        env.insert(crate::platform::WindowSafeArea(safe_area.clone()));
        let keyboard_area = nami::binding(waterui_layout::padding::EdgeInsets::default());
        env.insert(crate::platform::WindowKeyboardArea(keyboard_area.clone()));
        // The platform-view sink `PlatformView` leaves record their frames
        // into; the published table is what `nativePlatformViewFrames` serves.
        let platform_views = crate::platform_view::PlatformViewSink::new();
        env.insert(platform_views.clone());

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
            surface: AndroidSurface::new(gpu.clone()),
            bridge: Rc::new(bridge),
            redraw_pending: Cell::new(false),
            started: false,
            cursor_style: CursorStyle::default(),
            soft_input: None,
            // The session mounts parked — hidden until `onStart` and the
            // surface attach report it visible — so the wake's gate opens
            // on the first occlusion sync, not at construction.
            frame_wake_gate: Rc::new(Cell::new(false)),
            frame_transaction_open: Cell::new(false),
        };
        platform.apply_properties(&window);
        let mut renderer = HydrolysisRenderer::with_engine(
            theme,
            SessionTextEngine::from_collection(&fonts, FontFamilyResolution::Lenient),
        );
        renderer.set_window_id(shortcuts.mint_window_id());
        renderer.set_window_closable(window.closable);
        let mut runtime = RuntimeWindow::new(
            window,
            platform,
            renderer,
            RenderDiagnosticsConfig::from_env(),
        );
        // The session mounts parked: the activity's `onStart` and the
        // surface band's attach are the reports that unpark it.
        runtime.set_hidden(runtime.platform.is_occluded());
        runtime.platform.sync_frame_wake_gate();

        Ok(Box::new(Self {
            env,
            runtime,
            executor,
            gpu,
            pending_window_queue,
            a11y: AccessibilitySnapshot::default(),
            platform_views,
            ime: ImeBridge::default(),
            surface_generation: 0,
            frame_deadline_in_nanos: None,
            safe_area,
            keyboard_area,
            presented_once: Cell::new(false),
            ready_logged: Cell::new(false),
            back_navigation_available: false,
            _not_send: std::marker::PhantomData,
        }))
    }

    /// Pushes a metrics snapshot from the host: size/density/refresh/insets
    /// arrive as one coherent unit. A size change becomes a `Resize` event so
    /// the window's frame binding tracks the host.
    pub(crate) fn set_metrics(&mut self, metrics: MetricsSnapshot) {
        let container_insets_px = metrics.container_insets_px;
        let keyboard_insets_px = metrics.keyboard_insets_px;
        let density = metrics.density;
        let (size_changed, container_changed, keyboard_changed) = {
            let platform = &mut self.runtime.platform;
            let size_changed = platform.metrics.width_px != metrics.width_px
                || platform.metrics.height_px != metrics.height_px
                || platform.metrics.density.to_bits() != metrics.density.to_bits();
            let container_changed = platform.metrics.container_insets_px != container_insets_px;
            let keyboard_changed = platform.metrics.keyboard_insets_px != keyboard_insets_px;
            platform.metrics = metrics;
            if size_changed {
                let (w, h) = platform.content_size();
                platform.push_event(InputEvent::Resize {
                    width: w,
                    height: h,
                });
            }
            // Each binding write re-lays out through its own subscription, so
            // a region only re-sets when its own value moved — an IME
            // progress frame alone does not re-publish the container band.
            if container_changed || keyboard_changed {
                let density = crate::num_cast::f64_as_f32(density);
                let to_insets = |px: [i32; 4]| {
                    let [leading, top, trailing, bottom] = px;
                    waterui_layout::padding::EdgeInsets::new(
                        crate::num_cast::i32_as_f32(top) / density,
                        crate::num_cast::i32_as_f32(bottom) / density,
                        crate::num_cast::i32_as_f32(leading) / density,
                        crate::num_cast::i32_as_f32(trailing) / density,
                    )
                };
                if container_changed {
                    self.safe_area.set(to_insets(container_insets_px));
                }
                if keyboard_changed {
                    self.keyboard_area.set(to_insets(keyboard_insets_px));
                }
            }
            (size_changed, container_changed, keyboard_changed)
        };
        let insets_changed = container_changed || keyboard_changed;
        if size_changed || insets_changed {
            let metrics = &self.runtime.platform.metrics;
            tracing::debug!(
                target: "waterui::hydrolysis::android",
                width = metrics.width_px,
                height = metrics.height_px,
                density = metrics.density,
                container_insets_px = ?container_insets_px,
                keyboard_insets_px = ?keyboard_insets_px,
                "wake posted: metrics changed"
            );
        }
        // The insets' binding writes above mark their subscribers' nodes —
        // a `FrameSignals` request whose host wake posts the frame itself
        // (issue #2286) — so an insets change needs no explicit frame
        // request, and one nothing subscribes correctly schedules nothing.
    }

    /// The hosting Activity crossed `onStart`/`onStop` — Android's
    /// visibility signal. Stopping parks the pump even when the surface
    /// survives; starting posts the single restore frame when the band
    /// can present again.
    pub(crate) fn set_visible(&mut self, started: bool) {
        self.runtime.platform.started = started;
        self.runtime.platform.sync_frame_wake_gate();
        // Android has no about-to-wait pass to notice an armed mode — the
        // restore frame needs the explicit Choreographer post.
        self.runtime.sync_occlusion_and_post_restore();
    }

    /// Tells the host when the rendered frame's back-target answer changes.
    ///
    /// The callback starts disabled. A disabled callback leaves back to the
    /// system, which finishes the activity with its own predictive animation.
    fn sync_back_navigation_available(&mut self) {
        let available = self.runtime.renderer.has_back_navigation_target();
        if available == self.back_navigation_available {
            return;
        }
        self.back_navigation_available = available;
        self.runtime
            .platform
            .bridge
            .set_back_navigation_available(available);
    }

    /// The one coordinated frame transaction — the scheduler's
    /// Choreographer callback lands here. Executor wakes and input dispatch
    /// run against the last presented geometry; the pump renders only when
    /// the engine's `Next` says there is work, and the outcome tells the
    /// scheduler whether another frame is owed.
    pub(crate) fn on_frame(&mut self) -> FrameOutcome {
        tracing::debug!(target: "waterui::hydrolysis::android", "frame wake");
        // The transaction is open: a frame request recorded from now on
        // fires no wake — its scheduling is counted into `wants_next_frame`
        // below instead of posting into a frame already running.
        self.runtime.platform.frame_transaction_open.set(true);
        self.runtime.platform.sync_frame_wake_gate();
        self.executor.drain();
        let should_close = handle_input_events(&mut self.runtime, &self.env) || self.should_close();
        let now = Instant::now();
        let deadline = advance_runtime(&mut self.runtime, &self.env, now);
        let mut flushed = false;
        if self.runtime.mode.is_pending()
            && self.runtime.platform.surface.is_attached()
            && !self.runtime.is_hidden()
        {
            let executor = self.executor.clone();
            let presented = render_window(&mut self.runtime, &self.env, &mut || executor.drain());
            flushed = true;
            if presented {
                self.presented_once.set(true);
            }
            self.sync_back_navigation_available();
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
        super::platform_views::publish_if_pending(self, flushed);
        self.editing_sync();

        // Popup windows mounting mid-frame land on the pending queue: the
        // host has no second band to put one on, so this is the explicit
        // unsupported-component error the repository contract wants.
        assert!(
            self.pending_window_queue.borrow().is_empty(),
            "hydrolysis android: secondary windows are unsupported — the host mounts exactly one window per session"
        );

        // A hidden session reports no continuation: the armed mode stays
        // armed for the restore frame, but the scheduler must not keep
        // posting wakes into a parked pump.
        let redraw_pending = self.runtime.platform.take_redraw_pending();
        // The signals' pending count covers a frame request raised inside
        // this transaction after the pump drained the flags: its wake was
        // suppressed by the gate, so this is its only path to the frame it
        // needs. One raised while hidden stays armed for the restore frame.
        let wants_next_frame = wants_next_frame(
            self.runtime.is_hidden(),
            self.runtime.mode.is_pending(),
            redraw_pending,
            self.runtime.renderer.has_pending_frame_request(),
        );
        if reports_ui_idle(
            self.presented_once.get(),
            wants_next_frame,
            self.runtime.is_hidden(),
        ) && !self.ready_logged.get()
        {
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
        // The transaction closes: the wake's gate reopens for the requests
        // the next arrival records.
        self.runtime.platform.frame_transaction_open.set(false);
        self.runtime.platform.sync_frame_wake_gate();
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
        // next transaction must re-encode and present. Attaching while
        // `started` can un-hide a session parked on a missing surface; a
        // stale `surfaceCreated` delivered after `onStop` attaches nothing
        // the user sees, and `started` stays the lifecycle's report.
        self.runtime.request_refresh();
        self.runtime.platform.sync_frame_wake_gate();
        self.runtime.sync_occlusion_and_post_restore();
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
            .map_err(|error| error.to_string())?;
        // A surface parked on a zero-size attach configures here — the
        // resize that gave the band a real extent can be its un-hide, and
        // the Choreographer post is the only wake the restore frame gets.
        self.runtime.platform.sync_frame_wake_gate();
        self.runtime.sync_occlusion_and_post_restore();
        Ok(())
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
        // No surface means nothing to present into: the pump parks until
        // the band re-attaches or the activity's start report unhides it.
        self.runtime.platform.sync_frame_wake_gate();
        self.runtime.sync_occlusion();
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
