//! FFI bindings for the `ViewEffect` raw view.
//!
//! This module provides the FFI interface for capturing view content and applying
//! GPU effects using wgpu.
//!
//! The native backend is responsible for:
//! 1. Creating a capture layer for the child view (`CAMetalLayer` on Apple, `TextureView` on Android)
//! 2. Creating an output layer for the effect result
//! 3. Calling `waterui_view_effect_create` immediately to consume the semantic renderer
//! 4. Attaching/detaching presentation targets independently of renderer lifetime
//! 5. Installing `waterui_view_effect_set_redraw_callback`
//! 6. Rendering the child view to the capture layer
//! 7. Calling `waterui_view_effect_render` for each scheduled render with the captured texture
//! 8. Calling `waterui_view_effect_drop` when the semantic view is destroyed

use core::ffi::c_void;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use alloc::boxed::Box;
// Only a swapchain configuration names view formats.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
use alloc::vec;
// Only the platform input-import paths drive asynchronous effect setup.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
use executor_core::spawn_local;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    objc2::{rc::Retained, runtime::ProtocolObject},
    objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureType},
    wgpu_hal::api::Metal as MetalApi,
};

use waterui_graphics::RedrawHandle;
use waterui_graphics::shared_context::{GpuRuntime, reclaim_device};
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
use waterui_graphics::view_effect::ViewEffectContext;
use waterui_graphics::view_effect::{
    OutputSize, ViewEffectErased, ViewEffectInput, ViewEffectOutput,
};

use crate::{IntoFFI, WuiAnyView};

/// Native callback invoked when an idle view-effect surface becomes dirty.
pub type WuiViewEffectRedrawCallback = unsafe extern "C" fn(context: *mut c_void);

struct ForeignRedrawTarget {
    context: usize,
    wake: WuiViewEffectRedrawCallback,
    drop: WuiViewEffectRedrawCallback,
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
        unsafe {
            (self.wake)(self.context as *mut c_void);
        }
    }
}

impl Drop for ForeignRedrawTarget {
    fn drop(&mut self) {
        // SAFETY: `drop` and `context` are one registration, and `Drop` runs once.
        unsafe {
            (self.drop)(self.context as *mut c_void);
        }
    }
}

/// FFI representation of output size.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum WuiOutputSize {
    /// Match the input view's size.
    MatchInput,
    /// Fixed pixel dimensions.
    Fixed {
        /// Output width in pixels.
        width: u32,
        /// Output height in pixels.
        height: u32,
    },
    /// Scale factor relative to input.
    Scale {
        /// Multiplier applied to the input's width and height.
        factor: f32,
    },
}

impl From<OutputSize> for WuiOutputSize {
    fn from(size: OutputSize) -> Self {
        match size {
            OutputSize::MatchInput => Self::MatchInput,
            OutputSize::Fixed { width, height } => Self::Fixed { width, height },
            OutputSize::Scale(factor) => Self::Scale { factor },
        }
    }
}

impl From<WuiOutputSize> for OutputSize {
    fn from(size: WuiOutputSize) -> Self {
        match size {
            WuiOutputSize::MatchInput => Self::MatchInput,
            WuiOutputSize::Fixed { width, height } => Self::Fixed { width, height },
            WuiOutputSize::Scale { factor } => Self::Scale(factor),
        }
    }
}

/// FFI representation of a `ViewEffect` view.
///
/// This struct is passed to the native backend when rendering the view tree.
/// The native backend should:
/// 1. Create capture and output layers
/// 2. Call `waterui_view_effect_create` immediately
/// 3. Attach the output layer once it exists
/// 4. Render the child view to the capture layer
/// 5. Call `waterui_view_effect_render` when rendering is scheduled
#[repr(C)]
#[derive(Debug)]
pub struct WuiViewEffect {
    /// The child view to capture (pointer to `WuiAnyView`).
    pub content: *mut WuiAnyView,
    /// Opaque pointer to the boxed effect renderer.
    /// This is consumed during init and should not be used after.
    pub effect: *mut c_void,
    /// Output size configuration.
    pub output_size: WuiOutputSize,
}

impl IntoFFI for ViewEffectErased {
    type FFI = WuiViewEffect;

    fn into_ffi(mut self) -> Self::FFI {
        // Capture output_size before moving self
        let output_size: WuiOutputSize = self.output_size().into();

        // Take the child view and convert to FFI
        let content = self.take_content().into_ffi();

        // Box the ViewEffectErased for FFI transfer
        // The effect renderer remains inside the erased wrapper
        let effect_wrapper = Box::new(ViewEffectRendererWrapper { erased: self });
        let effect_ptr = Box::into_raw(effect_wrapper).cast::<c_void>();

        WuiViewEffect {
            content,
            effect: effect_ptr,
            output_size,
        }
    }
}

/// Wrapper to hold `ViewEffectErased` for FFI calls.
struct ViewEffectRendererWrapper {
    erased: ViewEffectErased,
}

// Generate waterui_view_effect_id() and waterui_force_as_view_effect()
ffi_view!(ViewEffectErased, WuiViewEffect, view_effect);

/// Where a view effect's finished frames go.
///
/// Apple hands the target in per frame: the host owns a pair of
/// `IOSurface`-backed textures and shows the one this effect just finished, so
/// there is no swapchain here and nothing to present. That is not a detail of
/// how the pixels arrive — a `CAMetalLayer`'s drawable can only be read by the
/// pipeline that presented it, so an effect that owned one was invisible to
/// `CARenderer` and `cacheDisplay(in:to:)`, which is what the preview snapshot
/// and every enclosing filter capture with, and it was composited under the
/// native content it should draw over (#579).
///
/// Every other platform still presents into a surface of its own.
enum ViewEffectTarget {
    /// A swapchain this effect presents into.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// A texture the host hands in with every frame.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    HostTexture { format: wgpu::TextureFormat },
}

impl ViewEffectTarget {
    /// The format frames are drawn in, which the host texture must match.
    const fn format(&self) -> wgpu::TextureFormat {
        match self {
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::Surface { config, .. } => config.format,
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::HostTexture { format } => *format,
        }
    }

    /// Follows an output resize.
    ///
    /// Only a swapchain has one to follow, which is why this exists nowhere
    /// else: a host texture pair is made by the host, which hands the first of
    /// the new pair in with the next frame.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let Self::Surface { surface, config } = self;
        config.width = width;
        config.height = height;
        surface.configure(device, config);
    }
}

/// Resolved output size returned to native before render scheduling.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiViewEffectOutputSize {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
}

/// Opaque state held by the native backend after initialization.
pub struct WuiViewEffectState {
    /// Explicit environment-owned GPU runtime used by this semantic effect.
    runtime: GpuRuntime,
    /// Currently attached presentation target.
    output: Option<ViewEffectTarget>,
    /// Imported native capture texture.
    imported_texture: Option<wgpu::Texture>,
    /// Format of the imported texture.
    imported_format: Option<wgpu::TextureFormat>,
    /// The effect renderer. Setup temporarily moves it into its local future.
    effect_wrapper: Rc<RefCell<Option<ViewEffectRendererWrapper>>>,
    /// Handle shared with renderer-driven redraw callbacks.
    redraw_handle: RedrawHandle,
    /// Immutable input/output formats captured by the one-time setup.
    setup_formats: Cell<Option<(wgpu::TextureFormat, wgpu::TextureFormat)>>,
    /// Becomes true only after asynchronous setup completes.
    setup_ready: Rc<Cell<bool>>,
    /// Current input dimensions (from child view)
    input_width: u32,
    input_height: u32,
    /// Current output dimensions
    output_width: u32,
    output_height: u32,
    /// Output size configuration
    output_size: OutputSize,
    /// Vulkan imports of the Android capture buffers handed to this effect.
    #[cfg(target_os = "android")]
    hardware_buffer_imports: super::hardware_buffer::HardwareBufferImports,
    /// Pipelines for drawing surfaces nested in the captured subtree back into it.
    #[cfg(target_os = "android")]
    capture_compositor: super::capture_composite::CaptureCompositor,
}

impl core::fmt::Debug for WuiViewEffectState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WuiViewEffectState")
            .field("input_width", &self.input_width)
            .field("input_height", &self.input_height)
            .field("output_width", &self.output_width)
            .field("output_height", &self.output_height)
            .finish_non_exhaustive()
    }
}

/// Creates persistent state and immediately consumes the semantic effect renderer.
///
/// Presentation targets are attached later with [`waterui_view_effect_attach`],
/// so a view destroyed before layout still has one clear Rust owner.
///
/// # Safety
///
/// `effect` must be a valid, unconsumed descriptor returned by
/// `waterui_force_as_view_effect`.
/// `env` must contain an installed GPU runtime.
///
/// # Panics
///
/// Panics if `effect`'s inner effect pointer is null, meaning the descriptor
/// was already consumed by a previous call to this function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_create(
    effect: *mut WuiViewEffect,
    env: *const crate::WuiEnv,
) -> *mut WuiViewEffectState {
    // SAFETY: the caller contract requires `effect` to be a valid descriptor that no
    // one else is borrowing for this call.
    let wui_effect = unsafe { &mut *effect };
    assert!(
        !wui_effect.effect.is_null(),
        "waterui_view_effect_create: descriptor was already consumed"
    );
    let effect_wrapper: ViewEffectRendererWrapper =
        // SAFETY: the assert above proves the descriptor still owns its renderer, and
        // the field is nulled immediately after, so it is reclaimed once.
        unsafe { *Box::from_raw(wui_effect.effect.cast::<ViewEffectRendererWrapper>()) };
    wui_effect.effect = core::ptr::null_mut();
    let output_size: OutputSize = wui_effect.output_size.into();

    // SAFETY: the caller contract requires `env` to be a valid handle alive for this
    // call; it is only borrowed.
    let runtime = super::gpu_runtime::gpu_runtime(&unsafe { &*env }.0);
    #[cfg(target_os = "android")]
    let hardware_buffer_imports =
        super::hardware_buffer::HardwareBufferImports::new(runtime.clone());
    let redraw_handle = effect_wrapper.erased.redraw_handle();
    Box::into_raw(Box::new(WuiViewEffectState {
        runtime,
        output: None,
        imported_texture: None,
        imported_format: None,
        effect_wrapper: Rc::new(RefCell::new(Some(effect_wrapper))),
        redraw_handle,
        setup_formats: Cell::new(None),
        setup_ready: Rc::new(Cell::new(false)),
        input_width: 0,
        input_height: 0,
        output_width: 0,
        output_height: 0,
        output_size,
        #[cfg(target_os = "android")]
        hardware_buffer_imports,
        #[cfg(target_os = "android")]
        capture_compositor: super::capture_composite::CaptureCompositor::default(),
    }))
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn create_configured_surface(
    state: &WuiViewEffectState,
    layer: *mut c_void,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) -> (wgpu::Surface<'static>, wgpu::SurfaceConfiguration) {
    assert!(
        input_width > 0 && input_height > 0,
        "waterui_view_effect_attach: dimensions must be non-zero, got {input_width}x{input_height}"
    );
    let (output_width, output_height) = state.output_size.compute(input_width, input_height);
    assert!(
        output_width > 0 && output_height > 0,
        "waterui_view_effect_attach: output size must be non-zero, got {output_width}x{output_height}"
    );

    let gpu = state.runtime.context();
    let surface = crate::components::gpu_surface::create_surface_from_layer(&gpu.instance, layer);
    let capabilities = surface.get_capabilities(&gpu.adapter);
    let format = waterui_graphics::gpu_surface::preferred_surface_format_with_preference(
        &capabilities,
        prefers_hdr,
    );
    assert!(
        capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Fifo),
        "waterui_view_effect_attach: output surface does not support FIFO presentation"
    );
    let alpha_mode = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
        wgpu::CompositeAlphaMode::Opaque,
    ]
    .into_iter()
    .find(|mode| capabilities.alpha_modes.contains(mode))
    .expect("waterui_view_effect_attach: output surface reports no composite alpha mode");
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: output_width,
        height: output_height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&gpu.device, &config);
    (surface, config)
}

/// Attaches a native presentation target while preserving the effect renderer.
///
/// # Safety
///
/// - `state` must come from [`waterui_view_effect_create`].
/// - `layer` must remain valid until [`waterui_view_effect_detach`].
/// - The state must currently be detached.
///
/// # Panics
///
/// Panics if `state` already has an output surface attached, if
/// `input_width`/`input_height` is zero, or if the computed output size is zero.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_attach(
    state: *mut WuiViewEffectState,
    layer: *mut c_void,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.output.is_none(),
        "waterui_view_effect_attach: output surface is already attached"
    );
    let (surface, config) =
        create_configured_surface(state, layer, input_width, input_height, prefers_hdr);
    let (output_width, output_height) = (config.width, config.height);
    finish_attach(
        state,
        ViewEffectTarget::Surface { surface, config },
        input_width,
        input_height,
        output_width,
        output_height,
    );
}

/// Attaches a native presentation surface (non-Apple only).
///
/// # Safety
///
/// Unreachable on Apple, where the host starts the renderer with
/// [`waterui_view_effect_attach_host_textures`].
///
/// # Panics
///
/// Always.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_attach(
    _state: *mut WuiViewEffectState,
    _layer: *mut c_void,
    _input_width: u32,
    _input_height: u32,
    _prefers_hdr: bool,
) {
    panic!(
        "waterui_view_effect_attach: Apple hosts attach with waterui_view_effect_attach_host_textures"
    );
}

/// Records an attached target and wakes the renderer if setup already finished.
fn finish_attach(
    state: &mut WuiViewEffectState,
    output: ViewEffectTarget,
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
) {
    if let Some((_, setup_output_format)) = state.setup_formats.get() {
        assert_eq!(
            setup_output_format,
            output.format(),
            "ViewEffect output format changed after setup"
        );
    }
    state.input_width = input_width;
    state.input_height = input_height;
    state.output_width = output_width;
    state.output_height = output_height;
    state.output = Some(output);
    let _ = state.redraw_handle.take_dirty();
    if state.setup_ready.get() {
        state.redraw_handle.request_redraw();
    }
}

/// Detaches the native presentation target without destroying the effect renderer.
///
/// # Safety
///
/// `state` must be valid and currently attached.
///
/// # Panics
///
/// Panics if `state` does not currently have an output surface attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_detach(state: *mut WuiViewEffectState) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.output.is_some(),
        "waterui_view_effect_detach: output surface is already detached"
    );
    // Before the input texture goes: the imported buffers are raw Vulkan objects
    // wgpu does not defer the destruction of, so they are released here rather
    // than left to outlive the target they were captured for.
    #[cfg(target_os = "android")]
    state.hardware_buffer_imports.clear();
    state.imported_texture = None;
    state.imported_format = None;
    state.input_width = 0;
    state.input_height = 0;
    state.output_width = 0;
    state.output_height = 0;
    // Last, after the imports above: where the target is a swapchain, it is what
    // those raw objects were captured for and must outlive them.
    state.output = None;
}

/// The format an Apple host renders this effect into, with no swapchain to ask.
///
/// A `CAMetalLayer` reported capabilities to choose from; an `IOSurface` has
/// none — it is created in whatever format it is told, so the choice moves here
/// and the host asks for it with
/// [`waterui_view_effect_output_metal_pixel_format`]. These are the two the
/// layer path picked between anyway: half-float linear for an extended-range
/// presentation, sRGB-encoded 8-bit otherwise.
#[cfg(any(target_os = "macos", target_os = "ios"))]
const fn apple_presentation_format(prefers_hdr: bool) -> wgpu::TextureFormat {
    if prefers_hdr {
        wgpu::TextureFormat::Rgba16Float
    } else {
        wgpu::TextureFormat::Bgra8UnormSrgb
    }
}

/// Attaches host-owned presentation on Apple, where frames arrive per texture.
///
/// No layer is named because none is configured: the host keeps a pair of
/// `IOSurface`-backed textures, hands one to
/// [`waterui_view_effect_render_to_metal_texture`] per frame, and shows it on
/// `CALayer.contents` once that frame's fence completes. The texture format is
/// this call's answer, read back with
/// [`waterui_view_effect_output_metal_pixel_format`], and the size the host
/// must allocate comes from [`waterui_view_effect_resolve_output_size`].
///
/// # Safety
///
/// - `state` must come from [`waterui_view_effect_create`].
/// - The state must currently be detached.
///
/// # Panics
///
/// Panics if `state` already has a presentation target attached, or if
/// `input_width`/`input_height` or the computed output size is zero.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_attach_host_textures(
    state: *mut WuiViewEffectState,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.output.is_none(),
        "waterui_view_effect_attach_host_textures: output surface is already attached"
    );
    assert!(
        input_width > 0 && input_height > 0,
        "waterui_view_effect_attach_host_textures: dimensions must be non-zero, got {input_width}x{input_height}"
    );
    let (output_width, output_height) = state.output_size.compute(input_width, input_height);
    assert!(
        output_width > 0 && output_height > 0,
        "waterui_view_effect_attach_host_textures: output size must be non-zero, got {output_width}x{output_height}"
    );
    let format = apple_presentation_format(prefers_hdr);
    finish_attach(
        state,
        ViewEffectTarget::HostTexture { format },
        input_width,
        input_height,
        output_width,
        output_height,
    );
}

/// The `MTLPixelFormat` an attached effect renders its output in (Apple only).
///
/// The host allocates its `IOSurface` pair from this. It is the raw Metal enum
/// value because the presentation format has no name in the capture-format
/// alphabet the Android path uses.
///
/// # Safety
///
/// `state` must be a valid pointer from [`waterui_view_effect_create`] with a
/// presentation target attached.
///
/// # Panics
///
/// Panics if the effect is detached, or if its output format has no Metal
/// equivalent.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_output_metal_pixel_format(
    state: *const WuiViewEffectState,
) -> u32 {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    let format = state
        .output
        .as_ref()
        .expect("waterui_view_effect_output_metal_pixel_format: presentation target is detached")
        .format();
    let metal_format = match format {
        wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => MTLPixelFormat::BGRA8Unorm_sRGB,
        wgpu::TextureFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
        other => panic!(
            "waterui_view_effect_output_metal_pixel_format: {other:?} has no Metal equivalent"
        ),
    };
    u32::try_from(metal_format.0).expect("MTLPixelFormat values fit in a u32")
}

/// The output size this effect resolves an input size to.
///
/// The host allocates the textures it presents from, so unlike the swapchain
/// path the size cannot stay entirely on the Rust side.
///
/// # Safety
///
/// `state` must be a valid pointer from [`waterui_view_effect_create`].
///
/// # Panics
///
/// Panics if the resolved size is zero in either dimension.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_resolve_output_size(
    state: *const WuiViewEffectState,
    input_width: u32,
    input_height: u32,
) -> WuiViewEffectOutputSize {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    let (width, height) = state.output_size.compute(input_width, input_height);
    assert!(
        width > 0 && height > 0,
        "waterui_view_effect_resolve_output_size: output size must be non-zero, got {width}x{height} for input {input_width}x{input_height}"
    );
    WuiViewEffectOutputSize { width, height }
}

/// Installs the native wake target for renderer-driven redraw requests.
///
/// # Safety
///
/// - `state` and `context` must be valid.
/// - Both callbacks must be thread-safe and use `context` only until
///   `drop_callback` releases it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_set_redraw_callback(
    state: *mut WuiViewEffectState,
    context: *mut c_void,
    wake: WuiViewEffectRedrawCallback,
    drop_callback: WuiViewEffectRedrawCallback,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
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

/// Validates and applies new input dimensions, resizing owned textures.
///
/// Only reachable from the platform input-import entry points — the Apple Metal
/// one and the Android hardware-buffer one; every other platform feeds
/// `ViewEffect` through the Rust-side filter pipeline.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
fn ensure_dimensions(state: &mut WuiViewEffectState, width: u32, height: u32) {
    assert!(
        width > 0 && height > 0,
        "ViewEffect input dimensions must be non-zero, got {width}x{height}"
    );
    let (output_width, output_height) = state.output_size.compute(width, height);
    assert!(
        output_width > 0 && output_height > 0,
        "ViewEffect output dimensions must be non-zero, got {output_width}x{output_height}"
    );
    let input_resized = width != state.input_width || height != state.input_height;
    let output_resized = output_width != state.output_width || output_height != state.output_height;

    if input_resized {
        state.input_width = width;
        state.input_height = height;
    }
    if output_resized {
        state.output_width = output_width;
        state.output_height = output_height;
        // A host texture pair follows the size on the host's side, so only a
        // swapchain has to be reconfigured here.
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        state
            .output
            .as_mut()
            .expect("ViewEffect resize requires an attached output surface")
            .resize(&state.runtime.context().device, output_width, output_height);
    }
}

fn assert_setup_input_format(state: &WuiViewEffectState, input_format: wgpu::TextureFormat) {
    if let Some((setup_input_format, _)) = state.setup_formats.get() {
        assert_eq!(
            setup_input_format, input_format,
            "ViewEffect input format changed after setup"
        );
    }
}

/// Imports the Metal texture containing the captured child view.
///
/// # Safety
///
/// - state must be a valid pointer from `waterui_view_effect_create`.
/// - texture must point to a live `MTLTexture` for the duration of this call.
///
/// # Panics
///
/// Panics if the imported Metal texture did not resolve to a supported
/// texture format.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_set_input_metal_texture(
    state: *mut WuiViewEffectState,
    texture: *mut c_void,
    width: u32,
    height: u32,
) {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    ensure_dimensions(state, width, height);
    import_metal_texture(state, texture, width, height);
    let input_format = state
        .imported_format
        .expect("ViewEffect Metal input import did not provide a texture format");
    start_view_effect_setup(state, input_format);
}

/// Copies a captured `AHardwareBuffer` into the effect's input (Android only).
///
/// The buffer is the one `HardwareRenderer` drew the effect's child subtree
/// into. The effect keeps a wgpu texture of that buffer's size and layout —
/// created once per size, and cleared so wgpu counts it as written — which the
/// import is copied into on the GPU. The returned fence is what tells the
/// backend the copy is finished, so it must be consumed by
/// `waterui_gpu_capture_fence_on_complete` and the `Image` the buffer came from
/// closed only from that completion.
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_view_effect_create` with an
///   attached target.
/// - `buffer` must be a live `AHardwareBuffer` for the duration of this call.
///
/// # Panics
///
/// Panics if `state` is detached, or if the buffer's layout is one the effect
/// pipeline cannot sample.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_set_input_hardware_buffer(
    state: *mut WuiViewEffectState,
    buffer: *mut c_void,
) -> *mut super::gpu_surface::WuiGpuCaptureFence {
    use super::capture_format::{create_effect_input_texture, effect_input_texture_format};
    use super::hardware_buffer::{copy_hardware_buffer_into_texture, describe_hardware_buffer};

    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let buffer = buffer.cast();
    // SAFETY: the caller contract makes `buffer` a live `AHardwareBuffer` for this
    // call, which is all `describe_hardware_buffer` needs.
    let description = unsafe { describe_hardware_buffer(buffer) };
    ensure_dimensions(state, description.width, description.height);

    let input_format = effect_input_texture_format(description.format);
    assert_setup_input_format(state, input_format);
    let input_texture = match state.imported_texture.take() {
        Some(texture)
            if texture.width() == description.width
                && texture.height() == description.height
                && texture.format() == input_format =>
        {
            texture
        }
        _ => create_effect_input_texture(
            &state.runtime.context().device,
            &state.runtime.context().queue,
            input_format,
            description.width,
            description.height,
        ),
    };

    // SAFETY: as above, `buffer` is live for this call, which is when the import
    // takes its own reference on it.
    let fence = unsafe {
        copy_hardware_buffer_into_texture(
            &mut state.hardware_buffer_imports,
            buffer,
            &input_texture,
            "waterui_view_effect_set_input_hardware_buffer",
        )
    };
    state.imported_texture = Some(input_texture);
    state.imported_format = Some(input_format);
    start_view_effect_setup(state, input_format);
    fence
}

/// Copies a captured `AHardwareBuffer` into the effect's input (Android only).
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_view_effect_create` with an
/// attached target, and `buffer` a live `AHardwareBuffer`.
///
/// # Panics
///
/// Always panics: `AHardwareBuffer` capture only exists on Android.
#[cfg(not(target_os = "android"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_set_input_hardware_buffer(
    _state: *mut WuiViewEffectState,
    _buffer: *mut c_void,
) -> *mut super::gpu_surface::WuiGpuCaptureFence {
    panic!("waterui_view_effect_set_input_hardware_buffer: only supported on Android");
}

/// Draws a `GpuSurface` nested in the captured subtree into the input (Android only).
///
/// HWUI records a `SurfaceView` as a cleared hole, so a GPU surface inside an
/// effect's subtree is missing from the buffer the subtree was captured into.
/// The backend calls this once per nested surface, between handing over the
/// captured buffer and rendering the effect, with the rectangle the surface
/// occupies inside the captured content in capture pixels. The surface renders
/// one frame of its own and it is drawn into the effect's input at that
/// rectangle.
///
/// Everything runs on one queue in call order, so the capture copy, the
/// composites and the effect render need no fence between them.
///
/// # Safety
///
/// - `effect` must be a valid pointer from `waterui_view_effect_create` that has
///   already been handed one captured buffer, so its input texture exists.
/// - `surface` must be a valid pointer from `waterui_gpu_surface_create` that
///   has been attached at least once, so it has an established renderer format.
/// - Both must be used from the thread that created them.
///
/// # Panics
///
/// Panics if the effect has no input texture yet, if the surface has never been
/// attached, or if the placement is empty.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_composite_gpu_surface(
    effect: *mut WuiViewEffectState,
    surface: *mut super::gpu_surface::WuiGpuSurfaceState,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
) {
    // SAFETY: the caller contract requires `effect` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let effect = unsafe { crate::borrow_ffi_mut(effect) };
    // SAFETY: the caller contract requires `surface` to be a valid handle for a
    // different state, alive and not otherwise borrowed for this call.
    let surface = unsafe { crate::borrow_ffi_mut(surface) };
    let input_texture = effect.imported_texture.take().expect(
        "waterui_view_effect_composite_gpu_surface: no captured input has been handed over yet",
    );
    super::capture_composite::composite_gpu_surface(
        &mut effect.capture_compositor,
        surface,
        &input_texture,
        super::capture_composite::CompositePlacement {
            x,
            y,
            width,
            height,
            scale,
        },
        "waterui_view_effect_composite_gpu_surface",
    );
    effect.imported_texture = Some(input_texture);
}

/// Draws a `GpuSurface` nested in the captured subtree into the input (Android only).
///
/// # Safety
///
/// `effect` and `surface` must be valid state pointers from their matching
/// constructors.
///
/// # Panics
///
/// Always panics: nested-surface compositing only exists on Android, because
/// only there does the capture arrive with the surface missing from it.
#[cfg(not(target_os = "android"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_composite_gpu_surface(
    _effect: *mut WuiViewEffectState,
    _surface: *mut super::gpu_surface::WuiGpuSurfaceState,
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
    _scale: f64,
) {
    panic!("waterui_view_effect_composite_gpu_surface: only supported on Android");
}

/// Returns whether asynchronous effect setup has completed.
///
/// Completion also triggers the installed redraw callback.
///
/// # Safety
///
/// `state` must be a valid pointer returned by [`waterui_view_effect_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_is_ready(state: *const WuiViewEffectState) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    state.setup_ready.get()
}

/// Render the effect.
///
/// This function applies the effect to the captured input and renders to the output.
///
/// # Arguments
///
/// * `state` - Pointer to attached persistent state
///
/// # Returns
///
/// Whether another frame should be scheduled immediately.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_view_effect_create` with an attached target.
///
/// # Panics
///
/// Panics if no input texture was imported before this call.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_render(state: *mut WuiViewEffectState) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let input_format = state
        .imported_format
        .expect("waterui_view_effect_render: input texture was not provided");

    assert!(
        state.setup_ready.get(),
        "waterui_view_effect_render called before asynchronous setup completed"
    );
    assert_setup_input_format(state, input_format);
    let Some(ViewEffectTarget::Surface {
        surface: output_surface,
        config: output_config,
    }) = state.output.as_ref()
    else {
        panic!("waterui_view_effect_render: presentation target is detached");
    };

    // Get output texture
    let Some(output) = super::acquire_surface_texture(
        output_surface,
        &state.runtime.context().device,
        output_config,
        "waterui_view_effect_render",
    ) else {
        // Nothing was drawn, so the frame this call was asked for is still
        // pending: the host must come back for it once the surface can be
        // acquired again. Reporting it done here would strand a view whose only
        // clock is its own render loop.
        return true;
    };

    let output_format = output_config.format;
    let needs_redraw = render_effect_into(
        state,
        &output.texture,
        output_format,
        "waterui_view_effect_render",
    );
    output.present();
    reclaim_device(&state.runtime.context().device);

    needs_redraw
}

/// Runs the semantic renderer over the imported capture, into `output`.
///
/// Both presentation paths end here: a swapchain texture this module acquired,
/// or a host-owned `IOSurface` texture handed in for the frame. What differs is
/// who owns the target and how the frame is shown, not the render.
fn render_effect_into(
    state: &WuiViewEffectState,
    output_texture: &wgpu::Texture,
    output_format: wgpu::TextureFormat,
    caller: &str,
) -> bool {
    let input_format = state
        .imported_format
        .unwrap_or_else(|| panic!("{caller}: input texture was not provided"));
    let input_texture = state
        .imported_texture
        .as_ref()
        .unwrap_or_else(|| panic!("{caller}: input texture was not provided"));

    let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("ViewEffect Input View"),
        ..Default::default()
    });
    let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("ViewEffect Output View"),
        format: Some(output_format),
        ..Default::default()
    });

    let input = ViewEffectInput {
        device: &state.runtime.context().device,
        queue: &state.runtime.context().queue,
        texture: input_texture,
        view: input_view,
        format: input_format,
        width: state.input_width,
        height: state.input_height,
    };
    let effect_output = ViewEffectOutput {
        device: &state.runtime.context().device,
        queue: &state.runtime.context().queue,
        texture: output_texture,
        view: output_view,
        format: output_format,
        width: state.output_width,
        height: state.output_height,
    };

    let mut effect_wrapper = state.effect_wrapper.borrow_mut();
    let effect_wrapper = effect_wrapper
        .as_mut()
        .expect("ViewEffect ready state is missing its semantic renderer");
    effect_wrapper.erased.render(&input, &effect_output)
}

/// Render the effect into a host-owned Metal texture (Apple only).
///
/// The returned fence completes when the GPU has finished writing `texture`;
/// the host shows it then, and not before, because Core Animation would
/// otherwise composite a half-drawn frame.
///
/// # Safety
///
/// - `state` must come from [`waterui_view_effect_create`] with a host-texture
///   target attached.
/// - `texture` must point to a live `MTLTexture` of the attached output format
///   and the resolved output size.
/// - `out_needs_redraw` must be writable.
///
/// # Panics
///
/// Panics if the effect is detached, if setup has not completed, or if the host
/// texture's format does not match the attached output format.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_render_to_metal_texture(
    state: *mut WuiViewEffectState,
    texture: *mut c_void,
    width: u32,
    height: u32,
    out_needs_redraw: *mut bool,
) -> *mut super::gpu_surface::WuiGpuCaptureFence {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    // SAFETY: the caller contract requires `texture` to be a live `MTLTexture`;
    // `retain` takes its own reference, so it outlives the render below.
    let metal_texture = unsafe {
        Retained::<ProtocolObject<dyn MTLTexture>>::retain(texture.cast())
            .expect("waterui_view_effect_render_to_metal_texture received a null texture")
    };

    assert!(
        state.setup_ready.get(),
        "waterui_view_effect_render_to_metal_texture called before asynchronous setup completed"
    );
    ensure_dimensions(state, width, height);
    let output_format = state
        .output
        .as_ref()
        .expect("waterui_view_effect_render_to_metal_texture: presentation target is detached")
        .format();
    let texture_format = match metal_texture.pixelFormat() {
        MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => panic!(
            "waterui_view_effect_render_to_metal_texture: unsupported Metal format {other:?}"
        ),
    };
    assert_eq!(
        texture_format, output_format,
        "waterui_view_effect_render_to_metal_texture: host texture format does not match the attached output format"
    );

    let (output_width, output_height) = (state.output_width, state.output_height);
    // SAFETY: `metal_texture` is the retained texture above, and the format and
    // size passed alongside it are read from that same texture and the output
    // size it was created for, so the HAL description matches the real resource.
    let hal_texture = unsafe {
        <MetalApi as wgpu_hal::Api>::Device::texture_from_raw(
            metal_texture,
            output_format,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu_hal::CopyExtent {
                width: output_width,
                height: output_height,
                depth: 1,
            },
        )
    };
    // SAFETY: the HAL texture above was created from this runtime's device, which
    // is the device the wgpu texture is created on.
    let wgpu_texture = unsafe {
        state
            .runtime
            .context()
            .device
            .create_texture_from_hal::<MetalApi>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("ViewEffect Host Presentation Texture"),
                    size: wgpu::Extent3d {
                        width: output_width,
                        height: output_height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: output_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
            )
    };

    let needs_redraw = render_effect_into(
        state,
        &wgpu_texture,
        output_format,
        "waterui_view_effect_render_to_metal_texture",
    );
    // SAFETY: the caller contract requires `out_needs_redraw` to be writable.
    unsafe { out_needs_redraw.write(needs_redraw) };

    let submission = state.runtime.context().queue.submit([]);
    let fence = super::gpu_surface::WuiGpuCaptureFence::new(
        state.runtime.context().submission_completion_driver(),
        submission,
    );
    // A frame loop that only submits never returns the resources wgpu retains
    // for a submission, so every presented frame would leak a little (#370).
    reclaim_device(&state.runtime.context().device);
    Box::into_raw(Box::new(fence))
}

/// Render the effect, presenting into the attached surface (non-Apple only).
///
/// # Safety
///
/// Unreachable on Apple, where the host renders with
/// [`waterui_view_effect_render_to_metal_texture`].
///
/// # Panics
///
/// Always.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_render(_state: *mut WuiViewEffectState) -> bool {
    panic!(
        "waterui_view_effect_render: Apple hosts render with waterui_view_effect_render_to_metal_texture"
    );
}

/// Kicks off asynchronous effect setup for the given input format.
///
/// Only reachable from the platform input-import entry points — the Apple Metal
/// one and the Android hardware-buffer one; every other platform feeds
/// `ViewEffect` through the Rust-side filter pipeline.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
fn start_view_effect_setup(state: &WuiViewEffectState, input_format: wgpu::TextureFormat) {
    let output_format = state
        .output
        .as_ref()
        .expect("ViewEffect setup requires an attached presentation target")
        .format();
    if let Some((setup_input_format, setup_output_format)) = state.setup_formats.get() {
        assert_eq!(
            setup_input_format, input_format,
            "ViewEffect input format changed after setup"
        );
        assert_eq!(
            setup_output_format, output_format,
            "ViewEffect output format changed after setup"
        );
        return;
    }

    state.setup_formats.set(Some((input_format, output_format)));
    let mut effect_wrapper = state
        .effect_wrapper
        .borrow_mut()
        .take()
        .expect("ViewEffect semantic renderer is unavailable before setup starts");
    let effect_slot = Rc::clone(&state.effect_wrapper);
    let setup_ready = Rc::clone(&state.setup_ready);
    let runtime = state.runtime.clone();
    let redraw_handle = state.redraw_handle.clone();
    spawn_local(async move {
        let gpu = runtime.context();
        let ctx = ViewEffectContext {
            device: &gpu.device,
            queue: &gpu.queue,
            input_format,
            output_format,
        };
        effect_wrapper.erased.setup(&ctx).await;
        effect_slot.replace(Some(effect_wrapper));
        setup_ready.set(true);
        redraw_handle.request_redraw();
    })
    .detach();
}

/// Clean up `ViewEffect` resources.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_view_effect_create`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_drop(state: *mut WuiViewEffectState) {
    // SAFETY: the caller contract makes `state` an owning handle from the matching
    // constructor that has not been dropped.
    unsafe {
        let _ = Box::from_raw(state);
    }
}

/// Import a Metal texture as a wgpu texture (Apple zero-copy path).
///
/// This function creates a wgpu texture that directly references the Metal texture,
/// enabling zero-copy texture sharing between the native view and the GPU effect pipeline.
///
/// The native side (Swift) is responsible for:
/// 1. Creating an `IOSurface`
/// 2. Creating a Metal texture backed by that `IOSurface`
/// 3. Passing the `MTLTexture` pointer to this function
///
/// # Arguments
///
/// * `state` - The `ViewEffect` state
/// * `mtl_texture_ptr` - Pointer to an `MTLTexture`
/// * `width` - Width in pixels
/// * `height` - Height in pixels
///
/// # Safety
///
/// The `MTLTexture` must remain valid for the lifetime of the imported texture.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn import_metal_texture(
    state: &mut WuiViewEffectState,
    mtl_texture_ptr: *mut c_void,
    width: u32,
    height: u32,
) {
    use wgpu_hal::Api;

    // SAFETY: the caller contract requires the pointer to be a live `MTLTexture`;
    // `retain` takes its own reference, so it stays alive for the work below.
    let metal_texture = unsafe {
        Retained::<ProtocolObject<dyn MTLTexture>>::retain(mtl_texture_ptr.cast())
            .expect("view_effect::import_metal_texture received a null texture")
    };

    tracing::debug!(
        "[ViewEffect] Importing Metal texture: {}x{} {:?}",
        width,
        height,
        metal_texture.pixelFormat()
    );

    let wgpu_format = match metal_texture.pixelFormat() {
        MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        MTLPixelFormat::RGBA16Float => wgpu::TextureFormat::Rgba16Float,
        other => {
            panic!("view_effect::import_metal_texture: unsupported Metal format {other:?}");
        }
    };
    assert_setup_input_format(state, wgpu_format);

    // Create HAL texture from the Metal texture
    // SAFETY: `metal_texture` is the retained texture above, described with the format
    // and size read from that same texture.
    let hal_texture = unsafe {
        <MetalApi as Api>::Device::texture_from_raw(
            metal_texture,
            wgpu_format,
            MTLTextureType::Type2D,
            1, // array_layers
            1, // mip_levels
            wgpu_hal::CopyExtent {
                width,
                height,
                depth: 1,
            },
        )
    };

    // Create wgpu texture descriptor
    let texture_desc = wgpu::TextureDescriptor {
        label: Some("ViewEffect Imported Metal Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    // Create wgpu texture from HAL texture
    // SAFETY: the HAL texture above came from this runtime's device, which is the
    // device the wgpu texture is created on.
    let wgpu_texture = unsafe {
        state
            .runtime
            .context()
            .device
            .create_texture_from_hal::<MetalApi>(hal_texture, &texture_desc)
    };

    // Store the imported texture
    state.imported_texture = Some(wgpu_texture);
    state.imported_format = Some(wgpu_format);
}
