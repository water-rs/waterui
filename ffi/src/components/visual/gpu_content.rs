//! FFI bindings for [`GpuContentView`]: user GPU work a native host presents.
//!
//! Cherenkov owns content rendering and composition. The native host presents
//! the engine texture on the environment's shared [`GpuRuntime`]:
//!
//! 1. `waterui_gpu_content_create` consumes the view descriptor and returns
//!    the state the host owns for the semantic view's lifetime.
//! 2. Somewhere to draw, which differs by platform:
//!    - Android attaches a `SurfaceView`'s `ANativeWindow` with
//!      `waterui_gpu_content_attach`, replaces it as its lifecycle demands, and
//!      renders into the swapchain with `waterui_gpu_content_render`.
//!    - Apple owns the presentation memory: `IOSurface`-backed `MTLTexture`s
//!      shown as a layer's `contents`. It declares the target format once with
//!      `waterui_gpu_content_prepare_metal_texture` and renders each frame with
//!      `waterui_gpu_content_render_to_metal_texture`; `attach`, `detach` and
//!      `render` panic there.
//! 3. Rendering whenever the installed redraw callback fires.
//! 4. `waterui_gpu_content_drop` when the semantic view is destroyed.
//!
//! # Thread affinity
//!
//! `WuiGpuContentState` is single-threaded state: every entry point taking one
//! runs on the thread that created it. The only cross-thread contract is the
//! installed redraw callback, which the content may fire from any thread.

use core::ffi::c_void;
use std::sync::Arc;

use alloc::boxed::Box;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
use alloc::vec;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    objc2::{rc::Retained, runtime::ProtocolObject},
    objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureType},
    wgpu_hal::{Api, api::Metal as MetalApi},
};

use waterui_core::Str;
use waterui_core::layout::Size;
use waterui_graphics::cherenkov::{Display, Next};
use waterui_graphics::cherenkov_gpu;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
use waterui_graphics::gpu::SharedGpuContext;
use waterui_graphics::gpu::{
    GpuContentHandle, GpuContentRenderer, GpuContentView, GpuRuntime, RedrawHandle,
};
use waterui_graphics::offscreen::OffscreenSize;

use crate::components::layouting::layout::{WuiProposalSize, WuiSize, WuiViewDimensions};
use crate::{IntoFFI, IntoRust, WuiStr};

/// FFI representation of a [`GpuContentView`].
///
/// The native backend consumes it with `waterui_gpu_content_create`, then owns
/// the returned state for the semantic view lifetime.
#[repr(C)]
#[derive(Debug)]
pub struct WuiGpuContent {
    /// Opaque pointer to the boxed `GpuContentView`, consumed by
    /// `waterui_gpu_content_create` and null afterwards.
    pub view: *mut c_void,
    /// Whether the content has an intrinsic size (`intrinsic_size` is then
    /// meaningful); otherwise it fills whatever layout offers.
    pub has_intrinsic_size: bool,
    /// The content's natural size in logical points.
    pub intrinsic_size: WuiSize,
    /// Whether every pixel the content draws is opaque.
    pub is_opaque: bool,
    /// Whether the view takes keyboard, pointer, IME and scroll events through
    /// `waterui_gpu_content_send_input_event`.
    pub wants_input_events: bool,
}

impl IntoFFI for GpuContentView {
    type FFI = WuiGpuContent;

    fn into_ffi(self) -> Self::FFI {
        let intrinsic_size = self.intrinsic_size();
        let is_opaque = self.is_opaque();
        let wants_input_events = self.wants_input_events();
        let view = Box::into_raw(Box::new(self)).cast::<c_void>();
        WuiGpuContent {
            view,
            has_intrinsic_size: intrinsic_size.is_some(),
            intrinsic_size: intrinsic_size.unwrap_or(Size::new(0.0, 0.0)).into_ffi(),
            is_opaque,
            wants_input_events,
        }
    }
}

// Generate waterui_gpu_content_id() and waterui_force_as_gpu_content()
ffi_view!(GpuContentView, WuiGpuContent, gpu_content);

/// Opaque state held by the native backend after initialization.
pub struct WuiGpuContentState {
    runtime: GpuRuntime,
    view: GpuContentView,
    /// The content once it has been handed to the engine — a shareable handle
    /// so a rebuilt context re-installs the same content object.
    content: Option<GpuContentHandle>,
    renderer: Option<GpuContentRenderer>,
    /// The format the content was set up for; `None` until the first target
    /// declares one. It never changes afterwards.
    format: Option<wgpu::TextureFormat>,
    /// Native presentation surface currently attached to this content.
    ///
    /// Android may replace the underlying `ANativeWindow` while preserving the
    /// `GpuContentView` and its persistent resources. Apple platforms present
    /// through host-owned textures instead and never attach a swapchain.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    surface: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,
    /// The `ANativeWindow` `layer` was — kept so the surface can be recreated
    /// on a rebuilt runtime after device loss without another attach call.
    /// Valid while `surface` is `Some`; stale otherwise.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    attached_layer: *mut c_void,
    /// The HDR preference `attach` configured the surface with.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    attached_prefers_hdr: bool,
    /// The physical size the attached surface was last configured at.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    surface_size: (u32, u32),
    /// The [`SharedGpuContext`] generation `surface` and `renderer` were built
    /// under. Anything else means they belong to a dead device and must be
    /// recreated before the next frame.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    context_generation: u64,
    redraw: RedrawHandle,
    /// The redraw waker installed by the host; the handle above fires it.
    waker: Arc<arc_swap::ArcSwapOption<ForeignRedrawTarget>>,
    /// Whether the content asked for a frame since the last render.
    dirty: Arc<core::sync::atomic::AtomicBool>,
}

impl core::fmt::Debug for WuiGpuContentState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WuiGpuContentState")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl WuiGpuContentState {
    /// Whether the semantic view takes its own input.
    pub(super) const fn wants_input_events(&self) -> bool {
        self.view.wants_input_events()
    }

    /// The view's text caret, in logical view-local coordinates.
    pub(super) fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.view.ime_caret()
    }

    fn prepare_format(&mut self, format: wgpu::TextureFormat) {
        if let Some(existing) = self.format {
            assert_eq!(existing, format, "native target format changed after setup");
        } else {
            self.format = Some(format);
            self.redraw.request_redraw();
        }
    }

    /// Measures the content against a layout proposal without touching
    /// presentation resources.
    ///
    /// `None` means the content moved to the engine already and answers
    /// through the retained handle; a content that never measured reports the
    /// proposal filled.
    pub(crate) fn measure(
        &self,
        proposal: waterui_core::layout::ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        self.view.measure(proposal)
    }

    /// The content's engine-facing handle, created on first use and kept so a
    /// rebuilt context re-installs the same content object.
    fn content_handle(&mut self) -> GpuContentHandle {
        if self.content.is_none() {
            self.content = Some(self.view.engine_content());
        }
        self.content
            .as_ref()
            .expect("content handle created above")
            .clone()
    }

    /// Ensures the state's device-bound resources run on the runtime's current
    /// context.
    ///
    /// On a generation mismatch — the driver reported the device lost and the
    /// runtime rebuilt it — the renderer and the swapchain are dropped and
    /// recreated on the fresh context: the renderer is recreated lazily by
    /// `render_into` from the retained content handle, and the surface is
    /// recreated here from the retained native layer.
    ///
    /// Android reattaches the surface without reconfiguring the FFI state: the
    /// still-attached `layer` is the handle `create_attached_surface` needs to
    /// rebuild the native binding on the new device.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    fn ensure_current_context(&mut self) {
        let context = self.runtime.context();
        if context.generation() == self.context_generation {
            return;
        }
        self.renderer = None;
        if self.surface.is_some() {
            let (surface, config) = create_attached_surface(
                &context,
                self.attached_layer,
                self.surface_size,
                self.format,
                self.attached_prefers_hdr,
                "waterui_gpu_content_render",
            );
            self.surface = Some((surface, config));
        }
        self.context_generation = context.generation();
    }

    /// Renders through the retained engine and presents its composed texture.
    fn render_into(
        &mut self,
        texture: &wgpu::Texture,
        format: wgpu::TextureFormat,
        (width, height): (u32, u32),
        display: Display,
    ) -> bool {
        assert_eq!(texture.format(), format, "native texture format mismatch");
        self.dirty
            .store(false, core::sync::atomic::Ordering::Release);
        self.view.frame();
        if self.renderer.is_none() {
            let redraw = self.redraw.clone();
            let content =
                cherenkov_gpu::interop::GpuContentBox::new(self.content_handle(), move || {
                    redraw.request_redraw();
                });
            self.renderer = Some(GpuContentRenderer::new(
                self.runtime.clone(),
                content,
                OffscreenSize::try_from_pixels(width, height)
                    .expect("native target must be nonempty"),
            ));
        }
        let renderer = self.renderer.as_mut().expect("renderer created above");
        renderer.present(texture, display) != Next::Idle
            || self.dirty.load(core::sync::atomic::Ordering::Acquire)
    }
}

/// Runs `use_view` on the state's view.
pub(super) fn with_view<T>(
    state: &WuiGpuContentState,
    use_view: impl FnOnce(&GpuContentView) -> T,
) -> T {
    use_view(&state.view)
}

/// Native callback invoked when idle content becomes dirty.
pub type WuiGpuContentRedrawCallback = unsafe extern "C" fn(context: *mut c_void);

struct ForeignRedrawTarget {
    context: usize,
    wake: WuiGpuContentRedrawCallback,
    drop: WuiGpuContentRedrawCallback,
}

// SAFETY: native installs callbacks whose context is explicitly documented as
// callable and releasable from any thread. The target owns that context until
// its `drop` callback runs.
unsafe impl Send for ForeignRedrawTarget {}
// SAFETY: see the `Send` implementation. The wake callback must be thread-safe.
unsafe impl Sync for ForeignRedrawTarget {}

impl ForeignRedrawTarget {
    fn wake(&self) {
        // SAFETY: `wake` and `context` were registered together by the backend, and
        // the context outlives this waker.
        unsafe { (self.wake)(self.context as *mut c_void) };
    }
}

impl Drop for ForeignRedrawTarget {
    fn drop(&mut self) {
        // SAFETY: `drop` and `context` are one registration from the backend, and
        // `Drop` runs once.
        unsafe { (self.drop)(self.context as *mut c_void) };
    }
}

/// Whether the process was asked to fake one device loss, for end-to-end
/// recovery checks on real Android drivers that do not lose on demand.
///
/// `WATERUI_SIMULATE_GPU_LOSS` is a testing hook: when set, the first render
/// call marks the shared context lost and `ensure_current_context` takes the
/// same rebuild a driver-reported loss would.
#[cfg(target_os = "android")]
fn simulate_device_loss_requested() -> bool {
    static REQUESTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *REQUESTED.get_or_init(|| std::env::var_os("WATERUI_SIMULATE_GPU_LOSS").is_some())
}

/// Marks the shared context lost exactly once per process when the
/// `WATERUI_SIMULATE_GPU_LOSS` test hook is set.
#[cfg(target_os = "android")]
fn simulate_device_loss_once(state: &WuiGpuContentState) {
    static FIRED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if !FIRED.swap(true, core::sync::atomic::Ordering::Relaxed) && simulate_device_loss_requested()
    {
        state
            .runtime
            .context()
            .mark_device_lost_for_testing("simulated loss via WATERUI_SIMULATE_GPU_LOSS");
    }
}

fn display_from_ffi(scale: f64, headroom: f64, context: &'static str) -> Display {
    assert!(
        scale.is_finite() && scale > 0.0,
        "{context}: scale must be a positive, finite device-pixel ratio, got {scale}"
    );
    assert!(
        headroom.is_finite() && headroom > 0.0,
        "{context}: headroom must be a positive, finite HDR headroom ratio, got {headroom}"
    );
    #[allow(clippy::cast_possible_truncation)]
    let headroom = headroom as f32;
    Display { scale, headroom }
}

/// Creates the persistent state for a `GpuContent` view.
///
/// # Safety
///
/// - `content` must be a valid, unconsumed descriptor returned by
///   `waterui_force_as_gpu_content`.
/// - `env` must remain valid for this call and hold a GPU runtime
///   (`waterui_env_install_gpu_runtime`).
///
/// # Panics
///
/// Panics if the descriptor was already consumed or the environment has no
/// GPU runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_create(
    content: *mut WuiGpuContent,
    env: *const crate::WuiEnv,
) -> *mut WuiGpuContentState {
    // SAFETY: the caller contract requires `content` to be a valid descriptor that
    // no one else is borrowing for this call.
    let descriptor = unsafe { &mut *content };
    assert!(
        !descriptor.view.is_null(),
        "waterui_gpu_content_create: descriptor was already consumed"
    );
    // SAFETY: the assert above proves the descriptor still owns its view, and the
    // field is nulled immediately after, so it is reclaimed once.
    let view: GpuContentView = unsafe { *Box::from_raw(descriptor.view.cast::<GpuContentView>()) };
    descriptor.view = core::ptr::null_mut();

    // SAFETY: the caller contract requires `env` to be a valid handle alive for this
    // call; it is only borrowed.
    let env = unsafe { &*env }.0.clone();
    let runtime = super::gpu_runtime::gpu_runtime(&env);

    let waker: Arc<arc_swap::ArcSwapOption<ForeignRedrawTarget>> = Arc::default();
    let dirty = Arc::new(core::sync::atomic::AtomicBool::new(false));
    let redraw = {
        let waker = Arc::clone(&waker);
        let dirty = Arc::clone(&dirty);
        RedrawHandle::new(move || {
            if !dirty.swap(true, core::sync::atomic::Ordering::AcqRel)
                && let Some(target) = waker.load().as_ref()
            {
                target.wake();
            }
        })
    };

    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let context_generation = runtime.context().generation();
    Box::into_raw(Box::new(WuiGpuContentState {
        runtime,
        view,
        content: None,
        renderer: None,
        format: None,
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        surface: None,
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        attached_layer: core::ptr::null_mut(),
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        attached_prefers_hdr: false,
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        surface_size: (0, 0),
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        context_generation,
        redraw,
        waker,
        dirty,
    }))
}

/// Installs the native wake target for content-driven redraw requests.
///
/// The wake callback may be called from any thread. `drop_callback` releases
/// `context` after the callback is replaced or the state is destroyed.
///
/// # Safety
///
/// - `state` and `context` must be valid.
/// - Both callbacks must be thread-safe and use `context` only for the lifetime
///   retained by `drop_callback`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_set_redraw_callback(
    state: *mut WuiGpuContentState,
    context: *mut c_void,
    wake: WuiGpuContentRedrawCallback,
    drop_callback: WuiGpuContentRedrawCallback,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let target = ForeignRedrawTarget {
        context: context as usize,
        wake,
        drop: drop_callback,
    };
    state.waker.store(Some(Arc::new(target)));
    if state.dirty.load(core::sync::atomic::Ordering::Acquire)
        && let Some(target) = state.waker.load().as_ref()
    {
        target.wake();
    }
}

/// Whether the content has been set up on a target and can render.
///
/// # Safety
///
/// `state` must be a valid pointer returned by [`waterui_gpu_content_create`].
#[unsafe(no_mangle)]
pub const unsafe extern "C" fn waterui_gpu_content_is_ready(
    state: *const WuiGpuContentState,
) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    state.format.is_some()
}

/// What this content says about itself, for a screen reader.
///
/// An owning [`WuiStr`], empty when the content has nothing to say — which a
/// host treats the same way it treats a view that never had a label.
///
/// # Safety
///
/// `state` must be a valid pointer returned by [`waterui_gpu_content_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_accessibility_label(
    state: *const WuiGpuContentState,
) -> WuiStr {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    Str::from(state.view.accessibility_label().unwrap_or_default()).into_ffi()
}

/// The semantic value this content carries, for a screen reader.
///
/// # Safety
///
/// `state` must be a valid pointer returned by [`waterui_gpu_content_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_accessibility_value(
    state: *const WuiGpuContentState,
) -> WuiStr {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    Str::from(state.view.accessibility_value().unwrap_or_default()).into_ffi()
}

/// Content-driven HDR preference exported to native backends before init.
///
/// `has_preference = false` means the content should follow backend/global
/// policy.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiGpuContentHdrPreference {
    /// Whether the view or its content provided an explicit HDR/SDR preference.
    pub has_preference: bool,
    /// Explicit preferred dynamic range when `has_preference` is true.
    pub prefers_hdr: bool,
}

/// Returns the HDR preference a `WuiGpuContent` declares.
///
/// The view's own `prefer_hdr_surface`/`prefer_sdr_surface` wins over the
/// content's `preferred_surface_hdr`.
///
/// This must be called before `waterui_gpu_content_create` consumes the
/// descriptor.
///
/// # Safety
///
/// - `content` must be a valid pointer obtained from
///   `waterui_force_as_gpu_content`
/// - `content` must not have been consumed by `waterui_gpu_content_create`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_hdr_preference(
    content: *const WuiGpuContent,
) -> WuiGpuContentHdrPreference {
    // SAFETY: the caller contract requires `content` to be a valid descriptor alive
    // for this call.
    let descriptor = unsafe { &*content };
    // SAFETY: a descriptor that has not been consumed holds a live `GpuContentView`;
    // the consuming path nulls the field, so a stale read cannot reach here.
    let view = unsafe { &*(descriptor.view as *const GpuContentView) };
    let explicit = view.resolved_hdr_preference();
    WuiGpuContentHdrPreference {
        has_preference: explicit.is_some(),
        prefers_hdr: explicit.unwrap_or(false),
    }
}

/// Measures the content without touching presentation resources.
///
/// # Safety
///
/// `state` must be valid and this function must run on the content's owning
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_measure(
    state: *const WuiGpuContentState,
    proposal: WuiProposalSize,
) -> WuiViewDimensions {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    // SAFETY: the caller contract makes `proposal` an owning handle from the
    // matching FFI constructor; it is consumed here and not observed again.
    measure_state(state, unsafe { proposal.into_rust() }).into_ffi()
}

pub(crate) fn measure_state(
    state: &WuiGpuContentState,
    proposal: waterui_core::layout::ProposalSize,
) -> waterui_core::layout::ViewDimensions {
    state.measure(proposal)
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn attached_surface_format(
    capabilities: &wgpu::SurfaceCapabilities,
    established: Option<wgpu::TextureFormat>,
    prefer_hdr: bool,
) -> wgpu::TextureFormat {
    established.map_or_else(
        || waterui_graphics::gpu::preferred_surface_format(capabilities, prefer_hdr),
        |format| {
            assert!(
                capabilities.formats.contains(&format),
                "waterui_gpu_content_attach: replacement surface does not support the established format {format:?}"
            );
            format
        },
    )
}

/// Creates and configures the wgpu surface for `layer` on `context`.
///
/// Split from `attach` because device-loss recovery recreates the surface from
/// the retained `layer` with the same arguments.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn create_attached_surface(
    context: &SharedGpuContext,
    layer: *mut c_void,
    (width, height): (u32, u32),
    established_format: Option<wgpu::TextureFormat>,
    prefers_hdr: bool,
    context_name: &'static str,
) -> (wgpu::Surface<'static>, wgpu::SurfaceConfiguration) {
    let surface = create_surface_from_layer(context.instance(), layer);
    let caps = surface.get_capabilities(context.adapter());
    let format = attached_surface_format(&caps, established_format, prefers_hdr);
    assert!(
        caps.present_modes.contains(&wgpu::PresentMode::Fifo),
        "{context_name}: surface does not support FIFO presentation"
    );
    let alpha_mode = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
        wgpu::CompositeAlphaMode::Opaque,
    ]
    .into_iter()
    .find(|mode| caps.alpha_modes.contains(mode))
    .unwrap_or_else(|| panic!("{context_name}: surface reports no supported composite alpha mode"));
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    super::checked_surface_configure(&surface, context.device(), &config, context_name);
    (surface, config)
}

/// Attaches a native presentation surface and sets the content up on its
/// format if this is the first target.
///
/// Android calls this when `SurfaceView` receives a replacement `Surface`.
///
/// # Safety
///
/// - `state` must be a valid pointer returned by [`waterui_gpu_content_create`].
/// - `layer` must remain valid until [`waterui_gpu_content_detach`] is called.
///
/// # Panics
///
/// Panics if a surface is already attached, if `width` or `height` is zero, or
/// if the surface does not offer the content's established format. Panics
/// unconditionally on Apple platforms.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_attach(
    state: *mut WuiGpuContentState,
    layer: *mut c_void,
    width: u32,
    height: u32,
    prefers_hdr: bool,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.surface.is_none(),
        "waterui_gpu_content_attach: native surface is already attached"
    );
    assert!(
        width > 0 && height > 0,
        "waterui_gpu_content_attach: native surface dimensions must be non-zero, got {width}x{height}"
    );

    let context = state.runtime.context();
    let (surface, config) = create_attached_surface(
        &context,
        layer,
        (width, height),
        state.format,
        prefers_hdr,
        "waterui_gpu_content_attach",
    );
    let format = config.format;
    state.surface = Some((surface, config));
    state.attached_layer = layer;
    state.attached_prefers_hdr = prefers_hdr;
    state.surface_size = (width, height);
    state.context_generation = context.generation();
    state.prepare_format(format);
}

/// Attaches a native presentation surface (non-Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_gpu_content_create`].
///
/// # Panics
///
/// Always panics: Apple hosts own their presentation memory and start the
/// content with [`waterui_gpu_content_prepare_metal_texture`].
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_attach(
    _state: *mut WuiGpuContentState,
    _layer: *mut c_void,
    _width: u32,
    _height: u32,
    _prefers_hdr: bool,
) {
    panic!(
        "waterui_gpu_content_attach: Apple hosts start the content with waterui_gpu_content_prepare_metal_texture"
    );
}

/// Detaches the current native presentation surface, keeping the content and
/// its resources.
///
/// # Safety
///
/// `state` must be valid and currently have an attached native surface.
///
/// # Panics
///
/// Panics if nothing is attached. Panics unconditionally on Apple platforms.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_detach(state: *mut WuiGpuContentState) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    drop(
        state
            .surface
            .take()
            .expect("waterui_gpu_content_detach: native surface is already detached"),
    );
}

/// Detaches the native presentation surface (non-Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_gpu_content_create`].
///
/// # Panics
///
/// Always panics: an Apple host releases its own textures.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_detach(_state: *mut WuiGpuContentState) {
    panic!("waterui_gpu_content_detach: Apple hosts own their presentation textures");
}

/// Renders one frame into the attached swapchain.
///
/// `width` and `height` are physical pixels from layout; `scale` is physical
/// pixels per logical unit for this frame. `headroom` is the display's HDR
/// headroom (peak white over SDR white; `1.0` on an SDR target).
///
/// # Returns
///
/// Whether another frame should be scheduled.
///
/// # Safety
///
/// `state` must be valid and have an attached native surface.
///
/// # Panics
///
/// Panics if `width` or `height` is zero, if `scale` or `headroom` is not
/// positive and finite, or if nothing is attached. Panics unconditionally on
/// Apple platforms.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_render(
    state: *mut WuiGpuContentState,
    width: u32,
    height: u32,
    scale: f64,
    headroom: f64,
) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        width > 0 && height > 0,
        "waterui_gpu_content_render: dimensions must be non-zero"
    );
    let display = display_from_ffi(scale, headroom, "waterui_gpu_content_render");
    #[cfg(target_os = "android")]
    simulate_device_loss_once(state);

    // Recreate the context's device generation when the driver reported the
    // previous one lost; surface and renderer are bound to the dead device and
    // must be rebuilt from the retained layer before anything else runs.
    state.ensure_current_context();

    // The whole frame is device-bound work; run_gpu_frame recovers a loss the
    // driver announces mid-frame as `None` instead of unwinding over the FFI.
    let presented = super::run_gpu_frame(
        &state.runtime.context(),
        "waterui_gpu_content_render",
        || {
            let Some((surface, config)) = state.surface.as_mut() else {
                // The content was detached while the frame was already inside
                // the C call; nothing to draw, so stay pending.
                return true;
            };
            if config.width != width || config.height != height {
                config.width = width;
                config.height = height;
                let gpu = state.runtime.context();
                super::checked_surface_configure(
                    surface,
                    gpu.device(),
                    config,
                    "waterui_gpu_content_render",
                );
                state.surface_size = (width, height);
            }
            let gpu = state.runtime.context();
            let Some(output) =
                super::acquire_surface_texture(surface, &gpu, config, "waterui_gpu_content_render")
            else {
                // Nothing was drawn, so the frame this call was asked for is
                // still pending: the host must come back for it once the
                // surface can be acquired again. Reporting it done here would
                // strand a view whose only clock is its own render loop.
                return true;
            };
            let format = output.texture.format();
            let still_pending =
                state.render_into(&output.texture, format, (width, height), display);
            output.present();
            gpu.note_frame_presented();
            still_pending
        },
    );
    // A device loss caught mid-frame leaves the frame pending exactly like a
    // skipped acquire: the next render rebuilds and draws.
    presented.unwrap_or(true)
}

/// Renders one frame into the attached swapchain (non-Apple only).
///
/// # Safety
///
/// `state` must come from [`waterui_gpu_content_create`].
///
/// # Panics
///
/// Always panics: Apple hosts render with
/// [`waterui_gpu_content_render_to_metal_texture`].
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_render(
    _state: *mut WuiGpuContentState,
    _width: u32,
    _height: u32,
    _scale: f64,
    _headroom: f64,
) -> bool {
    panic!(
        "waterui_gpu_content_render: Apple hosts render with waterui_gpu_content_render_to_metal_texture"
    );
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn metal_texture_format(texture: &ProtocolObject<dyn MTLTexture>) -> wgpu::TextureFormat {
    match texture.pixelFormat() {
        MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => panic!("GpuContent external Metal texture has unsupported format {other:?}"),
    }
}

/// Sets the content up for an external Metal render target's format.
///
/// # Safety
///
/// `state` must be valid and `texture` must point to a live `MTLTexture`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_prepare_metal_texture(
    state: *mut WuiGpuContentState,
    texture: *mut c_void,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    // SAFETY: the caller contract requires `texture` to be a live `MTLTexture` that
    // stays alive for this call; it is only borrowed to read its pixel format.
    let texture = unsafe { &*texture.cast::<ProtocolObject<dyn MTLTexture>>() };
    state.prepare_format(metal_texture_format(texture));
}

/// Renders one frame into an external Metal texture (Apple only).
///
/// `width` and `height` are physical pixels; `scale` is how many of them one
/// logical unit spans. `headroom` is the display's HDR headroom (peak white
/// over SDR white; `1.0` on an SDR target). Returns whether another frame
/// should be scheduled.
///
/// # Safety
///
/// `state` must be valid, `texture` must point to a live `MTLTexture`.
///
/// # Panics
///
/// Panics if `texture` is null, if `scale` or `headroom` is not positive and
/// finite, or if the texture's format differs from the one the content was
/// prepared for.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_render_to_metal_texture(
    state: *mut WuiGpuContentState,
    texture: *mut c_void,
    width: u32,
    height: u32,
    scale: f64,
    headroom: f64,
) -> bool {
    let display = display_from_ffi(
        scale,
        headroom,
        "waterui_gpu_content_render_to_metal_texture",
    );
    // SAFETY: the caller contract requires `state` to be a valid handle that no one
    // else is borrowing for this call.
    let state = unsafe { &mut *state };
    // SAFETY: the caller contract requires `texture` to be a live `MTLTexture`;
    // `retain` takes its own reference, so it stays alive for the render below.
    let metal_texture = unsafe {
        Retained::<ProtocolObject<dyn MTLTexture>>::retain(texture.cast())
            .expect("waterui_gpu_content_render_to_metal_texture received a null texture")
    };
    let format = metal_texture_format(&metal_texture);
    assert_eq!(
        state.format,
        Some(format),
        "waterui_gpu_content_render_to_metal_texture called before preparing this target format"
    );

    // SAFETY: `metal_texture` is the retained texture above, and the format and size
    // passed alongside it are read from that same texture, so the HAL description
    // matches the real resource.
    let hal_texture = unsafe {
        <MetalApi as Api>::Device::texture_from_raw(
            metal_texture,
            format,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu_hal::CopyExtent {
                width,
                height,
                depth: 1,
            },
            None,
        )
    };
    let desc = wgpu::TextureDescriptor {
        label: Some("GpuContent Imported Metal Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    // SAFETY: the HAL texture above was created from this runtime's device, which is
    // the device the wgpu texture is being created on.
    let wgpu_texture = unsafe {
        state.runtime.device().create_texture_from_hal::<MetalApi>(
            hal_texture,
            &desc,
            wgpu::wgt::TextureUses::COLOR_TARGET,
        )
    };
    let needs_redraw = state.render_into(&wgpu_texture, format, (width, height), display);
    // The host reads the texture after the queue drains; ordering the frame's
    // work before this empty submission is what it waits on.
    state.runtime.queue().submit([]);
    needs_redraw
}

/// Releases the content and its state.
///
/// # Safety
///
/// `state` must be a valid pointer from [`waterui_gpu_content_create`], and
/// must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_content_drop(state: *mut WuiGpuContentState) {
    // SAFETY: the caller contract makes `state` an owning handle from the matching
    // constructor that has not been dropped; clearing the waker before it falls out of
    // scope releases the backend's context first.
    unsafe {
        let state = Box::from_raw(state);
        state.waker.store(None);
    }
}

/// Creates a wgpu surface from a platform-specific layer pointer.
///
/// Apple has no arm here on purpose: every Apple host renders into a
/// host-owned `IOSurface` texture instead of presenting through a swapchain.
#[cfg(target_os = "android")]
fn create_surface_from_layer(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> wgpu::Surface<'static> {
    use raw_window_handle::{AndroidNdkWindowHandle, RawWindowHandle};
    use std::ptr::NonNull;

    let window_ptr = NonNull::new(layer).expect("ANativeWindow pointer must be non-null");
    let handle = AndroidNdkWindowHandle::new(window_ptr);

    // SAFETY: `create_surface_unsafe` requires the raw handle to stay valid for as
    // long as the returned surface. `layer` is the `ANativeWindow*` the Android
    // backend holds for this surface and releases only after dropping it.
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_window_handle::RawDisplayHandle::Android(
                    raw_window_handle::AndroidDisplayHandle::new(),
                )),
                raw_window_handle: RawWindowHandle::AndroidNdk(handle),
            })
            .expect("failed to create wgpu surface from ANativeWindow")
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
fn create_surface_from_layer(
    _instance: &wgpu::Instance,
    _layer: *mut c_void,
) -> wgpu::Surface<'static> {
    panic!("native GpuContent presentation is unsupported on this platform")
}

#[cfg(all(test, not(any(target_os = "macos", target_os = "ios"))))]
mod tests {
    use super::*;

    fn capabilities() -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats: vec![
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Bgra8UnormSrgb,
                wgpu::TextureFormat::Rgba16Float,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn established_format_wins_over_preference() {
        let format =
            attached_surface_format(&capabilities(), Some(wgpu::TextureFormat::Bgra8Unorm), true);
        assert_eq!(format, wgpu::TextureFormat::Bgra8Unorm);
    }

    #[test]
    fn hdr_preference_picks_float_format() {
        assert_eq!(
            attached_surface_format(&capabilities(), None, true),
            wgpu::TextureFormat::Rgba16Float
        );
        assert_eq!(
            attached_surface_format(&capabilities(), None, false),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
    }
}
