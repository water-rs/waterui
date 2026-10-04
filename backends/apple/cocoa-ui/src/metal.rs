//! Metal↔wgpu format and texture bridging for the GPU surfaces.
//!
//! [`metal_to_wgpu_format`], [`wgpu_to_metal_format`], [`color_space`] and
//! [`import_texture`] are the bridge a renderer living in wgpu uses to draw
//! into a Metal texture it does not own. Presentation lives in
//! [`crate::metal_presenter`]; the retired `IOSurface`/`CALayer.contents`
//! double buffer that used to sit here is gone with `CAMetalDisplayLink`
//! pacing.
//!
//! # Safety
//!
//! `unsafe` here is Metal object creation and `wgpu-hal`'s raw-texture
//! import, whose caller contract is reproduced on each function's own
//! `# Safety` section.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3, kCGColorSpaceSRGB};
use objc2_metal::{MTLPixelFormat, MTLTexture};

/// The wgpu format matching `format`, or `None` for a Metal format wgpu
/// presentation here has no use for.
///
/// Only the formats the presentation paths produce are mapped: the sRGB
/// pair an ordinary surface uses and the half-float linear target an
/// extended-range one does.
#[must_use]
pub const fn metal_to_wgpu_format(format: MTLPixelFormat) -> Option<wgpu::TextureFormat> {
    match format {
        MTLPixelFormat::BGRA8Unorm => Some(wgpu::TextureFormat::Bgra8Unorm),
        MTLPixelFormat::BGRA8Unorm_sRGB => Some(wgpu::TextureFormat::Bgra8UnormSrgb),
        MTLPixelFormat::RGBA16Float => Some(wgpu::TextureFormat::Rgba16Float),
        _ => None,
    }
}

/// The Metal format matching `format`.
///
/// # Panics
///
/// When `format` has no Metal equivalent a presentation uses, which is every
/// format but `Bgra8Unorm`, `Bgra8UnormSrgb` and `Rgba16Float`.
#[must_use]
pub fn wgpu_to_metal_format(format: wgpu::TextureFormat) -> MTLPixelFormat {
    match format {
        wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => MTLPixelFormat::BGRA8Unorm_sRGB,
        wgpu::TextureFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
        _ => panic!("{format:?} has no Metal equivalent a presentation uses"),
    }
}

/// Wraps a Metal texture a foreign pipeline renders into as a wgpu texture.
///
/// The Metal texture stays alive for the wgpu texture's lifetime — the caller
/// retains it before calling and drops that retain only after the wgpu
/// texture is gone.
///
/// # Safety
///
/// `device` must be a wgpu device created on the Metal backend whose HAL
/// device produced `texture` — in practice the one `texture.device` reports.
/// The HAL description `format`, `width` and `height` build must match the
/// real texture: pass them as read off it.
/// `initial_state` must describe the texture's actual state at handoff, not
/// the union of its permitted usages. All prior writes must have completed
/// before wgpu accesses it, and foreign access must be synchronized with wgpu.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the imported texture descriptor and explicit foreign handoff state form one unsafe boundary"
)]
pub unsafe fn import_texture(
    device: &wgpu::Device,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    usage: wgpu::TextureUsages,
    initial_state: wgpu::TextureUses,
    label: &str,
) -> wgpu::Texture {
    // SAFETY: `texture` is the retained Metal texture the caller handed in,
    // and `format`/`width`/`height` describe it.
    let hal_texture = unsafe {
        <wgpu_hal::api::Metal as wgpu_hal::Api>::Device::texture_from_raw(
            texture,
            format,
            objc2_metal::MTLTextureType::Type2D,
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
    let descriptor = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    };
    // SAFETY: the HAL texture above came from the same Metal device `device`
    // wraps, and the caller supplies its synchronized handoff state.
    unsafe {
        device.create_texture_from_hal::<wgpu_hal::api::Metal>(
            hal_texture,
            &descriptor,
            initial_state,
        )
    }
}

/// The colour space the compositor must read the surface in.
///
/// Core Animation has no other way to learn it: a plain `CALayer` carries no
/// colour space of its own, so an extended-range surface left unlabelled is
/// composited as if its values were display-referred sRGB and an HDR frame
/// comes out clipped and dark.
/// Half-float pixels use extended linear Display P3, matching the GPU
/// presenter's output and the filter working space; 8-bit pixels use sRGB.
///
/// # Panics
///
/// When `CGColorSpaceCreateWithName` rejects the system constant names —
/// it cannot on supported targets.
#[must_use]
pub(crate) fn color_space(format: MTLPixelFormat) -> CFRetained<CGColorSpace> {
    let name = if format == MTLPixelFormat::RGBA16Float {
        // SAFETY: the colorspace statics are system constants.
        unsafe { kCGColorSpaceExtendedLinearDisplayP3 }
    } else {
        // SAFETY: see above.
        unsafe { kCGColorSpaceSRGB }
    };
    CGColorSpace::with_name(Some(name)).expect("could not create the surface color space")
}
