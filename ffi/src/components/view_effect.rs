//! FFI bindings for the ViewEffect raw view.
//!
//! This module provides the FFI interface for capturing view content and applying
//! GPU effects using wgpu.
//!
//! The native backend is responsible for:
//! 1. Creating a capture layer for the child view (CAMetalLayer on Apple, TextureView on Android)
//! 2. Creating an output layer for the effect result
//! 3. Calling `waterui_view_effect_init` with both layer pointers
//! 4. Rendering the child view to the capture layer
//! 5. Calling `waterui_view_effect_render` each frame with the captured texture
//! 6. Calling `waterui_view_effect_drop` when the view is destroyed

use core::ffi::c_void;
use std::sync::Arc;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::vec;

// Platform-specific imports for Metal HAL texture import
#[cfg(any(target_os = "macos", target_os = "ios"))]
use {metal::MTLTextureType, wgpu_hal::api::Metal as MetalApi};

// Platform-specific imports for Vulkan HAL texture import (Android)
#[cfg(target_os = "android")]
use crate::components::android_ahb;

/// Native drop callback type for external resources.
///
/// Android: used to release an acquired `AHardwareBuffer*` without Rust linking to API-26+ symbols.
pub type WuiExternalDropFn = unsafe extern "C" fn(user_data: *mut c_void);

use waterui_graphics::shared_context::shared_context;
use waterui_graphics::view_effect::{
    EffectContext, EffectInput, EffectOutput, OutputSize, ViewEffectErased,
};

use crate::{IntoFFI, WuiAnyView};

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
/// 2. Call `waterui_view_effect_init` to initialize GPU resources
/// 3. Render the child view to the capture layer
/// 4. Call `waterui_view_effect_render` each frame
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
ffi_view!(ViewEffectErased, WuiViewEffect, view_effect);

/// Opaque state held by the native backend after initialization.
pub struct WuiViewEffectState {
    /// Shared device (Arc reference from SharedGpuContext)
    device: Arc<wgpu::Device>,
    /// Shared queue (Arc reference from SharedGpuContext)
    queue: Arc<wgpu::Queue>,
    /// Optional shared pipeline cache
    pipeline_cache: Option<wgpu::PipelineCache>,
    /// Per-view wgpu surface for output
    output_surface: wgpu::Surface<'static>,
    /// Output surface configuration
    output_config: wgpu::SurfaceConfiguration,
    /// Capture texture (for non-GpuSurface children)
    capture_texture: Option<wgpu::Texture>,
    /// Imported texture from external source (IOSurface/AHardwareBuffer)
    /// This replaces capture_texture when using zero-copy import
    imported_texture: Option<wgpu::Texture>,
    /// Format of the imported texture (if any)
    imported_format: Option<wgpu::TextureFormat>,
    /// Retained Metal texture when using the Metal import path (keeps it alive for wgpu)
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    imported_metal_texture: Option<metal::Texture>,
    /// Capture texture format
    capture_format: wgpu::TextureFormat,
    /// The effect renderer wrapper
    effect_wrapper: ViewEffectRendererWrapper,
    /// Whether setup() has been called
    initialized: bool,
    /// Current input dimensions (from child view)
    input_width: u32,
    input_height: u32,
    /// Current output dimensions
    output_width: u32,
    output_height: u32,
    /// Output size configuration
    output_size: OutputSize,
}

/// Initialize a ViewEffect with native layers.
///
/// This function creates wgpu resources for the effect rendering pipeline.
///
/// # Arguments
///
/// * `effect` - Pointer to the WuiViewEffect FFI struct (consumed)
/// * `output_layer` - Platform-specific layer for effect output:
///   - Apple: `CAMetalLayer*`
///   - Android: `ANativeWindow*`
/// * `input_width` - Width of the captured view in pixels
/// * `input_height` - Height of the captured view in pixels
///
/// # Returns
///
/// Opaque pointer to the initialized state, or null on failure.
///
/// # Safety
///
/// - `effect` must be a valid pointer obtained from `waterui_force_as_view_effect`
/// - `output_layer` must be a valid platform-specific layer pointer
/// - The layer must remain valid for the lifetime of the returned state
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_init(
    effect: *mut WuiViewEffect,
    output_layer: *mut c_void,
    input_width: u32,
    input_height: u32,
) -> *mut WuiViewEffectState {
    let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if effect.is_null() || output_layer.is_null() || input_width == 0 || input_height == 0 {
            tracing::error!(
                "[ViewEffect] init failed: invalid parameters (effect={:?}, layer={:?}, {}x{})",
                effect,
                output_layer,
                input_width,
                input_height
            );
            return core::ptr::null_mut();
        }

        let wui_effect = unsafe { &mut *effect };

        // Recover the effect wrapper
        if wui_effect.effect.is_null() {
            tracing::error!("[ViewEffect] init failed: effect pointer is null");
            return core::ptr::null_mut();
        }
        let effect_wrapper: ViewEffectRendererWrapper =
            unsafe { *Box::from_raw(wui_effect.effect as *mut ViewEffectRendererWrapper) };

        // Null out to prevent double-free
        wui_effect.effect = core::ptr::null_mut();

        let output_size: OutputSize = wui_effect.output_size.into();
        let (output_width, output_height) = output_size.compute(input_width, input_height);

        // Initialize shared context if needed
        if !waterui_graphics::shared_context::is_initialized() {
            tracing::info!("[ViewEffect] Shared context not initialized, initializing now...");
            if let Err(e) = waterui_graphics::shared_context::init_shared_context() {
                tracing::error!("[ViewEffect] Init failed: {}", e);
                return core::ptr::null_mut();
            }
        }
        let ctx = shared_context();
        let guard = ctx.read();

        let instance = &guard.instance;
        let adapter = &guard.adapter;
        let device = guard.device.clone();
        let queue = guard.queue.clone();
        let pipeline_cache = guard.pipeline_cache.clone();

        // Create output surface
        let Some(output_surface) =
            super::gpu_surface::create_surface_from_layer(instance, output_layer)
        else {
            tracing::error!("[ViewEffect] Failed to create output surface from layer");
            return core::ptr::null_mut();
        };

        // Configure output surface
        let surface_caps = output_surface.get_capabilities(adapter);
        if surface_caps.formats.is_empty() {
            tracing::error!("[ViewEffect] Shared adapter cannot present to output surface!");
            return core::ptr::null_mut();
        }

        let preferred = waterui_graphics::gpu_surface::preferred_surface_format(&surface_caps);
        let format = if surface_caps.formats.contains(&preferred) {
            preferred
        } else {
            tracing::warn!(
                "[ViewEffect] Preferred format {:?} not supported, using {:?}",
                preferred,
                surface_caps.formats[0]
            );
            surface_caps.formats[0]
        };

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo
        } else {
            surface_caps.present_modes[0]
        };

        let alpha_mode = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
            wgpu::CompositeAlphaMode::Opaque,
        ]
        .into_iter()
        .find(|mode| surface_caps.alpha_modes.contains(mode))
        .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

        let output_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: output_width,
            height: output_height,
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        tracing::info!(
            "[ViewEffect] Configuring output: {}x{} {:?}",
            output_width,
            output_height,
            format
        );

        if !try_configure_surface(&output_surface, &device, &output_config) {
            tracing::error!("[ViewEffect] Output surface configuration failed!");
            return core::ptr::null_mut();
        }

        // Create capture texture (for capturing child view output)
        // Use the same format as output for simplicity
        let capture_format = format;
        let capture_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ViewEffect Capture Texture"),
            size: wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: capture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let state = Box::new(WuiViewEffectState {
            device,
            queue,
            pipeline_cache,
            output_surface,
            output_config,
            capture_texture: Some(capture_texture),
            imported_texture: None,
            imported_format: None,
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            imported_metal_texture: None,
            capture_format,
            effect_wrapper,
            initialized: false,
            input_width,
            input_height,
            output_width,
            output_height,
            output_size,
        });

        Box::into_raw(state)
    }));

    match init_result {
        Ok(ptr) => ptr,
        Err(_) => {
            tracing::error!("[ViewEffect] init panicked");
            core::ptr::null_mut()
        }
    }
}

/// Input texture type for ViewEffect.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WuiInputType {
    /// wgpu texture pointer (from GpuSurface child - zero copy optimization)
    WgpuTexture,
    /// MTLTexture handle (Apple - zero copy)
    /// The native side should create the MTLTexture from IOSurface
    MetalTexture,
    /// AHardwareBuffer handle (Android - zero copy)
    AHardwareBuffer,
    /// Raw pixel data (fallback with copy)
    PixelData,
}

/// Provide input texture from child view.
///
/// Call this each frame before `waterui_view_effect_render` to provide
/// the captured child view's texture.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state
/// * `input_type` - Type of input being provided
/// * `input_handle` - Platform-specific handle:
///   - `WgpuTexture`: Pointer to `wgpu::Texture`
///   - `IOSurface`: `IOSurfaceRef` (Apple)
///   - `AHardwareBuffer`: `AHardwareBuffer*` (Android)
///   - `PixelData`: Pointer to pixel data
/// * `width` - Input width in pixels
/// * `height` - Input height in pixels
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_view_effect_init`
/// - `input_handle` must be valid for the specified `input_type`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_set_input(
    state: *mut WuiViewEffectState,
    input_type: WuiInputType,
    input_handle: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    if state.is_null() || input_handle.is_null() || width == 0 || height == 0 {
        return false;
    }

    let state = unsafe { &mut *state };

    // Handle dimension changes
    if width != state.input_width || height != state.input_height {
        state.input_width = width;
        state.input_height = height;

        // Recalculate output dimensions
        let (output_width, output_height) = state.output_size.compute(width, height);
        if output_width != state.output_width || output_height != state.output_height {
            state.output_width = output_width;
            state.output_height = output_height;
            state.output_config.width = output_width;
            state.output_config.height = output_height;

            if !try_configure_surface(&state.output_surface, &state.device, &state.output_config) {
                tracing::warn!(
                    "[ViewEffect] output resize failed ({}x{})",
                    output_width,
                    output_height
                );
                return false;
            }
        }

        // Recreate capture texture if needed
        state.capture_texture = Some(state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ViewEffect Capture Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: state.capture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        }));
    }

    match input_type {
        WuiInputType::WgpuTexture => {
            // Direct texture reference from GpuSurface - most efficient path
            // The input_handle is a pointer to the wgpu::Texture
            // We'll use this directly in render() instead of the capture texture
            // For now, we just validate it's not null
            state.imported_texture = None;
            state.imported_format = None;
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                state.imported_metal_texture = None;
            }
            true
        }
        WuiInputType::MetalTexture => {
            // Import MTLTexture as wgpu texture (Apple zero-copy)
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                import_metal_texture(state, input_handle, width, height)
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            {
                tracing::error!("[ViewEffect] MetalTexture not supported on this platform");
                false
            }
        }
        WuiInputType::AHardwareBuffer => {
            // Import AHardwareBuffer as wgpu texture (Android zero-copy).
            // This path requires a native drop callback to keep the AHardwareBuffer alive for
            // the lifetime of the imported wgpu texture. Use the dedicated
            // `waterui_view_effect_set_input_ahardwarebuffer` entry point.
            #[cfg(target_os = "android")]
            {
                tracing::error!(
                    "[ViewEffect] AHardwareBuffer import requires a drop callback; use waterui_view_effect_set_input_ahardwarebuffer"
                );
                false
            }
            #[cfg(not(target_os = "android"))]
            {
                tracing::error!("[ViewEffect] AHardwareBuffer not supported on this platform");
                false
            }
        }
        WuiInputType::PixelData => {
            // Copy pixel data to capture texture (fallback path)
            // input_handle is a pointer to RGBA pixel data
            let Some(ref capture_texture) = state.capture_texture else {
                return false;
            };

            let bytes_per_row = width * 4; // Assuming RGBA8
            let data = unsafe {
                core::slice::from_raw_parts(
                    input_handle as *const u8,
                    (bytes_per_row * height) as usize,
                )
            };

            state.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: capture_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            state.imported_texture = None;
            state.imported_format = None;
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                state.imported_metal_texture = None;
            }
            true
        }
    }
}

/// Render the effect.
///
/// This function applies the effect to the captured input and renders to the output.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state
///
/// # Returns
///
/// `true` if rendering succeeded, `false` on error.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_view_effect_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_render(state: *mut WuiViewEffectState) -> bool {
    let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if state.is_null() {
            return false;
        }

        let state = unsafe { &mut *state };

        let input_format = if state.imported_texture.is_some() {
            state
                .imported_format
                .expect("[ViewEffect] imported texture is set but format is missing")
        } else {
            state.capture_format
        };

        // Call setup on first render
        if !state.initialized {
            let ctx = EffectContext {
                device: &state.device,
                queue: &state.queue,
                input_format,
                output_format: state.output_config.format,
                pipeline_cache: state.pipeline_cache.as_ref(),
            };
            let setup_future = state.effect_wrapper.erased.setup(&ctx);
            pollster::block_on(setup_future);
            state.initialized = true;
        }

        // Get output texture
        let output = match state.output_surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                if !try_configure_surface(
                    &state.output_surface,
                    &state.device,
                    &state.output_config,
                ) {
                    return false;
                }
                match state.output_surface.get_current_texture() {
                    Ok(o) => o,
                    Err(_) => return false,
                }
            }
            Err(wgpu::SurfaceError::Timeout) => return true, // Skip frame
            Err(e) => {
                tracing::error!("[ViewEffect] render failed: {e}");
                return false;
            }
        };

        // Get input texture - prefer imported texture (zero-copy) over capture texture
        let input_texture: &wgpu::Texture = if let Some(ref imported) = state.imported_texture {
            imported
        } else if let Some(ref capture) = state.capture_texture {
            capture
        } else {
            tracing::error!("[ViewEffect] no input texture available");
            return false;
        };

        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ViewEffect Input View"),
            ..Default::default()
        });

        let output_view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ViewEffect Output View"),
            format: Some(state.output_config.format),
            ..Default::default()
        });

        // Create input/output structs
        let input = EffectInput {
            device: &state.device,
            queue: &state.queue,
            texture: input_texture,
            view: input_view,
            format: input_format,
            width: state.input_width,
            height: state.input_height,
        };

        let effect_output = EffectOutput {
            device: &state.device,
            queue: &state.queue,
            texture: &output.texture,
            view: output_view,
            format: state.output_config.format,
            width: state.output_width,
            height: state.output_height,
        };

        // Call effect render
        state.effect_wrapper.erased.render(&input, &effect_output);

        // Present
        output.present();

        true
    }));

    match render_result {
        Ok(ok) => ok,
        Err(_) => {
            tracing::error!("[ViewEffect] render panicked");
            false
        }
    }
}

/// Get a pointer to the capture texture for the child view to render into.
///
/// The native backend should render the child view to this texture, then call
/// `waterui_view_effect_render` to apply the effect.
///
/// # Arguments
///
/// * `state` - Pointer to initialized state
///
/// # Returns
///
/// Pointer to the capture wgpu::Texture, or null if not available.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_view_effect_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_get_capture_texture(
    state: *mut WuiViewEffectState,
) -> *const c_void {
    if state.is_null() {
        return core::ptr::null();
    }

    let state = unsafe { &*state };

    match &state.capture_texture {
        Some(texture) => texture as *const wgpu::Texture as *const c_void,
        None => core::ptr::null(),
    }
}

/// Clean up ViewEffect resources.
///
/// # Safety
///
/// `state` must be a valid pointer from `waterui_view_effect_init`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_drop(state: *mut WuiViewEffectState) {
    if !state.is_null() {
        unsafe {
            let _ = Box::from_raw(state);
        }
    }
}

/// Check if the child view content is a GpuSurface.
///
/// Returns `true` if the child is a GpuSurface, enabling the zero-copy optimization
/// where we can directly sample the GpuSurface's texture.
///
/// # Safety
///
/// `effect` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_child_is_gpu_surface(
    effect: *const WuiViewEffect,
) -> bool {
    if effect.is_null() {
        return false;
    }

    let effect = unsafe { &*effect };

    if effect.content.is_null() {
        return false;
    }

    // Check if content's type ID matches GpuSurface
    let content_type_id = unsafe { crate::waterui_view_id(effect.content) };
    let gpu_surface_id = super::gpu_surface::waterui_gpu_surface_id();

    content_type_id == gpu_surface_id
}

fn try_configure_surface(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> bool {
    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Internal);
    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let configure_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        surface.configure(device, config);
    }))
    .is_err();

    let validation_err = pollster::block_on(device.pop_error_scope());
    let internal_err = pollster::block_on(device.pop_error_scope());
    let oom_err = pollster::block_on(device.pop_error_scope());

    if configure_panicked {
        tracing::warn!("[ViewEffect] Surface::configure panicked");
        return false;
    }

    if let Some(err) = validation_err.or(internal_err).or(oom_err) {
        tracing::warn!("[ViewEffect] Surface::configure failed: {err}");
        return false;
    }

    true
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
) -> bool {
    use metal::foreign_types::ForeignTypeRef;
    use wgpu_hal::Api;

    if mtl_texture_ptr.is_null() {
        tracing::error!("[ViewEffect] MTLTexture pointer is null");
        return false;
    }

    // Create a metal::Texture from the raw MTLTexture pointer
    // The native side passes us an MTLTexture (id<MTLTexture>)
    // which is a raw pointer in Objective-C
    let metal_texture_ref = unsafe { metal::TextureRef::from_ptr(mtl_texture_ptr.cast()) };
    let metal_texture = metal_texture_ref.to_owned();

    tracing::debug!(
        "[ViewEffect] Importing Metal texture: {}x{} {:?}",
        width,
        height,
        metal_texture.pixel_format()
    );

    // Create HAL texture from the Metal texture
    let hal_texture = unsafe {
        <MetalApi as Api>::Device::texture_from_raw(
            metal_texture.clone(),
            wgpu::TextureFormat::Rgba16Float,
            MTLTextureType::D2,
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
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    // Create wgpu texture from HAL texture
    let wgpu_texture = unsafe {
        state
            .device
            .create_texture_from_hal::<MetalApi>(hal_texture, &texture_desc)
    };

    // Store the imported texture
    state.imported_texture = Some(wgpu_texture);
    state.imported_metal_texture = Some(metal_texture);
    state.input_width = width;
    state.input_height = height;

    // Recalculate output dimensions
    let (output_width, output_height) = state.output_size.compute(width, height);
    if output_width != state.output_width || output_height != state.output_height {
        state.output_width = output_width;
        state.output_height = output_height;
        state.output_config.width = output_width;
        state.output_config.height = output_height;

        if !try_configure_surface(&state.output_surface, &state.device, &state.output_config) {
            tracing::warn!(
                "[ViewEffect] output resize failed ({}x{})",
                output_width,
                output_height
            );
            return false;
        }
    }

    true
}

/// Set input from an AHardwareBuffer (Android-specific zero-copy path).
///
/// This function is called from JNI with a HardwareBuffer object.
/// The JNI layer extracts the AHardwareBuffer pointer and passes it here.
///
/// # Arguments
///
/// * `state` - Pointer to initialized ViewEffect state
/// * `ahb_ptr` - Pointer to AHardwareBuffer (from AHardwareBuffer_fromHardwareBuffer)
/// * `width` - Width in pixels
/// * `height` - Height in pixels
///
/// # Safety
///
/// - `state` must be a valid pointer from `waterui_view_effect_init`
/// - `ahb_ptr` must be a valid AHardwareBuffer pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_effect_set_input_ahardwarebuffer(
    state: *mut WuiViewEffectState,
    ahb_ptr: *mut c_void,
    drop_fn: WuiExternalDropFn,
    drop_data: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    if state.is_null() || ahb_ptr.is_null() || drop_fn as usize == 0 || width == 0 || height == 0 {
        return false;
    }

    #[cfg(target_os = "android")]
    {
        let state = unsafe { &mut *state };
        match android_ahb::import_ahardwarebuffer_as_wgpu_texture(
            &state.device,
            ahb_ptr,
            width,
            height,
            "ViewEffect Imported AHardwareBuffer Texture",
            drop_fn,
            drop_data,
        ) {
            Ok((texture, format)) => {
                state.imported_texture = Some(texture);
                state.imported_format = Some(format);
                state.input_width = width;
                state.input_height = height;
                true
            }
            Err(e) => {
                tracing::error!("[ViewEffect] AHardwareBuffer import failed: {e}");
                false
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = state;
        let _ = drop_fn;
        let _ = drop_data;
        tracing::error!("[ViewEffect] AHardwareBuffer import only supported on Android");
        false
    }
}
