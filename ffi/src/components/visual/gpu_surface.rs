//! FFI bindings for the `GpuSurface` raw view.
//!
//! This module provides the FFI interface for high-performance GPU rendering
//! using wgpu. Uses a shared GPU context for efficient multi-view rendering.
//!
//! The native backend is responsible for:
//! 1. Creating a native surface layer (`CAMetalLayer` on Apple, `SurfaceView` on Android)
//! 2. Creating persistent renderer state with `waterui_gpu_surface_create`
//! 3. Attaching and detaching native presentation surfaces as their lifecycle changes
//! 4. Calling `waterui_gpu_surface_render` when the redraw callback fires
//! 5. Calling `waterui_gpu_surface_drop` when the semantic view is destroyed

use core::ffi::c_void;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloc::boxed::Box;
use alloc::vec;
use executor_core::spawn_local;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    objc2::{rc::Retained, runtime::ProtocolObject},
    objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureType},
    wgpu_hal::{Api, api::Metal as MetalApi},
};

use waterui_graphics::gpu_surface::{
    GestureState, GpuContext, GpuFrame, GpuSurface, PointerState, RedrawHandle,
};
use waterui_graphics::shared_context::{GpuRuntime, GpuSubmissionCompletionDriver};

use crate::components::layouting::layout::{WuiProposalSize, WuiViewDimensions};
use crate::{IntoFFI, IntoRust};

/// FFI representation of a `GpuSurface` view.
///
/// This struct is passed to the native backend when rendering the view tree.
/// The native backend consumes it with `waterui_gpu_surface_create`, then owns
/// the returned state for the semantic view lifetime.
#[repr(C)]
#[derive(Debug)]
pub struct WuiGpuSurface {
    /// Opaque pointer to the boxed `GpuSurface`.
    /// This is consumed during state creation and should not be used after.
    pub surface: *mut c_void,
    /// Whether this surface should register as a picture-in-picture host.
    pub has_picture_in_picture_host_id: bool,
    /// Stable picture-in-picture host id when `has_picture_in_picture_host_id` is true.
    pub picture_in_picture_host_id: u64,
}

impl IntoFFI for GpuSurface {
    type FFI = WuiGpuSurface;

    fn into_ffi(self) -> Self::FFI {
        // Box the GpuSurface for FFI transfer.
        let boxed = Box::new(self);
        let picture_in_picture_host_id = boxed.picture_in_picture_host();
        let ptr = Box::into_raw(boxed).cast::<c_void>();
        WuiGpuSurface {
            surface: ptr,
            has_picture_in_picture_host_id: picture_in_picture_host_id.is_some(),
            picture_in_picture_host_id: picture_in_picture_host_id.unwrap_or(0),
        }
    }
}

// Generate waterui_gpu_surface_id() and waterui_force_as_gpu_surface()
ffi_view!(GpuSurface, WuiGpuSurface, gpu_surface);

/// Opaque state held by the native backend after initialization.
///
/// Owns the semantic renderer and environment GPU runtime independently of the
/// currently attached presentation surface.
pub struct WuiGpuSurfaceState {
    /// Explicit environment-owned GPU runtime used by this semantic view.
    runtime: GpuRuntime,
    /// Native presentation surface currently attached to this semantic GPU view.
    ///
    /// Android may replace the underlying `ANativeWindow` while preserving the
    /// `GpuView` and its persistent resources.
    wgpu_surface: Option<wgpu::Surface<'static>>,
    /// Surface configuration
    config: Option<wgpu::SurfaceConfiguration>,
    /// The format selected when asynchronous renderer setup starts.
    renderer_format: Cell<Option<wgpu::TextureFormat>>,
    /// Becomes true only after the local setup future has completed.
    setup_ready: Rc<Cell<bool>>,
    /// Maximum MSAA sample count requested by the public `GpuSurface` API.
    msaa_max_samples: core::num::NonZeroU32,
    /// Main-thread semantic renderer and environment. Setup temporarily moves
    /// this value into its local future, then returns it to the slot.
    semantic: Rc<RefCell<Option<GpuSurfaceSemantic>>>,
    /// Layout priority does not change during renderer setup.
    priority: i32,
    /// Current width from layout
    current_width: u32,
    /// Current height from layout
    current_height: u32,
    /// Current pointer/cursor state
    pointer_state: PointerState,
    /// Current gesture state (pinch, pan, double-tap)
    gesture_state: GestureState,
    /// Animation clock start for frame timing.
    start_time: Instant,
    /// Timestamp of the previous render.
    last_frame_time: Instant,
    /// Redraw handle for external redraw triggers.
    redraw_handle: RedrawHandle,
}

impl core::fmt::Debug for WuiGpuSurfaceState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WuiGpuSurfaceState")
            .field("current_width", &self.current_width)
            .field("current_height", &self.current_height)
            .finish_non_exhaustive()
    }
}

struct GpuSurfaceSemantic {
    gpu_surface: GpuSurface,
    env: waterui::Environment,
}

/// Submission fence returned after encoding an external-texture capture.
///
/// The renderer state remains on its owning UI thread. Native consumes this
/// token by registering a completion with
/// [`waterui_gpu_capture_fence_on_complete`].
#[derive(Debug)]
pub struct WuiGpuCaptureFence {
    completion_driver: GpuSubmissionCompletionDriver,
    submission: wgpu::SubmissionIndex,
}

/// Completion function invoked after an external GPU capture submission.
pub type WuiGpuCaptureCompletionCallback = unsafe extern "C" fn(context: *mut c_void);

/// Releases the foreign completion context after its callback returns.
pub type WuiGpuCaptureCompletionDrop = unsafe extern "C" fn(context: *mut c_void);

struct ForeignGpuCaptureCompletion {
    context: usize,
    callback: WuiGpuCaptureCompletionCallback,
    drop: WuiGpuCaptureCompletionDrop,
}

impl ForeignGpuCaptureCompletion {
    fn complete(self) {
        unsafe { (self.callback)(self.context as *mut c_void) };
    }
}

impl Drop for ForeignGpuCaptureCompletion {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.context as *mut c_void) };
    }
}

fn advance_frame_timing(state: &mut WuiGpuSurfaceState) -> (Duration, Duration) {
    let now = Instant::now();
    let elapsed = now.duration_since(state.start_time);
    let delta = now
        .duration_since(state.last_frame_time)
        .min(Duration::from_millis(100));
    state.last_frame_time = now;
    (elapsed, delta)
}

/// Native callback invoked when an idle `GpuSurface` becomes dirty.
pub type WuiGpuSurfaceRedrawCallback = unsafe extern "C" fn(context: *mut c_void);

struct ForeignRedrawTarget {
    context: usize,
    wake: WuiGpuSurfaceRedrawCallback,
    drop: WuiGpuSurfaceRedrawCallback,
}

// SAFETY: native installs callbacks whose context is explicitly documented as
// callable and releasable from any thread. The target owns that context until
// its `drop` callback runs.
unsafe impl Send for ForeignRedrawTarget {}
// SAFETY: see the `Send` implementation. The wake callback must be thread-safe.
unsafe impl Sync for ForeignRedrawTarget {}

impl ForeignRedrawTarget {
    fn wake(&self) {
        unsafe { (self.wake)(self.context as *mut c_void) };
    }
}

impl Drop for ForeignRedrawTarget {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.context as *mut c_void) };
    }
}

/// Renderer-driven HDR preference exported to native backends before init.
///
/// `has_preference = false` means the surface should follow backend/global policy.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiGpuSurfaceHdrPreference {
    /// Whether the renderer provided an explicit HDR/SDR preference.
    pub has_preference: bool,
    /// Explicit preferred dynamic range when `has_preference` is true.
    pub prefers_hdr: bool,
}

/// Returns the renderer-driven HDR preference for a `WuiGpuSurface`.
///
/// This must be called before `waterui_gpu_surface_create` consumes the surface.
///
/// # Safety
///
/// - `surface` must be a valid pointer obtained from `waterui_force_as_gpu_surface`
/// - `surface` must not have been consumed by `waterui_gpu_surface_create`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_hdr_preference(
    surface: *const WuiGpuSurface,
) -> WuiGpuSurfaceHdrPreference {
    let wui_surface = unsafe { &*surface };
    let gpu_surface = unsafe { &*(wui_surface.surface as *const GpuSurface) };
    let explicit = gpu_surface.resolved_hdr_preference();
    WuiGpuSurfaceHdrPreference {
        has_preference: explicit.is_some(),
        prefers_hdr: explicit.unwrap_or(false),
    }
}

fn attached_surface_format(
    capabilities: &wgpu::SurfaceCapabilities,
    renderer_format: Option<wgpu::TextureFormat>,
    prefer_hdr: bool,
) -> wgpu::TextureFormat {
    renderer_format.map_or_else(
        || {
            waterui_graphics::gpu_surface::preferred_surface_format_with_preference(
                capabilities,
                prefer_hdr,
            )
        },
        |format| {
            assert!(
                capabilities.formats.contains(&format),
                "waterui_gpu_surface_attach: replacement surface does not support the renderer's established format {format:?}"
            );
            format
        },
    )
}

fn create_attached_surface(
    runtime: &GpuRuntime,
    layer: *mut c_void,
    width: u32,
    height: u32,
    renderer_format: Option<wgpu::TextureFormat>,
    prefer_hdr: bool,
) -> (wgpu::Surface<'static>, wgpu::SurfaceConfiguration) {
    assert!(
        width > 0 && height > 0,
        "waterui_gpu_surface_attach: native surface dimensions must be non-zero, got {width}x{height}"
    );

    let gpu = runtime.context();
    let wgpu_surface = create_surface_from_layer(&gpu.instance, layer);
    let surface_caps = wgpu_surface.get_capabilities(&gpu.adapter);

    let format = attached_surface_format(&surface_caps, renderer_format, prefer_hdr);

    assert!(
        surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo),
        "waterui_gpu_surface_attach: surface does not support FIFO presentation"
    );

    let alpha_mode = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
        wgpu::CompositeAlphaMode::Opaque,
    ]
    .into_iter()
    .find(|mode| surface_caps.alpha_modes.contains(mode))
    .unwrap_or_else(|| {
        panic!("waterui_gpu_surface_attach: surface reports no supported composite alpha mode")
    });

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
    wgpu_surface.configure(&gpu.device, &config);
    (wgpu_surface, config)
}

fn start_renderer_setup(state: &WuiGpuSurfaceState, format: wgpu::TextureFormat) {
    if let Some(existing) = state.renderer_format.get() {
        assert_eq!(
            existing, format,
            "GpuSurface target format changed after renderer setup started"
        );
        return;
    }

    state.renderer_format.set(Some(format));
    let mut semantic = state
        .semantic
        .borrow_mut()
        .take()
        .expect("GpuSurface semantic renderer is unavailable before setup starts");
    let semantic_slot = Rc::clone(&state.semantic);
    let setup_ready = Rc::clone(&state.setup_ready);
    let runtime = state.runtime.clone();
    let redraw_handle = state.redraw_handle.clone();
    let msaa_samples = waterui_graphics::gpu_surface::preferred_msaa_samples(
        &runtime.context().adapter,
        format,
        state.msaa_max_samples,
    );

    spawn_local(async move {
        let gpu = runtime.context();
        let ctx = GpuContext {
            adapter: &gpu.adapter,
            device: &gpu.device,
            queue: &gpu.queue,
            surface_format: format,
            msaa_samples,
            redraw_handle: redraw_handle.clone(),
        };
        let GpuSurfaceSemantic { gpu_surface, env } = &mut semantic;
        gpu_surface.setup(&ctx, env).await;
        semantic_slot.replace(Some(semantic));
        setup_ready.set(true);
        redraw_handle.request_redraw();
    })
    .detach();
}

fn with_semantic_mut<T>(
    state: &WuiGpuSurfaceState,
    use_semantic: impl FnOnce(&mut GpuSurfaceSemantic) -> T,
) -> T {
    assert!(
        state.setup_ready.get(),
        "GpuSurface renderer used before asynchronous setup completed"
    );
    let mut semantic = state.semantic.borrow_mut();
    use_semantic(
        semantic
            .as_mut()
            .expect("GpuSurface ready state is missing its semantic renderer"),
    )
}

fn attached_surface<'a>(
    state: &'a WuiGpuSurfaceState,
    scope: &'static str,
) -> &'a wgpu::Surface<'static> {
    state
        .wgpu_surface
        .as_ref()
        .unwrap_or_else(|| panic!("{scope}: native surface is detached"))
}

fn attached_config<'a>(
    state: &'a WuiGpuSurfaceState,
    scope: &'static str,
) -> &'a wgpu::SurfaceConfiguration {
    state
        .config
        .as_ref()
        .unwrap_or_else(|| panic!("{scope}: native surface is detached"))
}

/// Creates the persistent state for one semantic `GpuSurface`.
///
/// This consumes the renderer exactly once. Native presentation surfaces are
/// attached and detached independently with [`waterui_gpu_surface_attach`] and
/// [`waterui_gpu_surface_detach`].
///
/// # Safety
///
/// - `surface` must be a valid, unconsumed descriptor returned by
///   `waterui_force_as_gpu_surface`.
/// - `env` must remain valid for this call; the state stores its own clone.
///
/// # Panics
///
/// Panics if `surface`'s inner `GpuSurface` pointer is null, meaning the
/// descriptor was already consumed by a previous call to this function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_create(
    surface: *mut WuiGpuSurface,
    env: *const crate::WuiEnv,
) -> *mut WuiGpuSurfaceState {
    let wui_surface = unsafe { &mut *surface };
    assert!(
        !wui_surface.surface.is_null(),
        "waterui_gpu_surface_create: descriptor was already consumed"
    );
    let gpu_surface: GpuSurface =
        unsafe { *Box::from_raw(wui_surface.surface.cast::<GpuSurface>()) };
    wui_surface.surface = core::ptr::null_mut();

    let msaa_max_samples = gpu_surface.msaa_sample_limit();
    let priority = gpu_surface.priority();
    let env = unsafe { &*env }.0.clone();
    let runtime = super::gpu_runtime::gpu_runtime(&env);
    let now = Instant::now();
    Box::into_raw(Box::new(WuiGpuSurfaceState {
        runtime,
        wgpu_surface: None,
        renderer_format: Cell::new(None),
        setup_ready: Rc::new(Cell::new(false)),
        msaa_max_samples,
        config: None,
        semantic: Rc::new(RefCell::new(Some(GpuSurfaceSemantic { gpu_surface, env }))),
        priority,
        current_width: 0,
        current_height: 0,
        pointer_state: PointerState::default(),
        gesture_state: GestureState::default(),
        start_time: now,
        last_frame_time: now
            .checked_sub(Duration::from_secs_f32(1.0 / 60.0))
            .unwrap(),
        redraw_handle: RedrawHandle::new(),
    }))
}

/// Measures the semantic GPU view without touching presentation resources.
///
/// # Safety
///
/// `state` must be valid and this function must run on the renderer's owning
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_measure(
    state: *const WuiGpuSurfaceState,
    proposal: WuiProposalSize,
) -> WuiViewDimensions {
    let state = unsafe { crate::borrow_ffi(state) };
    measure_state(state, unsafe { proposal.into_rust() }).into_ffi()
}

pub(crate) fn measure_state(
    state: &WuiGpuSurfaceState,
    proposal: waterui_core::layout::ProposalSize,
) -> waterui_core::layout::ViewDimensions {
    let semantic = state.semantic.borrow();
    semantic
        .as_ref()
        .expect("GpuSurface cannot be measured while asynchronous setup owns its renderer")
        .gpu_surface
        .measure(proposal)
}

/// Returns the layout priority declared by the semantic GPU view.
///
/// # Safety
///
/// `state` must be valid and this function must run on the renderer's owning
/// thread.
#[unsafe(no_mangle)]
pub const unsafe extern "C" fn waterui_gpu_surface_priority(
    state: *const WuiGpuSurfaceState,
) -> i32 {
    let state = unsafe { crate::borrow_ffi(state) };
    priority_state(state)
}

pub(crate) const fn priority_state(state: &WuiGpuSurfaceState) -> i32 {
    state.priority
}

/// Replaces the native presentation surface while preserving the semantic
/// `GpuView` and its persistent renderer resources.
///
/// Android calls this when `SurfaceView` receives a replacement `Surface`.
///
/// # Safety
///
/// - `state` must be a valid pointer returned by [`waterui_gpu_surface_create`].
/// - `layer` must remain valid until [`waterui_gpu_surface_detach`] is called.
/// - The state must currently be detached.
///
/// # Panics
///
/// Panics if `state` already has a native surface attached, or if `width` or
/// `height` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_attach(
    state: *mut WuiGpuSurfaceState,
    layer: *mut c_void,
    width: u32,
    height: u32,
    prefers_hdr: bool,
) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.wgpu_surface.is_none(),
        "waterui_gpu_surface_attach: native surface is already attached"
    );

    let (surface, config) = create_attached_surface(
        &state.runtime,
        layer,
        width,
        height,
        state.renderer_format.get(),
        prefers_hdr,
    );

    state.current_width = width;
    state.current_height = height;
    let format = config.format;
    state.config = Some(config);
    state.wgpu_surface = Some(surface);
    start_renderer_setup(state, format);
}

/// Detaches the current native presentation surface without destroying the
/// semantic `GpuView` or its persistent renderer resources.
///
/// # Safety
///
/// `state` must be valid and currently have an attached native surface.
///
/// # Panics
///
/// Panics if `state` does not currently have a native surface attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_detach(state: *mut WuiGpuSurfaceState) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let surface = state
        .wgpu_surface
        .take()
        .expect("waterui_gpu_surface_detach: native surface is already detached");
    drop(surface);
    state.config = None;
}

/// Installs the native wake target for renderer-driven redraw requests.
///
/// The wake callback may be called from any thread. `drop_callback` releases
/// `context` after the callback is replaced or the GPU state is destroyed.
///
/// # Safety
///
/// - `state` and `context` must be valid.
/// - Both callbacks must be thread-safe and use `context` only for the lifetime
///   retained by `drop_callback`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_set_redraw_callback(
    state: *mut WuiGpuSurfaceState,
    context: *mut c_void,
    wake: WuiGpuSurfaceRedrawCallback,
    drop_callback: WuiGpuSurfaceRedrawCallback,
) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let target = ForeignRedrawTarget {
        context: context as usize,
        wake,
        drop: drop_callback,
    };
    state
        .redraw_handle
        .set_waker(Some(Arc::new(move || target.wake())));
}

/// Returns whether the renderer's asynchronous setup has completed.
///
/// Setup completion also triggers the installed redraw callback, so native
/// backends can use this value to resolve first-paint readiness without polling.
///
/// # Safety
///
/// `state` must be a valid pointer returned by [`waterui_gpu_surface_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_is_ready(state: *const WuiGpuSurfaceState) -> bool {
    let state = unsafe { crate::borrow_ffi(state) };
    state.setup_ready.get()
}

/// Render a single frame.
///
/// This function should be called when the surface is dirty (size/input/state changed)
/// and backend should schedule another frame when `needs_redraw` is true.
///
/// # Arguments
///
/// * `state` - Pointer to the persistent state from `waterui_gpu_surface_create`
/// * `width` - Current surface width in pixels (from layout)
/// * `height` - Current surface height in pixels (from layout)
///
/// # Returns
///
/// Whether another frame should be scheduled immediately.
///
/// # Safety
///
/// `state` must be valid and have an attached native surface.
///
/// # Panics
///
/// Panics if `width` or `height` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_render(
    state: *mut WuiGpuSurfaceState,
    width: u32,
    height: u32,
) -> bool {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        width > 0 && height > 0,
        "waterui_gpu_surface_render: dimensions must be non-zero"
    );

    // Handle resize if needed
    if width != state.current_width || height != state.current_height {
        {
            let config = state
                .config
                .as_mut()
                .expect("waterui_gpu_surface_render: native surface is detached");
            config.width = width;
            config.height = height;
        }

        attached_surface(state, "waterui_gpu_surface_render").configure(
            &state.runtime.context().device,
            attached_config(state, "waterui_gpu_surface_render"),
        );
        state.current_width = width;
        state.current_height = height;
    }

    let format = attached_config(state, "waterui_gpu_surface_render").format;
    assert!(
        state.setup_ready.get(),
        "waterui_gpu_surface_render called before asynchronous setup completed"
    );

    let Some(output) = super::acquire_surface_texture(
        attached_surface(state, "waterui_gpu_surface_render"),
        &state.runtime.context().device,
        attached_config(state, "waterui_gpu_surface_render"),
        "waterui_gpu_surface_render",
    ) else {
        return false;
    };
    let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("GpuSurface Frame View"),
        format: Some(format),
        ..Default::default()
    });

    let (elapsed, delta) = advance_frame_timing(state);

    // Create frame data
    let mut frame = GpuFrame::new(
        &state.runtime.context().device,
        &state.runtime.context().queue,
        &output.texture,
        view,
        format,
        width,
        height,
        state.pointer_state,
        state.gesture_state,
        elapsed,
        delta,
    );

    // The dirty request that scheduled this frame is now satisfied. A new
    // request arriving during the renderer callback remains pending below.
    let _ = state.redraw_handle.take_dirty();
    with_semantic_mut(state, |semantic| {
        semantic.gpu_surface.render(&mut frame);
    });
    let needs_redraw = frame.was_redraw_requested() || state.redraw_handle.take_dirty();

    output.present();

    needs_redraw
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn metal_texture_format(texture: &ProtocolObject<dyn MTLTexture>) -> wgpu::TextureFormat {
    match texture.pixelFormat() {
        MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => panic!("GpuSurface external Metal texture has unsupported format {other:?}"),
    }
}

/// Starts asynchronous renderer setup for an external Metal render target.
///
/// Completion triggers the installed redraw callback. Native must wait until
/// [`waterui_gpu_surface_is_ready`] returns true before rendering into the texture.
///
/// # Safety
///
/// `state` must be valid and `texture` must point to a live `MTLTexture`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_prepare_metal_texture(
    state: *mut WuiGpuSurfaceState,
    texture: *mut c_void,
) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let texture = unsafe { &*texture.cast::<ProtocolObject<dyn MTLTexture>>() };
    start_renderer_setup(state, metal_texture_format(texture));
}

/// Render a single frame into an external Metal texture (Apple only).
///
/// # Safety
/// `state` must be valid, `texture` must point to a `MTLTexture`.
///
/// # Panics
///
/// Panics if `texture` is null.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_render_to_metal_texture(
    state: *mut WuiGpuSurfaceState,
    texture: *mut core::ffi::c_void,
    width: u32,
    height: u32,
) -> *mut WuiGpuCaptureFence {
    let state = unsafe { &mut *state };
    let metal_texture = unsafe {
        Retained::<ProtocolObject<dyn MTLTexture>>::retain(texture.cast())
            .expect("waterui_gpu_surface_render_to_metal_texture received a null texture")
    };

    let target_format = metal_texture_format(&metal_texture);
    assert_eq!(
        state.renderer_format.get(),
        Some(target_format),
        "waterui_gpu_surface_render_to_metal_texture called before preparing this target format"
    );
    assert!(
        state.setup_ready.get(),
        "waterui_gpu_surface_render_to_metal_texture called before asynchronous setup completed"
    );

    let hal_texture = unsafe {
        <MetalApi as Api>::Device::texture_from_raw(
            metal_texture,
            target_format,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu_hal::CopyExtent {
                width,
                height,
                depth: 1,
            },
        )
    };
    let texture_desc = wgpu::TextureDescriptor {
        label: Some("GpuSurface Imported Metal Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };

    let wgpu_texture = unsafe {
        state
            .runtime
            .context()
            .device
            .create_texture_from_hal::<MetalApi>(hal_texture, &texture_desc)
    };
    let view = wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("GpuSurface Metal Frame View"),
        format: Some(target_format),
        ..Default::default()
    });
    let (elapsed, delta) = advance_frame_timing(state);

    let mut frame = GpuFrame::new(
        &state.runtime.context().device,
        &state.runtime.context().queue,
        &wgpu_texture,
        view,
        target_format,
        width,
        height,
        state.pointer_state,
        state.gesture_state,
        elapsed,
        delta,
    );

    let _ = state.redraw_handle.take_dirty();
    with_semantic_mut(state, |semantic| {
        semantic.gpu_surface.render(&mut frame);
    });
    if frame.was_redraw_requested() || state.redraw_handle.take_dirty() {
        state.redraw_handle.request_redraw();
    }
    let submission = state.runtime.context().queue.submit([]);
    Box::into_raw(Box::new(WuiGpuCaptureFence {
        completion_driver: state.runtime.context().submission_completion_driver(),
        submission,
    }))
}

/// Schedules one external capture submission completion and consumes its fence.
///
/// `callback` runs exactly once on the shared GPU completion thread. `drop`
/// then runs exactly once on that same thread, including if the completion
/// driver fails before invoking `callback`. Native callbacks should only
/// enqueue their next platform-specific stage and return immediately.
///
/// # Safety
///
/// `fence` must be a valid owning pointer returned by
/// [`waterui_gpu_surface_render_to_metal_texture`] and must be consumed once.
/// `context`, `callback`, and `drop` must remain valid until completion; Rust
/// consumes the context and releases it through `drop`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_capture_fence_on_complete(
    fence: *mut WuiGpuCaptureFence,
    context: *mut c_void,
    callback: WuiGpuCaptureCompletionCallback,
    drop: WuiGpuCaptureCompletionDrop,
) {
    let fence = unsafe { Box::from_raw(fence) };
    let completion = ForeignGpuCaptureCompletion {
        context: context as usize,
        callback,
        drop,
    };
    fence
        .completion_driver
        .on_complete(fence.submission, move || completion.complete());
}

/// Clean up GPU resources.
///
/// This function should be called when the `GpuSurface` view is destroyed.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_create`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_drop(state: *mut WuiGpuSurfaceState) {
    unsafe {
        let state = Box::from_raw(state);
        state.redraw_handle.set_waker(None);
    }
}

/// FFI-safe pointer state for passing from native.
///
/// Native backends should update this before each render call to provide
/// current pointer/cursor information to the GPU renderer.
#[repr(C)]
#[derive(Debug)]
pub struct WuiPointerState {
    /// Whether the pointer is currently over this surface.
    pub has_position: bool,
    /// X coordinate in surface-local pixels.
    pub x: f32,
    /// Y coordinate in surface-local pixels.
    pub y: f32,
    /// Whether there's an active hit (press/touch in progress).
    pub has_hit: bool,
    /// X coordinate where hit started.
    pub hit_x: f32,
    /// Y coordinate where hit started.
    pub hit_y: f32,
}

#[inline]
const fn pointer_state_from_ffi(pointer: &WuiPointerState) -> PointerState {
    PointerState {
        position: if pointer.has_position {
            Some(waterui_core::layout::Point::new(pointer.x, pointer.y))
        } else {
            None
        },
        hit: if pointer.has_hit {
            Some(waterui_core::layout::Point::new(
                pointer.hit_x,
                pointer.hit_y,
            ))
        } else {
            None
        },
    }
}

/// FFI-safe gesture state for zoom/pan interactions.
///
/// Native backends should update this when pinch, pan, or double-tap
/// gestures are detected to enable interactive chart zoom/pan.
#[repr(C)]
#[derive(Debug)]
pub struct WuiGestureState {
    /// Whether a gesture is currently active.
    pub active: bool,
    /// Cumulative pinch scale factor (1.0 = no scaling).
    pub pinch_scale: f32,
    /// Whether a pinch center is present.
    pub has_pinch_center: bool,
    /// X coordinate of pinch center in surface-local pixels.
    pub pinch_center_x: f32,
    /// Y coordinate of pinch center in surface-local pixels.
    pub pinch_center_y: f32,
    /// Pan offset X in pixels since gesture began.
    pub pan_offset_x: f32,
    /// Pan offset Y in pixels since gesture began.
    pub pan_offset_y: f32,
    /// Whether a double-tap was detected this frame.
    pub double_tap: bool,
}

#[inline]
const fn gesture_state_from_ffi(gesture: &WuiGestureState) -> GestureState {
    GestureState {
        pinch_scale: gesture.pinch_scale,
        pinch_center: if gesture.has_pinch_center {
            Some(waterui_core::layout::Point::new(
                gesture.pinch_center_x,
                gesture.pinch_center_y,
            ))
        } else {
            None
        },
        pan_offset: waterui_core::layout::Point::new(gesture.pan_offset_x, gesture.pan_offset_y),
        double_tap: gesture.double_tap,
        active: gesture.active,
    }
}

/// FFI-safe combined input state for a `GpuSurface`.
///
/// This keeps the native bridge minimal by forwarding pointer and gesture
/// snapshots in one call.
#[repr(C)]
#[derive(Debug)]
pub struct WuiGpuSurfaceInput {
    /// Current pointer snapshot.
    pub pointer: WuiPointerState,
    /// Current gesture snapshot.
    pub gesture: WuiGestureState,
}

/// Update both pointer and gesture state for a `GpuSurface`.
///
/// Native backends should prefer this API to minimize bridge calls.
///
/// # Arguments
///
/// * `state` - Pointer to the persistent state from `waterui_gpu_surface_create`
/// * `input` - Combined pointer + gesture snapshot
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_gpu_surface_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_set_input(
    state: *mut WuiGpuSurfaceState,
    input: WuiGpuSurfaceInput,
) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    state.pointer_state = pointer_state_from_ffi(&input.pointer);
    state.gesture_state = gesture_state_from_ffi(&input.gesture);
}

/// Create a wgpu Surface from a platform-specific layer pointer.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn create_surface_from_layer(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> wgpu::Surface<'static> {
    // Apple backends provide a CAMetalLayer pointer.
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
            .expect("failed to create wgpu surface from CAMetalLayer")
    }
}

#[cfg(target_os = "android")]
pub(crate) fn create_surface_from_layer(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> wgpu::Surface<'static> {
    use raw_window_handle::{AndroidNdkWindowHandle, RawWindowHandle};
    use std::ptr::NonNull;

    // On Android, layer is an ANativeWindow*
    let window_ptr = NonNull::new(layer).expect("ANativeWindow pointer must be non-null");
    let handle = AndroidNdkWindowHandle::new(window_ptr);

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
pub(crate) fn create_surface_from_layer(
    _instance: &wgpu::Instance,
    _layer: *mut c_void,
) -> wgpu::Surface<'static> {
    panic!("native GpuSurface presentation is unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface_capabilities() -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats: vec![
                wgpu::TextureFormat::Bgra8UnormSrgb,
                wgpu::TextureFormat::Rgba16Float,
            ],
            ..wgpu::SurfaceCapabilities::default()
        }
    }

    #[test]
    fn first_surface_uses_dynamic_range_preference() {
        let capabilities = surface_capabilities();
        assert_eq!(
            attached_surface_format(&capabilities, None, false),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            attached_surface_format(&capabilities, None, true),
            wgpu::TextureFormat::Rgba16Float
        );
    }

    #[test]
    fn replacement_surface_reuses_established_renderer_format() {
        let capabilities = surface_capabilities();
        assert_eq!(
            attached_surface_format(&capabilities, Some(wgpu::TextureFormat::Rgba16Float), false,),
            wgpu::TextureFormat::Rgba16Float
        );
    }

    #[test]
    #[should_panic(
        expected = "replacement surface does not support the renderer's established format"
    )]
    fn replacement_surface_rejects_unsupported_renderer_format() {
        let capabilities = wgpu::SurfaceCapabilities {
            formats: vec![wgpu::TextureFormat::Bgra8UnormSrgb],
            ..wgpu::SurfaceCapabilities::default()
        };
        let _ =
            attached_surface_format(&capabilities, Some(wgpu::TextureFormat::Rgba16Float), true);
    }
}
