//! The pixel layouts a captured view subtree crosses the FFI in.
//!
//! `AppliedFilter` and `ViewEffect` are handed their input by the platform:
//! Apple renders the subtree straight into the wgpu capture texture through an
//! `MTLTexture`, while Android renders it into an `AHardwareBuffer` that
//! `super::hardware_buffer` imports and copies. Only Android has to be told
//! which layout to allocate, but the vocabulary it is told in is ordinary wgpu
//! reasoning and lives here, so the C header declares the same signatures on
//! every platform.

/// The pixel layout an Android capture buffer must be allocated with.
///
/// The backend allocates its `ImageReader` from this after attaching, so the
/// buffer it hands back is copy-compatible with the capture texture the filter
/// samples. The discriminants are the values the JNI binding returns as an
/// `Int`, and are part of the Kotlin contract.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WuiCaptureFormat {
    /// Four 8-bit unsigned channels: `AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM`,
    /// which is `android.graphics.PixelFormat.RGBA_8888`.
    Rgba8Unorm = 0,
    /// Four 16-bit half-float channels:
    /// `AHARDWAREBUFFER_FORMAT_R16G16B16A16_FLOAT`, which is
    /// `android.graphics.PixelFormat.RGBA_FP16`.
    Rgba16Float = 1,
}

/// The buffer layout a capture texture of `format` must be filled from.
///
/// The capture texture's format is chosen from the output surface's
/// capabilities, so it is sRGB-encoded whenever the surface offers that. An
/// `AHardwareBuffer` has no sRGB variant — `HardwareRenderer` writes
/// sRGB-encoded bytes into a plain `RGBA_8888` buffer, exactly as Apple's
/// capture writes them into a `BGRA8Unorm_sRGB` texture — so the two differ only
/// in whether the sampler decodes, which is what leaves them copy-compatible.
///
/// # Panics
///
/// Panics when the capture texture's format has no `AHardwareBuffer` layout at
/// all. A BGRA surface format is the realistic case: Android's `ImageReader`
/// cannot allocate a BGRA buffer, so there is nothing to capture into.
#[must_use]
pub fn capture_buffer_format(format: wgpu::TextureFormat) -> WuiCaptureFormat {
    match format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => {
            WuiCaptureFormat::Rgba8Unorm
        }
        wgpu::TextureFormat::Rgba16Float => WuiCaptureFormat::Rgba16Float,
        other => panic!(
            "Android view capture cannot fill a {other:?} texture: no AHardwareBuffer layout \
             matches it"
        ),
    }
}

/// The wgpu format a `ViewEffect` samples an imported buffer of `format` as.
///
/// `HardwareRenderer` writes sRGB-encoded pixels, so the 8-bit layout is read
/// back through an sRGB texture and decoded by the sampler — the same thing the
/// Apple path gets from its `BGRA8Unorm_sRGB` capture texture. Half-float
/// buffers already hold linear values and are read as they are.
#[must_use]
pub const fn effect_input_texture_format(format: WuiCaptureFormat) -> wgpu::TextureFormat {
    match format {
        WuiCaptureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8UnormSrgb,
        WuiCaptureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
    }
}

/// Creates the texture a `ViewEffect` copies its captured input into.
///
/// Cleared once through wgpu for the same reason the `AppliedFilter` capture
/// texture is: wgpu zero-clears a texture it has never written the first time it
/// is sampled, which would wipe the first captured frame.
#[must_use]
pub fn create_effect_input_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ViewEffect Capture Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ViewEffect Capture Texture Init"),
    });
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("ViewEffect Capture Texture Init"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    queue.submit([encoder.finish()]);
    texture
}
