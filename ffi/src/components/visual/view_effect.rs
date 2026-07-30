//! FFI bindings for the ViewEffect raw view.
//!
//! This module provides the FFI interface for capturing view content and applying
//! GPU effects using wgpu.
//!
//! The native backend is responsible for:
//! 1. Creating a capture layer for the child view (CAMetalLayer on Apple, TextureView on Android)
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
use alloc::vec;
use executor_core::spawn_local;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    objc2::{rc::Retained, runtime::ProtocolObject},
    objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureType},
    wgpu_hal::api::Metal as MetalApi,
};

use waterui_graphics::RedrawHandle;
use waterui_graphics::shared_context::GpuRuntime;
use waterui_graphics::view_effect::{
    OutputSize, ViewEffectContext, ViewEffectErased, ViewEffectInput, ViewEffectOutput,
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
        unsafe {
            (self.wake)(self.context as *mut c_void);
        }
    }
}

impl Drop for ForeignRedrawTarget {
    fn drop(&mut self) {
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
    Fixed { width: u32, height: u32 },
    /// Scale factor relative to input.
    Scale { factor: f32 },
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

/// FFI representation of a ViewEffect view.
///
/// This struct is passed to the native backend when rendering the view tree.
/// The native backend should:
/// 1. Create capture and output layers
/// 2. Call `waterui_view_effect_create` immediately
/// 3. Attach the output layer once it exists
/// 4. Render the child view to the capture layer
/// 5. Call `waterui_view_effect_render` when rendering is scheduled
#[repr(C)]
pub struct WuiViewEffect {
    /// The child view to capture (pointer to WuiAnyView).
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
        let effect_ptr = Box::into_raw(effect_wrapper) as *mut c_void;

        WuiViewEffect {
            content,
            effect: effect_ptr,
            output_size,
        }
    }
}

/// Wrapper to hold ViewEffectErased for FFI calls.
struct ViewEffectRendererWrapper {
    erased: ViewEffectErased,
}

// Generate waterui_view_effect_id() and waterui_force_as_view_effect()
ffi_view!(ViewEffectErased, WuiViewEffect, view_effect, all(), any());

/// Opaque state held by the native backend after initialization.
pub struct WuiViewEffectState {
    /// Explicit environment-owned GPU runtime used by this semantic effect.
    runtime: GpuRuntime,
    /// Native presentation surface attached to this semantic effect.
    output_surface: Option<wgpu::Surface<'static>>,
    /// Configuration for the currently attached surface.
    output_config: Option<wgpu::SurfaceConfiguration>,
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_create(
    effect: *mut WuiViewEffect,
    env: *const crate::WuiEnv,
) -> *mut WuiViewEffectState {
    let wui_effect = unsafe { &mut *effect };
    assert!(
        !wui_effect.effect.is_null(),
        "waterui_view_effect_create: descriptor was already consumed"
    );
    let effect_wrapper: ViewEffectRendererWrapper =
        unsafe { *Box::from_raw(wui_effect.effect as *mut ViewEffectRendererWrapper) };
    wui_effect.effect = core::ptr::null_mut();
    let output_size: OutputSize = wui_effect.output_size.into();

    let runtime = super::gpu_runtime::gpu_runtime(&unsafe { &*env }.0);
    let redraw_handle = effect_wrapper.erased.redraw_handle();
    Box::into_raw(Box::new(WuiViewEffectState {
        runtime,
        output_surface: None,
        output_config: None,
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
    }))
}

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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_attach(
    state: *mut WuiViewEffectState,
    layer: *mut c_void,
    input_width: u32,
    input_height: u32,
    prefers_hdr: bool,
) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    assert!(
        state.output_surface.is_none(),
        "waterui_view_effect_attach: output surface is already attached"
    );
    let (surface, config) =
        create_configured_surface(state, layer, input_width, input_height, prefers_hdr);
    if let Some((_, setup_output_format)) = state.setup_formats.get() {
        assert_eq!(
            setup_output_format, config.format,
            "ViewEffect output format changed after setup"
        );
    }
    state.input_width = input_width;
    state.input_height = input_height;
    state.output_width = config.width;
    state.output_height = config.height;
    state.output_config = Some(config);
    state.output_surface = Some(surface);
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_detach(state: *mut WuiViewEffectState) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let surface = state
        .output_surface
        .take()
        .expect("waterui_view_effect_detach: output surface is already detached");
    state.output_config = None;
    state.imported_texture = None;
    state.imported_format = None;
    state.input_width = 0;
    state.input_height = 0;
    state.output_width = 0;
    state.output_height = 0;
    drop(surface);
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
        let config = state
            .output_config
            .as_mut()
            .expect("ViewEffect resize requires an attached output configuration");
        config.width = output_width;
        config.height = output_height;
        let surface = state
            .output_surface
            .as_ref()
            .expect("ViewEffect resize requires an attached output surface");
        surface.configure(&state.runtime.context().device, config);
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
/// - state must be a valid pointer from waterui_view_effect_create.
/// - texture must point to a live MTLTexture for the duration of this call.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_set_input_metal_texture(
    state: *mut WuiViewEffectState,
    texture: *mut c_void,
    width: u32,
    height: u32,
) {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    ensure_dimensions(state, width, height);
    import_metal_texture(state, texture, width, height);
    let input_format = state
        .imported_format
        .expect("ViewEffect Metal input import did not provide a texture format");
    start_view_effect_setup(state, input_format);
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_render(state: *mut WuiViewEffectState) -> bool {
    let state = unsafe { crate::borrow_ffi_mut(state) };
    let input_format = state
        .imported_format
        .expect("waterui_view_effect_render: input texture was not provided");

    assert!(
        state.setup_ready.get(),
        "waterui_view_effect_render called before asynchronous setup completed"
    );
    assert_setup_input_format(state, input_format);
    let output_surface = state
        .output_surface
        .as_ref()
        .expect("waterui_view_effect_render: presentation target is detached");
    let output_config = state
        .output_config
        .as_ref()
        .expect("waterui_view_effect_render: output configuration is detached");

    // Get output texture
    let Some(output) = super::acquire_surface_texture(
        output_surface,
        &state.runtime.context().device,
        output_config,
        "waterui_view_effect_render",
    ) else {
        return false;
    };

    let input_texture = state
        .imported_texture
        .as_ref()
        .expect("waterui_view_effect_render: input texture was not provided");

    let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("ViewEffect Input View"),
        ..Default::default()
    });

    let output_view = output.texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("ViewEffect Output View"),
        format: Some(output_config.format),
        ..Default::default()
    });

    // Create input/output structs
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
        texture: &output.texture,
        view: output_view,
        format: output_config.format,
        width: state.output_width,
        height: state.output_height,
    };

    // Call effect render
    let mut effect_wrapper = state.effect_wrapper.borrow_mut();
    let effect_wrapper = effect_wrapper
        .as_mut()
        .expect("ViewEffect ready state is missing its semantic renderer");
    effect_wrapper.erased.render(&input, &effect_output);
    let needs_redraw = effect_wrapper.erased.needs_redraw();

    // Present
    output.present();

    needs_redraw
}

fn start_view_effect_setup(state: &WuiViewEffectState, input_format: wgpu::TextureFormat) {
    let output_format = state
        .output_config
        .as_ref()
        .expect("ViewEffect setup requires an attached presentation target")
        .format;
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

/// Clean up ViewEffect resources.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_view_effect_create`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_drop(state: *mut WuiViewEffectState) {
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
/// 1. Creating an IOSurface
/// 2. Creating a Metal texture backed by that IOSurface
/// 3. Passing the MTLTexture pointer to this function
///
/// # Arguments
///
/// * `state` - The ViewEffect state
/// * `mtl_texture_ptr` - Pointer to an MTLTexture
/// * `width` - Width in pixels
/// * `height` - Height in pixels
///
/// # Safety
///
/// The MTLTexture must remain valid for the lifetime of the imported texture.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn import_metal_texture(
    state: &mut WuiViewEffectState,
    mtl_texture_ptr: *mut c_void,
    width: u32,
    height: u32,
) {
    use wgpu_hal::Api;

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
            panic!(
                "view_effect::import_metal_texture: unsupported Metal format {:?}",
                other
            );
        }
    };
    assert_setup_input_format(state, wgpu_format);

    // Create HAL texture from the Metal texture
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
