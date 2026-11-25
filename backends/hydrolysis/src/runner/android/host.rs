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
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use jni::Env;
use jni::objects::{Global, JMethodID, JObject, JValue};
use jni::signature::{MethodSignature, Primitive, ReturnType, RuntimeMethodSignature};
use jni::strings::JNIString;
use jni::vm::JavaVM;
use jni::{jni_sig, jni_str};
use nami::{Binding, Signal};
use ndk::looper::{FdEvent, ForeignLooper, ThreadLooper};
use waterui::Environment;
#[cfg(feature = "menu")]
use waterui::app::AppMenuBar;
use waterui::cursor::CursorStyle;
use waterui::theme::ColorScheme;
use waterui::window::WindowState;
use waterui_backend_core::gesture::DEFAULT_LONG_PRESS_DURATION;
use waterui_core::layout::LayoutDirection;
use waterui_core::{Retain, contract_violation};
#[cfg(hydrolysis_gpu_painter)]
use waterui_frame_rate::{FrameRate, combine};
use waterui_structural_transition::ReduceMotion;

#[cfg(hydrolysis_gpu_painter)]
use super::gpu::RefreshDemand;

use super::accessibility::AccessibilitySnapshot;
#[cfg(hydrolysis_gpu_painter)]
use super::gpu::{AndroidGpuContext, AndroidSurface};
#[cfg(any(feature = "text-field", feature = "secure-field"))]
use super::ime::ImeBridge;
use super::jni::JniError;
#[cfg(hydrolysis_gpu_painter)]
use crate::platform::PaintedWindow;
use crate::platform::{InputEvent, PlatformWindow, TextInputState, validated_window_frame};
#[cfg(hydrolysis_gpu_painter)]
use crate::renderer::FontFamilyResolution;
#[cfg(hydrolysis_gpu_painter)]
use crate::renderer::HydrolysisRenderer;
#[cfg(any(feature = "text-field", feature = "secure-field"))]
use crate::renderer::HydrolysisTextContextMenuMode;
use crate::runner::android_methods::{HOST_METHODS, HostMethodId};
use crate::runner::android_metrics::InsetsMetrics;
use crate::runner::host_executor::{AndroidExecutor, EventFdWake, UiThreadPoster, redraw_wake};
#[cfg(hydrolysis_inspector)]
use crate::runner::init_main_thread_executors;
#[cfg(any(test, hydrolysis_platform_services))]
use crate::runner::install_platform_services;
#[cfg(feature = "menu")]
use crate::runner::menu_bar;
use crate::runner::window::split_open_and_watch_dormant;
use crate::runner::window::{
    FrameHost, FrameTransaction, RuntimeWindow, reports_ui_idle, run_frame_transaction,
};
#[cfg(hydrolysis_gpu_painter)]
use crate::runner::window::{WindowFrame, render_window};
use crate::runner::{
    RenderDiagnosticsConfig, install_core_window_openers, install_window_registries,
    queue_window_opener,
};
#[cfg(hydrolysis_gpu_painter)]
use crate::text::ParleyEngine;
use crate::theme::WidgetTheme;
use crate::time::Instant;
#[cfg(hydrolysis_hwui)]
use cherenkov_record::SurfaceId;

/// A frame reply wakes the host without requesting another render.
#[cfg(hydrolysis_gpu_painter)]
struct FrameCompletionWake {
    poster: UiThreadPoster<EventFdWake>,
    bridge: Arc<HostBridge>,
}

#[cfg(hydrolysis_gpu_painter)]
impl std::task::Wake for FrameCompletionWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let bridge = Arc::clone(&self.bridge);
        self.poster.post(move || bridge.frame_completed());
    }
}

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
    /// `ViewConfiguration.getDoubleTapTimeout()`, in milliseconds — `0`
    /// until the first `nativeSetMetrics` lands, like `scroll_friction`.
    pub(crate) double_tap_timeout_ms: u32,
    /// `ViewConfiguration.getScaledDoubleTapSlop()`, in physical px.
    pub(crate) double_tap_slop_px: f32,
    /// `ViewConfiguration.getLongPressTimeout()`, in milliseconds — `0`
    /// until the first `nativeSetMetrics` lands, like `scroll_friction`.
    pub(crate) long_press_timeout_ms: u32,
}

impl MetricsSnapshot {
    /// The metrics value `set_metrics` compares for the container region
    /// — the pushed pixel edges, the density AND the direction the
    /// region's logical insets derive from, as one value.
    const fn container_metrics(&self, direction: LayoutDirection) -> InsetsMetrics {
        InsetsMetrics::new(self.container_insets_px, self.density, direction)
    }

    /// The same for the keyboard region.
    const fn keyboard_metrics(&self, direction: LayoutDirection) -> InsetsMetrics {
        InsetsMetrics::new(self.keyboard_insets_px, self.density, direction)
    }
}

/// The session's JNI handle back into the Kotlin host — a cached `JavaVM`,
/// a global reference to the `HydrolysisSession` object that owns this
/// session, and the `jmethodID`s of every [`HOST_METHODS`] entry, resolved
/// once here. Every call lands on the UI thread (the only thread these
/// callbacks ever run on) through `attach_current_thread`.
pub struct HostBridge {
    vm: JavaVM,
    /// Shared, not owned: the Android system-webview controller installs
    /// itself with an `Arc::clone` of this same global, so one JNI global
    /// covers the `HydrolysisSession` object.
    host_view: Arc<Global<JObject<'static>>>,
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
        env: &mut Env,
        vm: JavaVM,
        host_view: Global<JObject<'static>>,
    ) -> Result<Self, JniError> {
        let class = env.get_object_class(host_view.as_obj())?;
        let mut methods = Vec::with_capacity(HOST_METHODS.len());
        for method in HOST_METHODS {
            let signature = RuntimeMethodSignature::from_str(method.signature)?;
            match env.get_method_id(
                &class,
                JNIString::new(method.name),
                MethodSignature::from(&signature),
            ) {
                Ok(id) => methods.push(id),
                Err(error) => {
                    // GetMethodID leaves a pending NoSuchMethodError; the
                    // guard's IllegalStateException is the report that must
                    // reach Kotlin, so the pending one is cleared first.
                    if env.exception_check() {
                        env.exception_clear();
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
            host_view: Arc::new(host_view),
            methods,
        })
    }

    fn call(&self, method: HostMethodId, args: &[JValue]) {
        let result: Result<(), jni::errors::Error> = self.vm.attach_current_thread(|env| {
            self.call_with_env(env, method, args);
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                method = HOST_METHODS[method as usize].name,
                %error,
                "host callback could not attach a JNI env"
            );
        }
    }

    fn call_with_env(&self, env: &mut Env, method: HostMethodId, args: &[JValue]) {
        let jni_args: Vec<jni::sys::jvalue> = args.iter().map(JValue::as_jni).collect();
        // SAFETY: `methods` holds the ids `GetMethodID` resolved on this
        // object's class, in table order; `method` indexes the matching id
        // and every table entry is a void method whose argument list the
        // caller matches to the declared signature.
        if let Err(error) = unsafe {
            env.call_method_unchecked(
                self.host_view.as_obj(),
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
            if env.exception_check() {
                env.exception_describe();
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

    #[cfg(hydrolysis_gpu_painter)]
    fn frame_completed(&self) {
        self.call(HostMethodId::FrameCompleted, &[]);
    }

    /// The one query the bridge makes back into Kotlin: whether a mounted
    /// platform-view child holds UI focus right now. A pull — the focus
    /// listener that used to push this re-entered the session synchronously
    /// from `requestFocus`/`removeView` on the host's own paths.
    pub(crate) fn platform_view_focus_inside(&self) -> bool {
        let method = self.methods[HostMethodId::PlatformViewFocus as usize];
        self.vm
            .with_top_local_frame(|env| -> jni::errors::Result<bool> {
                // SAFETY: `methods` holds the ids `GetMethodID` resolved on
                // this object's class, in table order; `PlatformViewFocus`'s
                // id matches the declared `()Z` signature.
                unsafe {
                    env.call_method_unchecked(
                        self.host_view.as_obj(),
                        method,
                        ReturnType::Primitive(Primitive::Boolean),
                        &[],
                    )
                }?
                .z()
            })
            .unwrap_or_else(|error| {
                tracing::error!(
                    target: "waterui::hydrolysis::android",
                    %error,
                    "the platform-view focus pull failed on the UI thread, or \
                     onNativePlatformViewFocus threw; focus is reported outside the platform views"
                );
                false
            })
    }

    /// `call` for a single `String` argument — the JSON pushes serialize
    /// into a `jstring` inside the env first, and the dispatch itself runs
    /// through `call_with_env`'s error path.
    fn call_str(&self, method: HostMethodId, json: &str) {
        let result: Result<(), jni::errors::Error> = self.vm.with_top_local_frame(|env| {
            let value = env.new_string(json)?;
            self.call_with_env(env, method, &[JValue::Object(value.as_ref())]);
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                method = HOST_METHODS[method as usize].name,
                %error,
                "host callback could not attach a JNI env"
            );
        }
    }

    /// `session.onNativeEditingState(json)` — the authoritative editing
    /// state for the connection's `Editable` mirror.
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    pub(crate) fn editing_state_changed(&self, json: &str) {
        self.call_str(HostMethodId::EditingState, json);
    }

    /// `session.onNativeCursorAnchorInfo(json)` — the subscribed cursor
    /// anchor info, in logical units.
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    pub(crate) fn cursor_anchor_changed(&self, json: &str) {
        self.call_str(HostMethodId::CursorAnchorInfo, json);
    }

    /// `session.onNativeSoftInput(visible)` — shows or hides the soft
    /// keyboard. Candidate geometry and the field's input contract travel on
    /// the editing-state and cursor-anchor pushes instead.
    fn set_soft_input_visible(&self, visible: bool) {
        self.call(HostMethodId::SoftInput, &[JValue::Bool(visible)]);
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

    /// `session.onNativeBandShow(view, colorSpace, paintedRects)` — the band
    /// `View` above the platform view whose placement id is `view` shows the
    /// newest image its `ImageReader` holds across the packed device-pixel
    /// rectangles `painted_rects`, encoded in `color_space`
    /// (water-rs/waterui#2225).
    #[cfg(hydrolysis_gpu_painter)]
    fn band_show(&self, view: u64, color_space: &str, painted_rects: &[[u32; 4]]) {
        let result: Result<(), jni::errors::Error> = self.vm.with_top_local_frame(|env| {
            let name = env.new_string(color_space)?;
            let mut flat = Vec::with_capacity(painted_rects.len() * 4);
            for rect in painted_rects {
                for &component in rect {
                    flat.push(i32::try_from(component).unwrap_or(i32::MAX));
                }
            }
            let rects_array = env.new_int_array(flat.len())?;
            rects_array.set_region(env, 0, &flat)?;
            self.call_with_env(
                env,
                HostMethodId::BandShow,
                &[
                    JValue::Long(crate::num_cast::u64_as_i64(view)),
                    JValue::Object(name.as_ref()),
                    JValue::Object(rects_array.as_ref()),
                ],
            );
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                method = HOST_METHODS[HostMethodId::BandShow as usize].name,
                %error,
                "host callback could not attach a JNI env"
            );
        }
    }

    /// `session.onNativeBandClear(view)` — the band `View` above the
    /// platform view whose placement id is `view` shows nothing
    /// (water-rs/waterui#2225).
    #[cfg(hydrolysis_gpu_painter)]
    fn band_clear(&self, view: u64) {
        self.call(
            HostMethodId::BandClear,
            &[JValue::Long(crate::num_cast::u64_as_i64(view))],
        );
    }

    /// The window's content asked to close — the host finishes the activity.
    fn close_requested(&self) {
        self.call(HostMethodId::CloseRequested, &[]);
    }

    /// `session.onNativeBackAvailable(available)` — the host enables its back
    /// callback only while a navigation stack can accept the pop. A disabled
    /// callback leaves back to the system, which finishes the activity.
    #[cfg(feature = "navigation")]
    fn set_back_navigation_available(&self, available: bool) {
        self.call(HostMethodId::BackAvailable, &[JValue::Bool(available)]);
    }

    /// `session.onNativeTranslucency(translucent, blurBehind, blurRadiusPx)` —
    /// the window's compositing request: a translucent window and surface
    /// when `translucent` holds, the platform behind-window blur at
    /// `blurRadiusPx` device pixels while `blurBehind` does.
    fn set_translucency(&self, translucent: bool, blur_behind: bool, blur_radius_px: i32) {
        self.call(
            HostMethodId::Translucency,
            &[
                JValue::Bool(translucent),
                JValue::Bool(blur_behind),
                JValue::Int(blur_radius_px),
            ],
        );
    }
}

/// The `android.graphics.ColorSpace.Named` constant a band's hardware
/// bitmap is tagged with for images its swapchain encoded in `space`
/// (water-rs/waterui#2225).
///
/// `None` for a space `ColorSpace.Named` cannot name — a broken contract,
/// since the band would show its pixels in the wrong space; the caller
/// shows nothing for the band then.
#[cfg(hydrolysis_gpu_painter)]
fn color_space_name(space: wgpu::SurfaceColorSpace) -> Option<&'static str> {
    match space {
        wgpu::SurfaceColorSpace::Srgb => Some("SRGB"),
        wgpu::SurfaceColorSpace::DisplayP3 => Some("DISPLAY_P3"),
        wgpu::SurfaceColorSpace::ExtendedSrgb => Some("EXTENDED_SRGB"),
        wgpu::SurfaceColorSpace::ExtendedSrgbLinear => Some("LINEAR_EXTENDED_SRGB"),
        wgpu::SurfaceColorSpace::Auto
        | wgpu::SurfaceColorSpace::Bt2100Pq
        | wgpu::SurfaceColorSpace::Bt2100Hlg
        | wgpu::SurfaceColorSpace::ExtendedDisplayP3 => {
            contract_violation!(
                "a band's swapchain negotiated {space:?}, which android.graphics.ColorSpace.Named \
                 cannot name; the band shows nothing"
            );
            None
        }
    }
}

/// Lifecycle subscriber registered on [`AndroidHostBridge`].
type LifecycleListener = Rc<dyn Fn(&mut Env<'_>, crate::android_content::AndroidLifecycle)>;

#[derive(Default)]
struct AndroidLifecycleHub {
    started: bool,
    resumed: bool,
    next_token: u64,
    listeners: Vec<(u64, LifecycleListener)>,
}

impl AndroidLifecycleHub {
    fn subscribe(&mut self, listener: LifecycleListener) -> u64 {
        self.next_token += 1;
        let token = self.next_token;
        self.listeners.push((token, listener));
        token
    }

    fn unsubscribe(&mut self, token: u64) {
        self.listeners.retain(|(id, _)| *id != token);
    }

    fn snapshot(&self) -> Vec<LifecycleListener> {
        self.listeners
            .iter()
            .map(|(_, listener)| Rc::clone(listener))
            .collect()
    }
}

/// The session handle installed in [`Environment`] for [`AndroidView`](crate::AndroidView)
/// leaves: resolves host `Context` wrappers, registers platform-view instances,
/// provides cached `android.view.View` measurement/invalidation JNI ids, and
/// dispatches Activity lifecycle transitions.
#[derive(Clone)]
pub struct AndroidHostBridge {
    vm: JavaVM,
    session: Arc<Global<JObject<'static>>>,
    acquire_instance_context: JMethodID,
    register_instance: JMethodID,
    unregister_instance: JMethodID,
    view_post_invalidate: JMethodID,
    lifecycle: Rc<RefCell<AndroidLifecycleHub>>,
}

impl AndroidHostBridge {
    fn resolve(env: &mut Env, bridge: &HostBridge) -> Result<Self, JniError> {
        let session_class = env.get_object_class(bridge.host_view.as_obj())?;
        let acquire_instance_context = env.get_method_id(
            &session_class,
            jni_str!("acquireInstanceContext"),
            jni_sig!("(J)Landroid/content/Context;"),
        )?;
        let register_instance = env.get_method_id(
            &session_class,
            jni_str!("registerPlatformViewInstance"),
            jni_sig!("(JLandroid/view/View;)V"),
        )?;
        let unregister_instance = env.get_method_id(
            &session_class,
            jni_str!("unregisterPlatformViewInstance"),
            jni_sig!("(J)V"),
        )?;
        let view_class = env.find_class(jni_str!("android/view/View"))?;
        let view_post_invalidate =
            env.get_method_id(&view_class, jni_str!("postInvalidate"), jni_sig!("()V"))?;

        Ok(Self {
            vm: bridge.vm.clone(),
            session: Arc::clone(&bridge.host_view),
            acquire_instance_context,
            register_instance,
            unregister_instance,
            view_post_invalidate,
            lifecycle: Rc::new(RefCell::new(AndroidLifecycleHub::default())),
        })
    }

    pub(crate) const fn vm(&self) -> &JavaVM {
        &self.vm
    }

    pub(crate) fn acquire_instance_context<'local>(
        &self,
        env: &mut Env<'local>,
        id: u64,
    ) -> jni::errors::Result<JObject<'local>> {
        // SAFETY: `acquire_instance_context` was resolved on `self.session`'s
        // class with signature `(J)Landroid/content/Context;`.
        unsafe {
            env.call_method_unchecked(
                self.session.as_obj(),
                self.acquire_instance_context,
                ReturnType::Object,
                &[JValue::Long(crate::num_cast::u64_as_i64(id)).as_jni()],
            )
        }?
        .l()
    }

    pub(crate) fn register_instance(
        &self,
        env: &mut Env<'_>,
        id: u64,
        view: &JObject<'_>,
    ) -> jni::errors::Result<()> {
        // SAFETY: `register_instance` was resolved on `self.session`'s class
        // with signature `(JLandroid/view/View;)V`.
        unsafe {
            env.call_method_unchecked(
                self.session.as_obj(),
                self.register_instance,
                ReturnType::Primitive(Primitive::Void),
                &[
                    JValue::Long(crate::num_cast::u64_as_i64(id)).as_jni(),
                    JValue::Object(view).as_jni(),
                ],
            )
        }?;
        Ok(())
    }

    pub(crate) fn unregister_instance(
        &self,
        env: &mut Env<'_>,
        id: u64,
    ) -> jni::errors::Result<()> {
        // SAFETY: `unregister_instance` was resolved on `self.session`'s
        // class with signature `(J)V`.
        unsafe {
            env.call_method_unchecked(
                self.session.as_obj(),
                self.unregister_instance,
                ReturnType::Primitive(Primitive::Void),
                &[JValue::Long(crate::num_cast::u64_as_i64(id)).as_jni()],
            )
        }?;
        Ok(())
    }

    pub(crate) fn post_invalidate(
        &self,
        env: &mut Env<'_>,
        view: &JObject<'_>,
    ) -> jni::errors::Result<()> {
        // SAFETY: `view_post_invalidate` has signature `()V`.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_post_invalidate,
                ReturnType::Primitive(Primitive::Void),
                &[],
            )
        }?;
        Ok(())
    }

    pub(crate) fn subscribe_lifecycle(&self, listener: LifecycleListener) -> (u64, bool, bool) {
        let mut hub = self.lifecycle.borrow_mut();
        let token = hub.subscribe(listener);
        (token, hub.started, hub.resumed)
    }

    pub(crate) fn unsubscribe_lifecycle(&self, token: u64) {
        self.lifecycle.borrow_mut().unsubscribe(token);
    }

    pub(crate) fn lifecycle_state(&self) -> (bool, bool) {
        let hub = self.lifecycle.borrow();
        (hub.started, hub.resumed)
    }

    fn set_started(&self, env: &mut Env<'_>, started: bool) {
        let (pause_first, emit_event, listeners) = {
            let mut hub = self.lifecycle.borrow_mut();
            if hub.started == started {
                return;
            }
            let pause_first = !started && hub.resumed;
            if pause_first {
                hub.resumed = false;
            }
            hub.started = started;
            let event = if started {
                crate::android_content::AndroidLifecycle::Start
            } else {
                crate::android_content::AndroidLifecycle::Stop
            };
            (pause_first, event, hub.snapshot())
        };
        if pause_first {
            for listener in &listeners {
                listener(env, crate::android_content::AndroidLifecycle::Pause);
            }
        }
        for listener in &listeners {
            listener(env, emit_event);
        }
    }

    fn set_resumed(&self, env: &mut Env<'_>, resumed: bool) {
        let (start_first, emit_event, listeners) = {
            let mut hub = self.lifecycle.borrow_mut();
            if hub.resumed == resumed {
                return;
            }
            let start_first = resumed && !hub.started;
            if start_first {
                hub.started = true;
            }
            hub.resumed = resumed;
            let event = if resumed {
                crate::android_content::AndroidLifecycle::Resume
            } else {
                crate::android_content::AndroidLifecycle::Pause
            };
            (start_first, event, hub.snapshot())
        };
        if start_first {
            for listener in &listeners {
                listener(env, crate::android_content::AndroidLifecycle::Start);
            }
        }
        for listener in &listeners {
            listener(env, emit_event);
        }
    }
}

/// The Kotlin host's window on the runner's side: host services only. The
/// GPU attachment is a separate object — this type's [`PlatformWindow`] impl
/// never touches it, and [`PaintedWindow::surface`] is the only path that
/// hands it to the painter.
pub struct AndroidHostWindow {
    metrics: MetricsSnapshot,
    events: Vec<InputEvent>,
    /// The host view's attach report, from `nativeSetHostAttached` —
    /// Kotlin's bind/unbind own it; on the GPU path the surface attach is
    /// the draw gate below, here it is part of `presentation_attached`.
    host_attached: bool,
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) surface: AndroidSurface,
    /// The band `View`s' `ImageReader` surfaces, by the placement id of
    /// the embedded platform view each sits above: a band carries the
    /// self-drawn content a frame paints above that view and beneath the
    /// next one (water-rs/waterui#2225).
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) bands: std::collections::BTreeMap<u64, super::gpu::BandAttachment>,
    /// Shared with the installed frame wake, which posts its own
    /// Choreographer request through it, and with the GPU-surface redraw
    /// handle, whose queued post calls it from the executor's drain.
    pub(crate) bridge: Arc<HostBridge>,
    /// The cross-thread redraw handle's transport onto the UI thread: a
    /// post rides the executor's shared `eventfd`, coalescing with its
    /// other wakes.
    ui_poster: UiThreadPoster<EventFdWake>,
    /// A redraw the engine asked for. `on_frame` drains it at the render
    /// boundary — a request raised before the render is demand the render
    /// itself serves — and reads it again at close, where only a request
    /// raised after the render (an animation asking for its next frame)
    /// remains as scheduling demand. Atomic because the GPU-surface redraw
    /// handle latches it from whichever thread its producer runs on.
    redraw_pending: Arc<AtomicBool>,
    /// The Activity is between `onStart` and `onStop` — half of the Android
    /// visibility report; the other half is a live surface below.
    started: bool,
    cursor_style: CursorStyle,
    /// The [`TextInputState::activation`] the soft keyboard was last shown
    /// for; `None` while no field holds focus. The runner syncs text-input
    /// state on every frame, and only a change here reaches the IME.
    soft_input: Option<u64>,
    /// The session-wide "a mounted platform-view child holds UI focus" flag
    /// report UI focus into; while it is held the IME channel belongs to the
    /// child and a stale cleared claim must not hide its keyboard.
    pub(crate) platform_view_focus: crate::platform_view::PlatformViewFocus,
    /// The window's last compositing request: `translucent` composites the
    /// window with what lies behind it, `blur_behind` is the behind-window
    /// material level asking the compositor to blur that content — the pair
    /// the host callback carries. The HWUI painter rejects behind-window
    /// material outright, so the blur half exists only on the GPU build.
    translucent: bool,
    #[cfg(not(hydrolysis_hwui))]
    blur_behind: Option<crate::renderer::material::BehindWindowLevel>,
    frame_transaction: FrameTransaction,
}

impl AndroidHostWindow {
    /// Queues an input event for the next frame transaction's dispatch.
    ///
    /// Host-side `MotionEvent` coordinates arrive in physical pixels while the
    /// retained scene is laid out in logical units — the boundary converts,
    /// exactly as the winit runner's `PhysicalPosition::to_logical` does.
    pub(crate) fn push_event(&mut self, event: InputEvent) {
        self.push_event_with_density(event, self.metrics.density);
    }

    pub(super) fn push_event_with_density(&mut self, mut event: InputEvent, density: f64) {
        let density = crate::num_cast::f64_as_f32(density);
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

    pub fn take_redraw_pending(&self) -> bool {
        self.redraw_pending.swap(false, Ordering::AcqRel)
    }

    /// Re-derives the installed frame wake's gate: closed while `on_frame`
    /// runs (a request raised inside the transaction is counted into
    /// `wants_next_frame` instead of posting) and while the window is
    /// occluded (the request stays armed for the restore frame). Called
    /// wherever either input can move — the transaction's edges and every
    /// occlusion sync — so the wake never carries a second visibility
    /// report of its own.
    pub(crate) fn sync_frame_wake_gate(&self) {
        self.frame_transaction.sync_occlusion(self.is_occluded());
    }

    /// Whether a drawn frame can reach the user: the GPU band needs its
    /// surface attached; the HWUI path draws into the bound view once it
    /// carries real metrics.
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) const fn presentation_attached(&self) -> bool {
        self.surface.is_attached()
    }

    #[cfg(hydrolysis_hwui)]
    pub(crate) const fn presentation_attached(&self) -> bool {
        self.metrics.width_px > 0 && self.metrics.height_px > 0
    }

    /// Pushes the compositing request to the Kotlin host. Both halves ride
    /// one callback: the window's translucency and its blur-behind ask
    /// describe the same resolved background. The blur radius is the level's
    /// measured σ converted to device pixels, then to the radius Android's
    /// `setBackgroundBlurRadius` expects — Skia relates them as
    /// `σ = radius·0.57735 + 0.5`.
    fn sync_translucency(&self) {
        #[cfg(hydrolysis_hwui)]
        let (blur_behind, blur_radius_px) = (false, 0);
        #[cfg(not(hydrolysis_hwui))]
        let (blur_behind, blur_radius_px) = self.blur_behind.map_or((false, 0), |level| {
            let sigma_px = f64::from(level.sigma()) * self.metrics.density;
            (
                true,
                crate::num_cast::f64_as_i32(((sigma_px - 0.5) / 0.57735).round()),
            )
        });
        self.bridge
            .set_translucency(self.translucent, blur_behind, blur_radius_px);
    }
}

impl PlatformWindow for AndroidHostWindow {
    /// The drawable content area in physical pixels, from the metrics
    /// snapshot — valid before any surface exists and while between surface
    /// generations.
    fn content_size(&self) -> (u32, u32) {
        (self.metrics.width_px, self.metrics.height_px)
    }

    fn apply_properties<C, T>(&mut self, window: &waterui::window::Window<C, T>) {
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
    /// Activity's `onStart`/`onStop` state and the host's attach and
    /// presentation readiness. A stopped activity or a detached host means
    /// nothing the renderer could draw reaches the user.
    fn is_occluded(&self) -> bool {
        !self.started || !self.host_attached || !self.presentation_attached()
    }

    /// The translucent-window request, split at the painter boundary: the
    /// GPU attachment's next engine surface is created with the request's
    /// compositing mode, and the Kotlin host switches the Activity window
    /// translucent. An unchanged request repeats nothing — the per-frame
    /// application stays a comparison.
    fn set_transparent(&mut self, transparent: bool) {
        if self.translucent == transparent {
            return;
        }
        self.translucent = transparent;
        #[cfg(not(hydrolysis_hwui))]
        self.surface.set_transparent(transparent);
        self.sync_translucency();
    }

    fn request_redraw(&self) {
        self.redraw_pending.store(true, Ordering::Release);
        // Inside `on_frame`'s transaction the request posts nothing: the
        // frame already running serves a request raised before it renders,
        // and a later one joins the continuation `wants_next_frame` carries.
        // An occluded window's request stays armed for the restore frame.
        if self.frame_transaction.may_post() {
            tracing::debug!(
                target: "waterui::hydrolysis::android",
                "wake posted: redraw requested"
            );
            self.bridge.request_frame();
        }
    }

    /// The wake installed on the window's frame signals: posts the
    /// Choreographer frame a request raised outside a transaction needs —
    /// an executor drain, a platform-view callback, an accessibility
    /// action. The gate suppresses it inside `on_frame` and while
    /// occluded; Kotlin's `posted` flag coalesces repeated posts, so the
    /// wake itself posts unconditionally while the gate is open.
    fn frame_wake(&self) -> Rc<dyn Fn()> {
        let bridge = Arc::clone(&self.bridge);
        self.frame_transaction.frame_wake(Rc::new(move || {
            tracing::debug!(
                target: "waterui::hydrolysis::android",
                "wake posted: frame request pending"
            );
            bridge.request_frame();
        }))
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
        Some(crate::platform::TouchScrollConfig::new(
            crate::num_cast::f64_as_f32(f64::from(metrics.touch_slop_px) / density),
            crate::num_cast::f64_as_f32(f64::from(metrics.min_fling_velocity_px) / density),
            crate::num_cast::f64_as_f32(f64::from(metrics.max_fling_velocity_px) / density),
            crate::platform::ScrollPhysics::android(
                crate::platform::SplineDeceleration::new(
                    physical_coeff_px / density,
                    metrics.scroll_friction,
                ),
                crate::platform::EdgeStretch::android(density),
            ),
        ))
    }

    /// The multi-click run's window is the platform's tap window:
    /// `ViewConfiguration.getDoubleTapTimeout()` and
    /// `getScaledDoubleTapSlop()` from the metrics snapshot, the slop in
    /// logical units. Android's own detector compares the distance as a
    /// radius; the per-axis tolerance here is the square around it. The
    /// snapshot's creation-time zeros read as a run that never continues —
    /// a window still waiting on its first metrics dispatches no press.
    fn input_conventions(&self) -> crate::platform::InputConventions {
        let metrics = &self.metrics;
        let slop = f64::from(metrics.double_tap_slop_px) / metrics.density;
        crate::platform::InputConventions {
            multi_click: crate::platform::MultiClickConfig {
                interval: Duration::from_millis(u64::from(metrics.double_tap_timeout_ms)),
                tolerance: kurbo::Vec2::new(slop, slop),
            },
            long_press_timeout: if metrics.long_press_timeout_ms == 0 {
                DEFAULT_LONG_PRESS_DURATION
            } else {
                Duration::from_millis(u64::from(metrics.long_press_timeout_ms))
            },
            // Android's paste shortcut, on a hardware keyboard, is Control-V:
            // the non-Apple family's.
            ..crate::platform::InputConventions::for_platform(
                crate::platform::ScrollPlatform::Other,
            )
        }
    }

    fn refresh_rate_hz(&self) -> Option<f64> {
        self.metrics.refresh_hz
    }

    fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
        // While a platform-view child holds the IME, a renderer claim that
        // was cleared on the focus-gain edge must not reach the host as a
        // hide request — it would drop the web field's keyboard. A claim set
        // during the hold is the hand-off itself: the Kotlin show path
        // re-requests this view's focus, which ends the hold.
        if self.platform_view_focus.is_holding() && state.is_none() {
            return;
        }
        let soft_input = state.map(|state| state.activation());
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

impl AndroidHostWindow {
    /// The cross-thread `request_redraw` of the HWUI painter's host redraw
    /// handle; see [`redraw_wake`], which both painters' handles share.
    #[cfg(hydrolysis_hwui)]
    pub(crate) fn content_redraw_handle(&self) -> crate::renderer::HostRedraw {
        crate::renderer::HostRedraw::new(self.content_redraw_wake())
    }

    /// The wake behind both painters' `HostRedraw`: on the UI thread it
    /// posts the Choreographer frame through the transaction gate.
    fn content_redraw_wake(&self) -> impl Fn() + Send + Sync + 'static {
        let bridge = Arc::clone(&self.bridge);
        redraw_wake(
            self.ui_poster.clone(),
            Arc::clone(&self.redraw_pending),
            self.frame_transaction.ui_thread_wake(move || {
                tracing::debug!(
                    target: "waterui::hydrolysis::android",
                    "wake posted: cross-thread redraw requested"
                );
                bridge.request_frame();
            }),
        )
    }
}

#[cfg(hydrolysis_gpu_painter)]
impl PaintedWindow for AndroidHostWindow {
    type Presentation = AndroidSurface;
    fn surface(&mut self) -> &mut AndroidSurface {
        &mut self.surface
    }

    fn host_redraw_handle(&self) -> Option<crate::renderer::HostRedraw> {
        Some(crate::renderer::HostRedraw::new(self.content_redraw_wake()))
    }

    /// The compositor blur a behind-window material asks for — the host's
    /// `Window.setBackgroundBlurRadius`, applied while the platform's
    /// cross-window blur stays enabled. An unchanged request repeats
    /// nothing.
    fn set_blur_behind(&mut self, level: Option<crate::renderer::material::BehindWindowLevel>) {
        if self.blur_behind == level {
            return;
        }
        self.blur_behind = level;
        self.sync_translucency();
    }
}

/// Owns the `ALooper` registration that drains an
/// [`AndroidExecutor`]'s wake fd. Dropping unregisters the fd, so
/// the looper can never fire into a dead executor.
struct ExecutorWake {
    looper: ForeignLooper,
    /// A share of the executor's wake fd, keeping it open for the looper for
    /// exactly as long as the registration lives.
    fd: Arc<EventFdWake>,
}

/// The local-reference capacity the wake drain's frame reserves. The
/// `PushLocalFrame` contract is "at least this many", not a bound, so the
/// usual JNI default covers a drain's bridge calls.
const WAKE_DRAIN_LOCALS: usize = 32;

impl ExecutorWake {
    /// Registers the executor's own wake fd's drain callback on this thread's
    /// looper — must be the UI thread (its main looper exists already). The
    /// executor has no other wake fd, so a wake written while the looper
    /// idles is always observed on the registered fd.
    fn register(executor: &AndroidExecutor, vm: &'static JavaVM) -> Result<Self, JniError> {
        let fd = Arc::clone(executor.wake());
        let looper = ThreadLooper::for_thread().ok_or_else(|| {
            JniError("hydrolysis android: no ALooper on the UI thread".to_owned())
        })?;
        let drain = executor.clone();
        looper
            .add_fd_with_callback(fd.fd(), FdEvent::INPUT, move |fd, _events| {
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
                let drained = vm
                    .with_local_frame(WAKE_DRAIN_LOCALS, |_env| Ok::<_, JniError>(drain.drain()))
                    .unwrap_or_else(|error| {
                        // The looper callback runs on the attached UI
                        // thread, where PushLocalFrame fails only when the
                        // VM is out of memory. The queued tasks stay queued
                        // until the next wake.
                        tracing::error!(
                            target: "waterui::hydrolysis::android",
                            ?error,
                            "executor wake: the local-reference frame around the drain could not be pushed"
                        );
                        false
                    });
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
        let _ = self.looper.remove_fd(self.fd.fd());
    }
}

impl FrameHost for AndroidSession {
    type Platform = AndroidHostWindow;
    type Engine = super::AndroidTextEngine;

    fn parts(
        &mut self,
    ) -> (
        &mut RuntimeWindow<AndroidHostWindow, Self::Engine>,
        &Environment,
    ) {
        (&mut self.runtime, &self.env)
    }

    fn frame_transaction(&self) -> &FrameTransaction {
        &self.runtime.platform.frame_transaction
    }

    fn drain_executor(&self) {
        self.executor.drain();
    }

    fn before_input(&mut self) {
        // The frame reads this frame's focus answer: the pull precedes
        // `render_window`'s `sync_text_input_state` hold gate.
        self.platform_view_focus_sync();
    }

    fn input_ready(&self) -> bool {
        // The GPU painter's first frame completes asynchronously; the HWUI
        // painter presents its first frame inside the render.
        #[cfg(hydrolysis_gpu_painter)]
        let ready = self.presented_once.get();
        #[cfg(hydrolysis_hwui)]
        let ready = true;
        ready
    }

    fn poll_submitted_frame(&mut self) {
        #[cfg(hydrolysis_gpu_painter)]
        self.poll_gpu_frame();
    }

    fn frame_in_flight(&self) -> bool {
        #[cfg(hydrolysis_gpu_painter)]
        return self.gpu_frame_pending();
        #[cfg(hydrolysis_hwui)]
        false
    }

    fn rendering_ended(&self) -> bool {
        self.rendering_failed
    }

    fn should_close(&self) -> bool {
        Self::should_close(self)
    }

    fn presentation_attached(&self) -> bool {
        self.runtime.platform.presentation_attached()
    }

    #[cfg(hydrolysis_hwui)]
    fn has_pending_scene_changes(&self) -> bool {
        // Queued scene changes the Kotlin callback's wake posted nothing
        // for; no Java frame post runs inside the open transaction.
        self.hwui.surface.has_pending()
    }

    fn take_redraw_pending(platform: &AndroidHostWindow) -> bool {
        platform.take_redraw_pending()
    }

    fn closing_redraw_pending(&mut self) -> bool {
        #[cfg(hydrolysis_gpu_painter)]
        if self.gpu_frame_pending() {
            return self.runtime.platform.redraw_pending.load(Ordering::Acquire);
        }
        self.runtime.platform.take_redraw_pending()
    }

    fn render(&mut self) -> bool {
        let executor = self.executor.clone();
        #[cfg(hydrolysis_gpu_painter)]
        let presented = {
            let presented =
                match render_window(&mut self.runtime, &self.env, &mut || executor.drain()) {
                    WindowFrame::Failed(error) => {
                        self.end_rendering(&format!("{:#}", waterui_core::Error::from(error)));
                        false
                    }
                    frame => frame.presented(),
                };
            self.runtime.engine_next = crate::renderer::Next::Idle;
            self.poll_gpu_frame();
            presented
        };
        #[cfg(hydrolysis_hwui)]
        let presented = match self
            .hwui
            .render(&mut self.runtime, &self.env, &mut || executor.drain())
        {
            Ok(presented) => presented,
            Err(error) => {
                self.end_rendering(&format!(
                    "hydrolysis HWUI: frame render failed: {}",
                    error.0
                ));
                false
            }
        };
        presented
    }

    fn after_render(&mut self, presented: bool) {
        if presented {
            self.presented_once.set(true);
        }
        #[cfg(feature = "navigation")]
        self.sync_back_navigation_available();
    }

    fn notify_close(&mut self) {
        self.runtime.platform.bridge.close_requested();
    }

    fn publish(&mut self, flushed: bool) {
        // A lost device is recreated inside the frame
        // (`PresentationSurface::ensure_live`, water-rs/waterui#2742); a
        // recreation that exhausts its loss budget, like any other terminal
        // frame failure, ends the session's rendering there: it is logged
        // once and no further frame is drawn or asked for.
        super::accessibility::publish_if_pending(self);
        super::platform_views::publish_if_pending(self, flushed);
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        self.editing_sync();

        // Popup windows mounting mid-frame land on the pending queue: the
        // host has no second band to put one on, so this is the explicit
        // unsupported-component error the repository contract wants.
        let secondary_windows = self.pending_window_queue.borrow().len();
        if secondary_windows != 0 {
            // The windows are dropped unmounted so the violation is not
            // reported again by every later frame.
            self.pending_window_queue.borrow_mut().clear();
            contract_violation!(
                "hydrolysis android: secondary windows are unsupported — the host mounts exactly one window per session; {secondary_windows} popup window(s) dropped"
            );
        }
    }

    fn finished(&mut self, pending: bool) {
        self.report_idle(pending);
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
    pub(super) env: Environment,
    pub(super) runtime: RuntimeWindow<AndroidHostWindow, super::AndroidTextEngine>,
    executor: AndroidExecutor,
    #[cfg(hydrolysis_gpu_painter)]
    completion_waker: std::task::Waker,
    #[cfg(hydrolysis_gpu_painter)]
    host_frame_demand: (bool, bool),
    /// The GPU context outlives the surface and renderer inside `runtime`
    /// (they drop first — field order is teardown order).
    #[cfg(hydrolysis_gpu_painter)]
    #[expect(
        dead_code,
        reason = "held so the wgpu device outlives the runtime's GPU objects"
    )]
    gpu: AndroidGpuContext,
    /// The HWUI painter peer: the surface/mount/JNI client this session
    /// renders through. The GPU path's surface owns this role under the
    /// other cfg — they are build-time alternatives, never switched.
    #[cfg(hydrolysis_hwui)]
    pub(super) hwui: super::hwui::HwuiWindow,
    /// Extra windows the app asks for while mounted: on Android one session
    /// mounts exactly one host view, so a second window is an explicit
    /// unsupported-feature error, not a silently dropped request.
    pending_window_queue:
        Rc<RefCell<Vec<super::super::window::RootWindow<super::AndroidTextEngine>>>>,
    /// Keeps state watchers on dormant `Closed` secondary windows alive so
    /// opening one at run time reports the unsupported secondary-window error.
    _dormant_window_watches: Vec<Retain>,
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
    android_bridge: AndroidHostBridge,
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    pub(crate) ime: ImeBridge,
    /// The live surface generation, as last reported by the host.
    #[cfg(hydrolysis_gpu_painter)]
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
    /// The direction the insets' leading and trailing were last resolved
    /// under — the publish direction `set_metrics` compares so a
    /// direction-only change re-derives both regions
    /// (water-rs/waterui#1827).
    insets_direction: LayoutDirection,
    /// The live colour-scheme signal the session's theme tokens resolve
    /// against — `set_night_mode` writes it on the host's pushes.
    color_scheme: nami::Binding<ColorScheme>,
    /// The live Reduce Motion signal driven by Android's
    /// `Settings.Global.ANIMATOR_DURATION_SCALE` preference.
    reduce_motion: Binding<bool>,
    /// A frame reached the surface at least once; together with an idle
    /// `wants_next_frame` it arms the one-shot readiness log the device's
    /// idle-CPU sampling waits on.
    presented_once: Cell<bool>,
    /// A frame failed for good: the surface cannot render again, so the
    /// session draws and asks for no further frame.
    rendering_failed: bool,
    /// The readiness line was already logged — it fires exactly once per
    /// session so a later busy window cannot re-arm it.
    ready_logged: Cell<bool>,
    /// The last value reported to the host's back callback. Starts disabled,
    /// matching the callback the activity registers.
    #[cfg(feature = "navigation")]
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
    pub(super) executor: AndroidExecutor,
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
    pub(crate) fn init(vm: &'static JavaVM) -> Result<Self, JniError> {
        #[cfg(hydrolysis_inspector)]
        let inspector = init_main_thread_executors();
        #[cfg(hydrolysis_inspector)]
        let inspector_probe = inspector
            .as_ref()
            .map(|inspector| inspector.task_probe() as Arc<dyn waterui::task::RuntimeProbe>);
        #[cfg(not(hydrolysis_inspector))]
        let inspector_probe: Option<Arc<dyn waterui::task::RuntimeProbe>> = {
            crate::runner::init_global_executor();
            None
        };
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
        let executor = AndroidExecutor::new(EventFdWake::new(unsafe { OwnedFd::from_raw_fd(fd) }));
        let wake = ExecutorWake::register(&executor, vm)?;
        executor_core::try_init_local_executor(
            waterui::task::monitored_local_executor_with_probes(
                executor.clone(),
                waterui::task::RefreshRate::HEADLESS,
                inspector_probe,
            ),
        )
        .map_err(|_| {
            JniError("hydrolysis android: the UI thread's executor is already attached".to_owned())
        })?;
        #[cfg(hydrolysis_inspector)]
        let mut env = Environment::new();
        #[cfg(not(hydrolysis_inspector))]
        let env = Environment::new();
        #[cfg(hydrolysis_inspector)]
        waterui::inspector::install(&mut env, inspector);
        Ok(Self {
            executor,
            wake,
            env,
        })
    }
}

/// Installs the platform-view sink and the surface producers that publish into it.
fn install_platform_view_surfaces(env: &mut Environment) -> crate::platform_view::PlatformViewSink {
    // The platform-view sink `PlatformView` leaves record their frames
    // into; the published table is what `nativePlatformViewFrames` serves.
    let platform_views = crate::platform_view::PlatformViewSink::new();
    env.insert(platform_views.clone());
    // GPU content under the HWUI painter: each view's own platform surface,
    // placed through the same sink (water-rs/waterui#1812).
    #[cfg(hydrolysis_hwui_gpu)]
    env.insert(Rc::new(super::hosted_gpu::HostedGpu::new(
        platform_views.clone(),
    )));
    // The GPU video player's decoder planes, placed through the same sink.
    #[cfg(hydrolysis_video_plane)]
    env.insert(Rc::new(super::video_plane::VideoPlanes::new(
        platform_views.clone(),
    )));
    platform_views
}

/// What `create` wires into the platform host and the session: the one
/// window the app registered, its environment, theme and fonts, the
/// window's registry id, and the bindings and sinks the host later drives.
struct SessionMounts {
    env: Environment,
    window: super::super::window::RootWindow<super::AndroidTextEngine>,
    theme: Rc<dyn WidgetTheme>,
    /// The collection the env installs; only the GPU runtime's
    /// `ParleyEngine` reads it back — the HWUI engine draws text through
    /// the platform's own `StaticLayout`.
    #[cfg(hydrolysis_gpu_painter)]
    fonts: waterui_text::FontCollection,
    window_id: crate::renderer::WindowId,
    pending_window_queue:
        Rc<RefCell<Vec<super::super::window::RootWindow<super::AndroidTextEngine>>>>,
    dormant_window_watches: Vec<Retain>,
    platform_views: crate::platform_view::PlatformViewSink,
    platform_view_focus: crate::platform_view::PlatformViewFocus,
    android_bridge: AndroidHostBridge,
    safe_area: nami::Binding<waterui_layout::padding::EdgeInsets>,
    keyboard_area: nami::Binding<waterui_layout::padding::EdgeInsets>,
    /// The session's colour-scheme signal — the binding `install_theme_tokens`
    /// resolved the default tokens over; `set_night_mode` writes it.
    color_scheme: nami::Binding<ColorScheme>,
    /// The session's Reduce Motion signal — `set_reduce_motion` writes it.
    reduce_motion: Binding<bool>,
    #[cfg(hydrolysis_android_system_webview)]
    android_webview: crate::widgets::platform::webview::AndroidSystemWebViewController,
}

/// Builds the session's mount inputs: instantiates the app, layers the
/// window's environment over the process's, installs the component hooks,
/// theme, fonts and platform-view state, and mints the window's registry
/// id — everything `create` then wires into the platform host and the
/// runtime window.
///
/// `None` after a reported contract violation (no registered app, no
/// seeded shortcut registry, no or several windows).
fn session_mounts(
    services: &'static UiThreadServices,
    night: bool,
    reduce_motion: bool,
    jni_env: &mut Env,
    bridge: &HostBridge,
) -> Result<Option<SessionMounts>, JniError> {
    let Some((parts, style)) = super::instantiate_app() else {
        return Ok(None);
    };
    let waterui::app::AppParts {
        windows,
        #[cfg(feature = "menu")]
        menu_bar,
        #[cfg(not(feature = "menu"))]
            menu_bar: _,
        env,
        // A phone activity already embodies both policies: when the last
        // window asks to close the host finishes the activity, and the OS
        // itself decides whether the process stays resident.
        last_window: _,
        // Android kills the process without notice, so the termination
        // hooks are never called and the machine is never started.
        termination: _,
    } = parts;
    let mut env = env.layered_on(&services.env);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    #[cfg(any(test, hydrolysis_platform_services))]
    install_platform_services(&mut env);
    install_window_registries::<super::AndroidTextEngine>(&mut env);
    #[cfg(feature = "menu")]
    if let Some(menu_bar) = AppMenuBar::declared(&menu_bar) {
        menu_bar::register_menu_bar::<super::AndroidTextEngine>(menu_bar, &env);
    }
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    // The configuration's night mode is the colour-scheme signal: created
    // with the real `uiMode` the create call carried and kept current by
    // `nativeSetColorScheme` pushes, so the default tokens re-resolve on a
    // theme flip the way macOS's appearance binding does (#1972).
    let color_scheme = nami::binding(if night {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    });
    let reduce_motion = Binding::bool(reduce_motion);
    env.insert(ReduceMotion::new(reduce_motion.clone()));
    crate::theme::install_theme_tokens(&mut env, Some(&*style), color_scheme.clone());
    let theme: Rc<dyn WidgetTheme> = style;
    #[cfg(hydrolysis_gpu_painter)]
    let fonts = crate::text::fonts::native_collection(&env);
    #[cfg(hydrolysis_gpu_painter)]
    fonts.clone().install(&mut env);
    let Some(identities) = env.get::<crate::renderer::WindowIdentities>().cloned() else {
        contract_violation!(
            "hydrolysis android: install_window_registries did not seed \
             WindowIdentities; no session is created"
        );
        return Ok(None);
    };
    let safe_area = nami::binding(waterui_layout::padding::EdgeInsets::default());
    env.insert(crate::platform::WindowSafeArea(safe_area.clone()));
    let keyboard_area = nami::binding(waterui_layout::padding::EdgeInsets::default());
    env.insert(crate::platform::WindowKeyboardArea(keyboard_area.clone()));
    let platform_views = install_platform_view_surfaces(&mut env);
    let android_bridge = AndroidHostBridge::resolve(jni_env, bridge)?;
    env.insert(android_bridge.clone());
    // The session-wide "focus sits inside a platform-view container"
    // state the Kotlin registry reports through
    // `onNativePlatformViewFocus`: while a mounted child holds the IME,
    // the renderer must not keep a `WaterUI` text-input claim, and the
    // window reads the same state to gate its soft-input pushes.
    let platform_view_focus = crate::platform_view::PlatformViewFocus::default();

    // The system-WebView controller attaches directly to the renderer's
    // `SemanticCore` so `NativeBackend::web_view_controller` supplies it
    // without going through `Environment`. Resolving the wrapper class must
    // happen here, on the JNI thread — a thread that did not enter from
    // Java cannot resolve app classes later.
    #[cfg(hydrolysis_android_system_webview)]
    let android_webview = crate::widgets::platform::webview::AndroidSystemWebViewController::new(
        jni_env,
        Arc::clone(&bridge.host_view),
    )?;

    let super::super::window::RootWindows {
        initial: windows,
        channels,
    } = windows;
    let opener = queue_window_opener(&pending_window_queue);
    super::super::window::RootWindows::connect_channels(channels, &opener);
    let (open_windows, dormant_window_watches) = split_open_and_watch_dormant(windows, || {
        contract_violation!(
            "hydrolysis android: secondary windows are unsupported — the host mounts exactly one window per session; the window is not shown"
        );
    });
    let mut windows = VecDeque::from(open_windows);
    let Some(window) = windows.pop_front() else {
        contract_violation!(
            "hydrolysis android: the app registered no window to mount; no session is created"
        );
        return Ok(None);
    };
    if !windows.is_empty() {
        contract_violation!(
            "hydrolysis android: multiple windows are unsupported — the host mounts exactly one window per session; no session is created"
        );
        return Ok(None);
    }

    Ok(Some(SessionMounts {
        env,
        window,
        theme,
        #[cfg(hydrolysis_gpu_painter)]
        fonts,
        window_id: identities.mint_window_id(),
        pending_window_queue,
        dormant_window_watches,
        platform_views,
        platform_view_focus,
        android_bridge,
        safe_area,
        keyboard_area,
        color_scheme,
        #[cfg(hydrolysis_android_system_webview)]
        android_webview,
    }))
}

/// The GPU build's runtime window: the Parley-engine renderer under
/// diagnostics. A painter reaching `nativeCreateSession` on this build is
/// a feature mismatch, not a surface to adopt.
#[cfg(hydrolysis_gpu_painter)]
fn gpu_runtime_window(
    window: super::super::window::RootWindow<super::AndroidTextEngine>,
    platform: AndroidHostWindow,
    theme: Rc<dyn WidgetTheme>,
    fonts: &waterui_text::FontCollection,
    window_id: crate::renderer::WindowId,
    painter: &JObject<'_>,
    env: &Environment,
) -> Result<RuntimeWindow<AndroidHostWindow, super::AndroidTextEngine>, JniError> {
    if !painter.is_null() {
        return Err(JniError(
            "hydrolysis GPU: nativeCreateSession got a painter — this build is the GPU backend; rebuild with the HWUI feature".to_owned(),
        ));
    }
    let mut renderer = HydrolysisRenderer::with_engine(
        theme,
        ParleyEngine::from_collection(fonts, FontFamilyResolution::Lenient),
    );
    renderer.set_window_id(window_id);
    renderer.set_window_closable(window.closable);
    Ok(RuntimeWindow::new_painted(
        window,
        platform,
        renderer,
        RenderDiagnosticsConfig::from_env(),
        env,
    ))
}

/// An HWUI window: the painter peer and the runtime window rendering
/// through it.
#[cfg(hydrolysis_hwui)]
type HwuiRuntimeWindow = (
    super::hwui::HwuiWindow,
    RuntimeWindow<AndroidHostWindow, super::AndroidTextEngine>,
);

/// The HWUI build's runtime window and the painter peer it renders
/// through: the renderer is wired to the surface's `SurfaceId`, the
/// registry's window id and the host's redraw handle.
#[cfg(hydrolysis_hwui)]
fn hwui_runtime_window(
    jni_env: &mut Env,
    painter: &JObject<'_>,
    window: super::super::window::RootWindow<super::AndroidTextEngine>,
    platform: AndroidHostWindow,
    theme: Rc<dyn WidgetTheme>,
    window_id: crate::renderer::WindowId,
    env: &Environment,
) -> Result<Option<HwuiRuntimeWindow>, JniError> {
    let ordinal = match window_id {
        crate::renderer::WindowId::Runner(id) => id,
        crate::renderer::WindowId::Orphan => {
            contract_violation!(
                "hydrolysis android: the Android window registry minted an orphan window id; \
                 no session is created"
            );
            return Ok(None);
        }
    };
    let redraw = platform.content_redraw_handle();
    let effect_wake = std::sync::Arc::new(move || redraw.request_redraw());
    let (hwui, mut renderer) = super::hwui::HwuiWindow::new(
        jni_env,
        painter,
        SurfaceId::new(ordinal),
        theme,
        platform.frame_wake(),
        effect_wake,
        #[cfg(hydrolysis_hwui_gpu)]
        env.get::<Rc<super::hosted_gpu::HostedGpu>>()
            .expect("session_mounts installs the hosted-GPU owner under HWUI with GPU content")
            .clone(),
    )?;
    renderer.set_window_id(window_id);
    renderer.set_window_closable(window.closable);
    renderer.set_host_redraw_handle(platform.content_redraw_handle());
    Ok(Some((
        hwui,
        RuntimeWindow::new(
            window,
            platform,
            renderer,
            RenderDiagnosticsConfig::from_env(),
            env,
        ),
    )))
}

fn android_host_window(
    metrics: MetricsSnapshot,
    #[cfg(hydrolysis_gpu_painter)] gpu: &AndroidGpuContext,
    bridge: Arc<HostBridge>,
    executor: &AndroidExecutor,
    platform_view_focus: crate::platform_view::PlatformViewFocus,
) -> AndroidHostWindow {
    AndroidHostWindow {
        metrics,
        events: Vec::new(),
        host_attached: false,
        #[cfg(hydrolysis_gpu_painter)]
        surface: AndroidSurface::new(gpu.clone(), executor.clone()),
        #[cfg(hydrolysis_gpu_painter)]
        bands: std::collections::BTreeMap::new(),
        bridge,
        ui_poster: executor.poster(),
        redraw_pending: Arc::new(AtomicBool::new(false)),
        started: false,
        cursor_style: CursorStyle::default(),
        soft_input: None,
        platform_view_focus,
        translucent: false,
        #[cfg(not(hydrolysis_hwui))]
        blur_behind: None,
        // The session mounts parked — hidden until `onStart` and the
        // surface attach report it visible — so the wake's gate opens
        // on the first occlusion sync, not at construction.
        frame_transaction: FrameTransaction::default(),
    }
}

impl AndroidSession {
    /// Mounts the registered app on the host view: builds the environment,
    /// GPU context and the runtime window. `services` is the UI-thread
    /// bundle the load hook created — the session clones its executor and
    /// layers its environment over the process's.
    ///
    /// `metrics` is the host's snapshot at creation time — content size and
    /// scale exist from the start, before the surface band attaches. `night`
    /// is the configuration's real `uiMode` at the same instant, so the
    /// colour-scheme signal never starts on a Light default.
    ///
    /// `None` after a reported contract violation (no registered app, no or
    /// several windows, an orphan window id): no session exists then.
    pub(crate) fn create(
        jni_env: &mut Env,
        bridge: Arc<HostBridge>,
        #[cfg(hydrolysis_gpu_painter)] gpu: AndroidGpuContext,
        painter: &JObject<'_>,
        metrics: MetricsSnapshot,
        night: bool,
        reduce_motion: bool,
        services: &'static UiThreadServices,
    ) -> Result<Option<Box<Self>>, JniError> {
        let executor = services.executor.clone();

        waterui_locale::start_system_locale_listener();

        let Some(SessionMounts {
            env,
            window,
            theme,
            #[cfg(hydrolysis_gpu_painter)]
            fonts,
            window_id,
            pending_window_queue,
            dormant_window_watches,
            platform_views,
            platform_view_focus,
            android_bridge,
            safe_area,
            keyboard_area,
            color_scheme,
            reduce_motion,
            #[cfg(hydrolysis_android_system_webview)]
            android_webview,
        }) = session_mounts(services, night, reduce_motion, jni_env, &bridge)?
        else {
            return Ok(None);
        };

        let insets_direction = waterui_core::layout::layout_direction(&env).snapshot();
        safe_area.set(metrics.container_metrics(insets_direction).logical());
        keyboard_area.set(metrics.keyboard_metrics(insets_direction).logical());
        let mut platform = android_host_window(
            metrics,
            #[cfg(hydrolysis_gpu_painter)]
            &gpu,
            bridge,
            &executor,
            platform_view_focus,
        );
        platform.apply_properties(&window);
        #[cfg(hydrolysis_gpu_painter)]
        let mut runtime =
            gpu_runtime_window(window, platform, theme, &fonts, window_id, painter, &env)?;
        #[cfg(hydrolysis_hwui)]
        let Some((hwui, mut runtime)) =
            hwui_runtime_window(jni_env, painter, window, platform, theme, window_id, &env)?
        else {
            return Ok(None);
        };
        install_core_window_openers(
            &mut runtime.renderer,
            queue_window_opener(&pending_window_queue),
        );
        #[cfg(hydrolysis_android_system_webview)]
        runtime.renderer.set_android_webview(android_webview);
        // The session mounts parked: the activity's `onStart` and the
        // surface band's attach are the reports that unpark it.
        runtime.set_hidden(runtime.platform.is_occluded());
        runtime.platform.sync_frame_wake_gate();

        #[cfg(hydrolysis_gpu_painter)]
        let completion_waker = std::task::Waker::from(Arc::new(FrameCompletionWake {
            poster: executor.poster(),
            bridge: Arc::clone(&runtime.platform.bridge),
        }));

        Ok(Some(Box::new(Self {
            env,
            runtime,
            executor,
            #[cfg(hydrolysis_gpu_painter)]
            completion_waker,
            #[cfg(hydrolysis_gpu_painter)]
            host_frame_demand: (false, false),
            #[cfg(hydrolysis_gpu_painter)]
            gpu,
            #[cfg(hydrolysis_hwui)]
            hwui,
            pending_window_queue,
            _dormant_window_watches: dormant_window_watches,
            a11y: AccessibilitySnapshot::default(),
            platform_views,
            android_bridge,
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
            ime: ImeBridge::default(),
            #[cfg(hydrolysis_gpu_painter)]
            surface_generation: 0,
            frame_deadline_in_nanos: None,
            safe_area,
            keyboard_area,
            insets_direction,
            color_scheme,
            reduce_motion,
            presented_once: Cell::new(false),
            rendering_failed: false,
            ready_logged: Cell::new(false),
            #[cfg(feature = "navigation")]
            back_navigation_available: false,
            _not_send: std::marker::PhantomData,
        })))
    }

    /// Pushes a metrics snapshot from the host: size/density/refresh/insets
    /// arrive as one coherent unit. A size change becomes a `Resize` event so
    /// the window's frame binding tracks the host.
    pub(crate) fn set_metrics(&mut self, metrics: MetricsSnapshot) {
        let container_insets_px = metrics.container_insets_px;
        let keyboard_insets_px = metrics.keyboard_insets_px;
        // The physical push edges land on different logical edges under
        // right-to-left, so the direction is a third input the compared
        // value carries — a direction-only move re-publishes both
        // regions (water-rs/waterui#1827).
        let direction = waterui_core::layout::layout_direction(&self.env).snapshot();
        let container = metrics.container_metrics(direction);
        let keyboard = metrics.keyboard_metrics(direction);
        let (size_changed, container_changed, keyboard_changed) = {
            let platform = &mut self.runtime.platform;
            let size_changed = platform.metrics.width_px != metrics.width_px
                || platform.metrics.height_px != metrics.height_px
                || platform.metrics.density.to_bits() != metrics.density.to_bits();
            // A region's logical insets derive from the region's pixel
            // edges, the density AND the direction, so the compared value
            // is that whole triple — a density-only or direction-only
            // change still re-publishes both regions.
            let container_changed =
                platform.metrics.container_metrics(self.insets_direction) != container;
            let keyboard_changed =
                platform.metrics.keyboard_metrics(self.insets_direction) != keyboard;
            platform.metrics = metrics;
            self.insets_direction = direction;
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
            if container_changed {
                self.safe_area.set(container.logical());
            }
            if keyboard_changed {
                self.keyboard_area.set(keyboard.logical());
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
                "metrics changed"
            );
        }
        // The insets' binding writes above mark their subscribers' nodes —
        // a `FrameSignals` request whose host wake posts the frame itself
        // (issue #2286) — so an insets change needs no explicit frame
        // request, and one nothing subscribes correctly schedules nothing.
        // A zero→real extent unparks the HWUI surface: its draw gate is the
        // metrics, not a surface attach.
        #[cfg(hydrolysis_hwui)]
        {
            self.runtime.platform.sync_frame_wake_gate();
            self.hwui
                .surface
                .set_hidden(self.runtime.platform.is_occluded());
            self.runtime.sync_occlusion_and_post_restore();
        }
    }

    /// The host's configuration reported a night-mode value — at bind and
    /// on every `onConfigurationChanged` dispatch. A moved value writes the
    /// scheme binding; the token signals' subscribers re-resolve and their
    /// frame signals schedule the repaint, so no explicit frame request is
    /// needed, and an unchanged push sets nothing.
    pub(crate) fn set_night_mode(&self, night: bool) {
        let scheme = if night {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        if self.color_scheme.snapshot() != scheme {
            self.color_scheme.set(scheme);
        }
    }

    /// Updates the session's Reduce Motion signal when Android's
    /// `Settings.Global.ANIMATOR_DURATION_SCALE` changes.
    pub(crate) fn set_reduce_motion(&self, reduced: bool) {
        if self.reduce_motion.snapshot() != reduced {
            self.reduce_motion.set(reduced);
        }
    }

    /// The hosting Activity crossed `onStart`/`onStop` — Android's
    /// visibility signal. Stopping parks the pump even when the surface
    /// survives; starting posts the single restore frame when the band
    /// can present again.
    pub(crate) fn set_visible(&mut self, jni_env: &mut Env, started: bool) {
        self.runtime.platform.started = started;
        #[cfg(hydrolysis_gpu_painter)]
        if !started {
            self.runtime.platform.surface.cancel_frame();
            self.runtime.pending_render_diagnostic = None;
            self.runtime
                .platform
                .redraw_pending
                .store(true, Ordering::Release);
        }
        self.runtime.platform.sync_frame_wake_gate();
        #[cfg(hydrolysis_hwui)]
        self.hwui
            .surface
            .set_hidden(self.runtime.platform.is_occluded());
        self.android_bridge.set_started(jni_env, started);
        // Android has no about-to-wait pass to notice an armed mode — the
        // restore frame needs the explicit Choreographer post.
        self.runtime.sync_occlusion_and_post_restore();
    }

    /// The hosting Activity crossed `onResume`/`onPause` — forwarded to
    /// mounted `AndroidContent` views.
    pub(crate) fn set_resumed(&self, jni_env: &mut Env, resumed: bool) {
        self.android_bridge.set_resumed(jni_env, resumed);
    }

    /// The Kotlin host bound or unbound the view — the HWUI draw gate.
    /// Kotlin's `bind`/`unbind` call this; the GPU band keeps owning its
    /// own `SurfaceHolder` callbacks, so this updates the attach half of
    /// `is_occluded` on both paths.
    pub(crate) fn set_host_attached(&mut self, attached: bool) {
        self.runtime.platform.host_attached = attached;
        #[cfg(hydrolysis_gpu_painter)]
        if !attached {
            self.runtime.platform.surface.cancel_frame();
            self.runtime.pending_render_diagnostic = None;
        }
        self.runtime.platform.sync_frame_wake_gate();
        #[cfg(hydrolysis_hwui)]
        self.hwui
            .surface
            .set_hidden(self.runtime.platform.is_occluded());
        self.runtime.sync_occlusion_and_post_restore();
    }

    /// Tells the host when the rendered frame's back-target answer changes.
    ///
    /// The callback starts disabled. A disabled callback leaves back to the
    /// system, which finishes the activity with its own predictive animation.
    #[cfg(feature = "navigation")]
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
    /// Choreographer callback lands here. The sequence is the shared
    /// [`run_frame_transaction`]; this host supplies the steps around it.
    ///
    /// `presentation` is when `Choreographer` expects this frame to reach
    /// the display; the engine renders against it. A frame woken by a
    /// deadline timer has none and renders at the instant it starts.
    pub(crate) fn on_frame(
        &mut self,
        presentation: Option<Instant>,
        vsync_id: Option<i64>,
    ) -> FrameOutcome {
        tracing::debug!(target: "waterui::hydrolysis::android", "frame wake");
        #[cfg(not(hydrolysis_hwui))]
        self.runtime.renderer.set_frame_timeline(vsync_id);
        #[cfg(hydrolysis_hwui)]
        let _ = vsync_id;
        let outcome = run_frame_transaction(self, presentation);
        #[cfg(not(hydrolysis_hwui))]
        self.runtime.renderer.set_frame_timeline(None);
        let deadline_in_nanos = outcome.deadline.map(|at| {
            i64::try_from(
                at.checked_duration_since(Instant::now())
                    .unwrap_or_default()
                    .as_nanos(),
            )
            .unwrap_or_default()
        });
        let frame_rate_vote = outcome.frame_rate_vote;
        #[cfg(hydrolysis_gpu_painter)]
        {
            let (continuous, touching) = self.host_frame_demand;
            if let Err(error) = self.set_frame_demand(continuous, touching) {
                self.end_rendering(&error);
            }
        }
        self.frame_deadline_in_nanos = deadline_in_nanos;
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            wants_next_frame = outcome.wants_next_frame,
            deadline_in_nanos,
            should_close = outcome.should_close,
            ?frame_rate_vote,
            "frame outcome"
        );
        FrameOutcome {
            wants_next_frame: outcome.wants_next_frame,
            deadline_in_nanos,
            should_close: outcome.should_close,
        }
    }

    /// Whether a submitted GPU frame's completion is still outstanding.
    #[cfg(hydrolysis_gpu_painter)]
    const fn gpu_frame_pending(&self) -> bool {
        self.runtime.platform.surface.frame_pending()
    }

    fn report_idle(&self, pending: bool) {
        if reports_ui_idle(
            self.presented_once.get(),
            pending || matches!(self.runtime.engine_next, crate::renderer::Next::At { .. }),
            self.runtime.is_hidden(),
        ) && !self.ready_logged.get()
        {
            tracing::info!(target: "waterui::hydrolysis::android", "hydrolysis android: first frame presented; ui idle");
            self.ready_logged.set(true);
        }
    }

    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn complete_gpu_frame(&mut self) {
        self.poll_gpu_frame();
        let (continuous, touching) = self.host_frame_demand;
        if let Err(error) = self.set_frame_demand(continuous, touching) {
            self.end_rendering(&error);
        }
        self.report_idle(
            self.runtime.platform.surface.frame_pending()
                || self.runtime.mode.is_pending()
                || self.runtime.platform.redraw_pending.load(Ordering::Acquire)
                || self.runtime.renderer.has_pending_frame_request(),
        );
        if !self.rendering_failed
            && !self.runtime.is_hidden()
            && (self.runtime.mode.is_pending()
                || self.runtime.platform.redraw_pending.load(Ordering::Acquire)
                || self.runtime.renderer.has_pending_frame_request()
                || matches!(self.runtime.engine_next, crate::renderer::Next::At { .. }))
        {
            self.runtime.platform.bridge.request_frame();
        }
    }

    #[cfg(hydrolysis_gpu_painter)]
    fn poll_gpu_frame(&mut self) {
        let preparing = self.runtime.platform.surface.window_pending();
        let mut cx = std::task::Context::from_waker(&self.completion_waker);
        match self
            .runtime
            .platform
            .surface
            .poll_frame(&mut self.runtime.renderer, &mut cx)
        {
            Ok(true) => {
                if let Some((mut sample, submitted)) = self.runtime.pending_render_diagnostic.take()
                {
                    let elapsed = submitted.elapsed();
                    sample.render += elapsed;
                    sample.total += elapsed;
                    let title = self.runtime.window.title.snapshot();
                    self.runtime
                        .render_diagnostics
                        .record_frame(title.as_str(), sample);
                }
                self.runtime.presented_frames += 1;
                tracing::debug!(frames = self.runtime.presented_frames, "frame presented");
                self.presented_once.set(true);
                if !self.runtime.platform.events.is_empty() {
                    self.runtime.platform.request_redraw();
                }
                if let Some(next) = self.runtime.renderer.take_engine_next() {
                    self.runtime.engine_next = next;
                }
                self.commit_bands();
            }
            Ok(false) => {
                if preparing && !self.runtime.platform.surface.window_pending() {
                    self.runtime.request_refresh();
                }
            }
            Err(crate::runner::window::FrameFailure::Skipped(_)) => {
                self.runtime.request_refresh();
                self.runtime.platform.request_redraw();
            }
            Err(crate::runner::window::FrameFailure::Failed(error)) => {
                self.end_rendering(&format!("{:#}", waterui_core::Error::from(error)));
            }
        }
    }

    /// Ends the session's rendering after a frame failed for good: the
    /// surface cannot render again, so `message` is logged once and no
    /// further frame is drawn or asked for.
    fn end_rendering(&mut self, message: &str) {
        tracing::error!(
            target: "waterui::hydrolysis::android",
            "{message}; the window cannot render and draws no further frame"
        );
        self.rendering_failed = true;
    }

    /// The Kotlin GPU band created or replaced its surface; `generation`
    /// comes from the host's own counter, so a stale `surfaceDestroyed` can
    /// never tear down a newer attachment.
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn surface_attached_with_generation(
        &mut self,
        native_window: ndk::native_window::NativeWindow,
        refresh: super::gpu::DisplayRefresh,
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
        let attached =
            self.runtime
                .platform
                .surface
                .attach(native_window, refresh, width, height, generation);
        self.runtime.platform.sync_frame_wake_gate();
        attached.map_err(|error| error.to_string())?;
        // A new surface never inherits the old one's presented frame — the
        // next transaction must re-encode and present. Attaching while
        // `started` can un-hide a session parked on a missing surface; a
        // stale `surfaceCreated` delivered after `onStop` attaches nothing
        // the user sees, and `started` stays the lifecycle's report.
        self.runtime.request_refresh();
        self.runtime.sync_occlusion_and_post_restore();
        Ok(())
    }

    /// The band's size changed on the live surface generation.
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn surface_resized(&mut self, width: u32, height: u32, generation: u64) {
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            generation,
            width,
            height,
            "wake posted: surface resized"
        );
        if !self
            .runtime
            .platform
            .surface
            .resize_for(width, height, generation)
        {
            return;
        }
        self.runtime.platform.sync_frame_wake_gate();
        self.runtime.sync_occlusion();
        // Resizing cancels an in-flight submission or surface preparation.
        // Its completion can no longer drive the pump, so the new size owns
        // a refresh and wake even when the window was already visible.
        self.runtime.request_refresh();
        self.runtime.request_redraw();
    }

    /// The band's surface is going away; `generation` names which one, so a
    /// stale destroy does not tear down a newer attachment.
    #[cfg(hydrolysis_gpu_painter)]
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

    /// A band `View` created its `ImageReader` — on attach and on every
    /// resize, each with a new `generation` from the band's own counter.
    /// The band above the platform view whose placement id is `view`
    /// takes over its region from the next frame: the engine presents that
    /// region's content into the reader's surface (water-rs/waterui#2225).
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn band_attached(
        &mut self,
        view: u64,
        native_window: ndk::native_window::NativeWindow,
        width: u32,
        height: u32,
        generation: u64,
    ) {
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            view,
            generation,
            width,
            height,
            "wake posted: band attached"
        );
        let band = super::gpu::BandAttachment::new(native_window, width, height, generation);
        let platform = &mut self.runtime.platform;
        platform.surface.set_band(view, Some(&band));
        platform.bands.insert(view, band);
        self.runtime.request_refresh();
    }

    /// A band `View`'s reader is about to close; `generation` names which
    /// one, so a stale detach never clears a newer reader. Until a band
    /// attaches again its region is not shown — never drawn beneath the
    /// platform view the paint order puts it above.
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn band_detached(&mut self, view: u64, generation: u64) {
        tracing::debug!(
            target: "waterui::hydrolysis::android",
            view,
            generation,
            "wake posted: band detached"
        );
        let platform = &mut self.runtime.platform;
        if platform
            .bands
            .get(&view)
            .is_some_and(|band| band.is(generation))
        {
            platform.surface.set_band(view, None);
            platform.bands.remove(&view);
        }
    }

    /// Hands the bands their part of the frames the engine finished: a
    /// rebuilt engine window first receives every band, then each band
    /// commit's images are latched by their `View`s and the commit is
    /// applied in this same UI-thread turn — ahead of the next traversal,
    /// so the band `View`s draw their new images in the view frame that
    /// accompanies the base surface's transaction (water-rs/waterui#2225).
    #[cfg(hydrolysis_gpu_painter)]
    fn commit_bands(&mut self) {
        let platform = &mut self.runtime.platform;
        let delivered = platform.surface.deliver_bands(&platform.bands);
        let commits: Vec<_> = platform.surface.take_band_commits().collect();
        for commit in commits {
            for update in commit.updates() {
                // A band that detached since the frame has no `View` left
                // to latch into; its region is not shown.
                if !platform.bands.contains_key(&update.window) {
                    continue;
                }
                match update.image.and_then(color_space_name) {
                    Some(name) => {
                        platform
                            .bridge
                            .band_show(update.window, name, &update.painted_rects);
                    }
                    None => platform.bridge.band_clear(update.window),
                }
            }
            commit.apply();
        }
        if delivered {
            self.runtime.request_refresh();
        }
    }

    /// Combines host work with the window's effective refresh-rate vote.
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn set_frame_demand(
        &mut self,
        continuous: bool,
        touching: bool,
    ) -> Result<(), String> {
        self.host_frame_demand = (continuous, touching);
        let visible = !self.runtime.is_hidden();
        let touching_vote = (visible && touching).then_some(FrameRate::FAST);
        let mut rate = combine(self.runtime.effective_frame_rate_vote(), touching_vote);
        if rate.is_none()
            && visible
            && (continuous || self.runtime.platform.surface.frame_pending())
        {
            rate = Some(FrameRate::FAST);
        }
        self.runtime
            .platform
            .surface
            .set_refresh_demand(RefreshDemand { rate })
            .map_err(|error| error.to_string())
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
