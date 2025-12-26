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

use alloc::boxed::Box;
use alloc::vec;

// Platform-specific imports for Metal HAL texture import
#[cfg(any(target_os = "macos", target_os = "ios"))]
use {
    metal::MTLTextureType,
    wgpu_hal::api::Metal as MetalApi,
};

// Platform-specific imports for Vulkan HAL texture import (Android)
#[cfg(target_os = "android")]
use {
    ash::vk,
    wgpu_hal::api::Vulkan as VulkanApi,
};

use waterui_graphics::shared_context::shared_context;
use waterui_graphics::view_effect::{EffectContext, EffectInput, EffectOutput, OutputSize, ViewEffectErased};

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
        let effect_wrapper = Box::new(ViewEffectRendererWrapper {
            erased: self,
        });
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
        let Some(output_surface) = super::gpu_surface::create_surface_from_layer(instance, output_layer) else {
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
            tracing::warn!("[ViewEffect] Preferred format {:?} not supported, using {:?}", preferred, surface_caps.formats[0]);
            surface_caps.formats[0]
        };

        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
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
            output_width, output_height, format
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
                tracing::warn!("[ViewEffect] output resize failed ({}x{})", output_width, output_height);
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
            // Import AHardwareBuffer as wgpu texture (Android zero-copy)
            #[cfg(target_os = "android")]
            {
                import_ahardwarebuffer(state, input_handle, width, height)
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

        // Call setup on first render
        if !state.initialized {
            let ctx = EffectContext {
                device: &state.device,
                queue: &state.queue,
                input_format: state.capture_format,
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
                if !try_configure_surface(&state.output_surface, &state.device, &state.output_config) {
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
            format: state.capture_format,
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
    use metal::foreign_types::ForeignType;
    use wgpu_hal::Api;

    if mtl_texture_ptr.is_null() {
        tracing::error!("[ViewEffect] MTLTexture pointer is null");
        return false;
    }

    // Create a metal::Texture from the raw MTLTexture pointer
    // The native side passes us an MTLTexture (id<MTLTexture>)
    // which is a raw pointer in Objective-C
    let metal_texture = unsafe {
        // Cast void pointer to the raw MTLTexture pointer type
        // MTLTexture is an opaque type, *mut MTLTexture is what from_ptr expects
        metal::Texture::from_ptr(mtl_texture_ptr.cast())
    };

    tracing::debug!(
        "[ViewEffect] Importing Metal texture: {}x{} {:?}",
        width, height, metal_texture.pixel_format()
    );

    // Create HAL texture from the Metal texture
    let hal_texture = unsafe {
        <MetalApi as Api>::Device::texture_from_raw(
            metal_texture,
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
        state.device.create_texture_from_hal::<MetalApi>(hal_texture, &texture_desc)
    };

    // Store the imported texture
    state.imported_texture = Some(wgpu_texture);
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
            tracing::warn!("[ViewEffect] output resize failed ({}x{})", output_width, output_height);
            return false;
        }
    }

    true
}

/// Import an AHardwareBuffer as a wgpu texture (Android zero-copy path).
///
/// This function creates a wgpu texture that directly references the AHardwareBuffer,
/// enabling zero-copy texture sharing between the Android view system and the GPU effect pipeline.
///
/// Uses the VK_ANDROID_external_memory_android_hardware_buffer extension to import
/// the hardware buffer as a Vulkan image, then wraps it as a wgpu texture.
///
/// # Arguments
///
/// * `state` - The ViewEffect state
/// * `ahb_ptr` - Pointer to an AHardwareBuffer
/// * `width` - Width in pixels
/// * `height` - Height in pixels
///
/// # Safety
///
/// The AHardwareBuffer must remain valid for the lifetime of the imported texture.
#[cfg(target_os = "android")]
fn import_ahardwarebuffer(
    state: &mut WuiViewEffectState,
    ahb_ptr: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    use wgpu_hal::Api;

    if ahb_ptr.is_null() {
        tracing::error!("[ViewEffect] AHardwareBuffer pointer is null");
        return false;
    }

    tracing::debug!(
        "[ViewEffect] Importing AHardwareBuffer: {}x{}",
        width, height
    );

    // Get the raw Vulkan device from wgpu
    // We need to access the HAL device to create a texture from external memory
    let hal_device_result = unsafe {
        state.device.as_hal::<VulkanApi, _, _>(|hal_device| {
            hal_device.map(|device| {
                import_ahb_as_vulkan_texture(device, ahb_ptr, width, height)
            })
        })
    };

    let Some(import_result) = hal_device_result else {
        tracing::error!("[ViewEffect] Failed to get Vulkan HAL device (not using Vulkan backend?)");
        return false;
    };

    let (hal_texture, _vk_image, _vk_memory) = match import_result {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("[ViewEffect] AHardwareBuffer import failed: {}", e);
            return false;
        }
    };

    // Create wgpu texture descriptor
    let texture_desc = wgpu::TextureDescriptor {
        label: Some("ViewEffect Imported AHardwareBuffer Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // AHardwareBuffer typically uses RGBA8 or RGBA16Float
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    // Create wgpu texture from HAL texture
    let wgpu_texture = unsafe {
        state.device.create_texture_from_hal::<VulkanApi>(hal_texture, &texture_desc)
    };

    // Store the imported texture
    state.imported_texture = Some(wgpu_texture);
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
            tracing::warn!("[ViewEffect] output resize failed ({}x{})", output_width, output_height);
            return false;
        }
    }

    true
}

/// Import an AHardwareBuffer as a Vulkan HAL texture.
///
/// This function uses the VK_ANDROID_external_memory_android_hardware_buffer extension
/// to create a VkImage backed by the AHardwareBuffer.
#[cfg(target_os = "android")]
fn import_ahb_as_vulkan_texture(
    hal_device: &wgpu_hal::vulkan::Device,
    ahb_ptr: *mut c_void,
    width: u32,
    height: u32,
) -> Result<(wgpu_hal::vulkan::Texture, vk::Image, vk::DeviceMemory), &'static str> {
    use wgpu_hal::Api;

    // Get the raw Vulkan device
    let raw_device = hal_device.raw_device();

    // Get the physical device for memory type queries
    let shared = hal_device.shared_instance();
    let physical_device = shared.physical_device();
    let instance = shared.raw_instance();

    // Create ExternalMemoryImageCreateInfo for AHardwareBuffer
    let mut external_memory_create_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);

    // Create the image with external memory support
    // Using RGBA8 format which is commonly supported by AHardwareBuffer
    let image_create_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_SRGB)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external_memory_create_info);

    // Create the VkImage
    let vk_image = unsafe {
        raw_device.create_image(&image_create_info, None)
            .map_err(|_| "Failed to create VkImage")?
    };

    // Get memory requirements for the image
    let mem_requirements = unsafe {
        raw_device.get_image_memory_requirements(vk_image)
    };

    // Get memory properties
    let memory_properties = unsafe {
        instance.get_physical_device_memory_properties(physical_device)
    };

    // Get AHardwareBuffer properties to determine the correct memory type
    let ahb_properties = unsafe {
        let mut properties = vk::AndroidHardwareBufferPropertiesANDROID::default();

        // Load the extension function
        let get_ahb_properties_fn = instance
            .get_device_proc_addr(
                raw_device.handle(),
                c"vkGetAndroidHardwareBufferPropertiesANDROID".as_ptr(),
            )
            .ok_or("Failed to load vkGetAndroidHardwareBufferPropertiesANDROID")?;

        type GetAhbPropertiesFn = unsafe extern "system" fn(
            vk::Device,
            *const c_void,
            *mut vk::AndroidHardwareBufferPropertiesANDROID,
        ) -> vk::Result;

        let get_ahb_properties: GetAhbPropertiesFn = std::mem::transmute(get_ahb_properties_fn);

        let result = get_ahb_properties(
            raw_device.handle(),
            ahb_ptr,
            &mut properties,
        );

        if result != vk::Result::SUCCESS {
            // Clean up the image we created
            raw_device.destroy_image(vk_image, None);
            return Err("vkGetAndroidHardwareBufferPropertiesANDROID failed");
        }

        properties
    };

    // Find memory type from AHB properties
    let ahb_memory_type_index = find_memory_type_index(
        &memory_properties,
        ahb_properties.memory_type_bits,
        vk::MemoryPropertyFlags::empty(), // AHB memory type is already suitable
    ).ok_or("No suitable memory type from AHardwareBuffer properties")?;

    // Allocate memory from AHardwareBuffer
    let mut dedicated_allocate_info = vk::MemoryDedicatedAllocateInfo::default()
        .image(vk_image);

    let mut import_info = vk::ImportAndroidHardwareBufferInfoANDROID::default()
        .buffer(ahb_ptr as *mut _);

    let allocate_info = vk::MemoryAllocateInfo::default()
        .allocation_size(ahb_properties.allocation_size)
        .memory_type_index(ahb_memory_type_index)
        .push_next(&mut dedicated_allocate_info)
        .push_next(&mut import_info);

    let vk_memory = unsafe {
        raw_device.allocate_memory(&allocate_info, None)
            .map_err(|_| {
                raw_device.destroy_image(vk_image, None);
                "Failed to allocate memory from AHardwareBuffer"
            })?
    };

    // Bind the memory to the image
    unsafe {
        raw_device.bind_image_memory(vk_image, vk_memory, 0)
            .map_err(|_| {
                raw_device.free_memory(vk_memory, None);
                raw_device.destroy_image(vk_image, None);
                "Failed to bind image memory"
            })?;
    }

    // Create the HAL texture wrapper
    let hal_texture = unsafe {
        <VulkanApi as Api>::Device::texture_from_raw(
            vk_image,
            &wgpu::TextureDescriptor {
                label: Some("AHardwareBuffer Imported Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            None, // Drop callback - memory is handled separately
        )
    };

    tracing::info!("[ViewEffect] Successfully imported AHardwareBuffer as Vulkan texture");

    Ok((hal_texture, vk_image, vk_memory))
}

/// Find a memory type index that satisfies the given requirements.
#[cfg(target_os = "android")]
fn find_memory_type_index(
    mem_properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required_flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    for i in 0..mem_properties.memory_type_count {
        let type_supported = (type_bits & (1 << i)) != 0;
        let properties_match = mem_properties.memory_types[i as usize]
            .property_flags
            .contains(required_flags);

        if type_supported && properties_match {
            return Some(i);
        }
    }
    None
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
    width: u32,
    height: u32,
) -> bool {
    if state.is_null() || ahb_ptr.is_null() || width == 0 || height == 0 {
        return false;
    }

    #[cfg(target_os = "android")]
    {
        let state = unsafe { &mut *state };
        import_ahardwarebuffer(state, ahb_ptr, width, height)
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = state;
        tracing::error!("[ViewEffect] AHardwareBuffer import only supported on Android");
        false
    }
}
