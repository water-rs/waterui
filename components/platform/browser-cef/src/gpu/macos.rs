//! CEF's macOS GPU path.
//!
//! Chromium's macOS shared image is an `IOSurface` valid for the duration
//! of `on_accelerated_paint` — Metal imports one directly with
//! `newTextureWithDescriptor:iosurface:plane:`, wrapped for wgpu by
//! [`interop::metal::import_texture`]. The transient import only samples,
//! so every write into an owned texture runs through the compositor.
//!
//! The pooled destination is an engine-owned `IOSurface` of its own,
//! wrapped as a fresh `MTLTexture` on every present — each engine-held
//! generation retains the surface once through its own texture, the way a
//! queued sample or a showing plane would, so
//! [`IOSurfaceRef::retain_count`] reports a released buffer the same way a
//! `CVPixelBufferPool` recycles one. A pooled surface is reused only when
//! its retain count falls back to the allocation's own.

use std::ptr::NonNull;

use cef::{AcceleratedPaintInfo, ColorType};
use num_traits::ToPrimitive as _;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetIOSurface,
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
    kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};
use waterui_graphics::cherenkov_gpu::interop::metal;
use waterui_graphics::cherenkov_gpu::interop::{ExternalFrame, FrameColor, RgbAlpha};
use waterui_graphics::gpu::{ExternalFrameView, FrameOutput};

use super::sink::{Backend, external_view};
use crate::CefPageHandle;

/// One paint's transient import: the `MTLTexture` created on CEF's
/// `IOSurface`, the normalized region the frame occupies in the padded
/// surface, and the visible extent it presents at.
struct MacImport {
    texture: wgpu::Texture,
    uv: [f32; 4],
    size: (u32, u32),
}

/// One pooled presentation target: an engine-owned `IOSurface`, the
/// retain count nothing outside the sink holds, and the `MTLTexture`
/// generation the last write left in it.
struct SurfaceTarget {
    surface: CFRetained<IOSurfaceRef>,
    /// The surface's retain count while only this target holds it: the
    /// `CFRetained` alone, the pixel buffer having dropped. A live
    /// generation's `MTLTexture` — the sink's own clone, an engine-held
    /// frame's, or a queued sample on a promoted plane — retains the
    /// surface once more, which is the hold a reuse must never write
    /// into.
    free: usize,
    frame: Option<wgpu::Texture>,
}

/// The macOS [`Backend`]: imports on the output's Metal device.
struct MacBackend {
    device: wgpu::Device,
    metal: Retained<ProtocolObject<dyn MTLDevice>>,
}

impl Backend for MacBackend {
    /// The pixel extent — every pooled surface is a `BGRA8` renderable
    /// texture, so the extent is the whole reuse key.
    type Key = (u32, u32);
    type Import = MacImport;
    type Target = SurfaceTarget;

    const COPIES: bool = false;

    fn open(output: &FrameOutput) -> Self {
        assert_eq!(
            output.shared_device().adapter.get_info().backend,
            wgpu::Backend::Metal,
            "CEF IOSurface import requires WaterUI's Metal backend"
        );
        // SAFETY: the backend asserts the device is Metal just above, so
        // `as_hal` borrows a live Metal device.
        let hal = unsafe { output.device().as_hal::<wgpu::hal::metal::Api>() }
            .expect("CEF IOSurface import requires a Metal device");
        Self {
            device: output.device().clone(),
            metal: hal.raw_device().clone(),
        }
    }

    /// # Panics
    ///
    /// Panics on a null `IOSurface`, a color type outside the ones
    /// Chromium's macOS shared images use, an extent that does not fit a
    /// `u32`, or Metal refusing the surface.
    fn import(&self, frame: &AcceleratedPaintInfo) -> Self::Import {
        let coded = &frame.extra.coded_size;
        let coded_width =
            u32::try_from(coded.width).expect("CEF IOSurface width must be positive");
        let coded_height =
            u32::try_from(coded.height).expect("CEF IOSurface height must be positive");
        let visible = &frame.extra.visible_rect;
        // Clamped to the allocation: a visible rect larger than the coded
        // size would be a CEF bug, and reading past the end of the surface
        // is not the way to find out.
        let visible_width = u32::try_from(visible.width)
            .unwrap_or(coded_width)
            .min(coded_width);
        let visible_height = u32::try_from(visible.height)
            .unwrap_or(coded_height)
            .min(coded_height);
        let format = if frame.format == ColorType::BGRA_8888 {
            wgpu::TextureFormat::Bgra8Unorm
        } else if frame.format == ColorType::RGBA_8888 {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            panic!("CEF returned unsupported macOS accelerated color format")
        };
        let raw = autoreleasepool(|_| {
            // SAFETY: the pixel format is a valid `MTLPixelFormat` and the
            // descriptor's extent is the surface's allocated one — an
            // `IOSurface`-backed texture must be created at the
            // allocation's extent; the composite crops to the visible
            // region.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    metal_format(format),
                    coded_width as usize,
                    coded_height as usize,
                    false,
                )
            };
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            let pointer = NonNull::new(frame.shared_texture_io_surface)
                .expect("CEF accelerated paint returned a null IOSurface");
            // SAFETY: `import` runs inside `on_accelerated_paint`, so the
            // pointer names a live `IOSurface` for exactly this call; the
            // texture the device creates on it retains the surface and is
            // dropped inside this callback.
            let surface = unsafe { pointer.cast::<IOSurfaceRef>().as_ref() };
            self.metal
                .newTextureWithDescriptor_iosurface_plane(&descriptor, surface, 0)
                .expect("Metal rejected the CEF IOSurface")
        });
        // SAFETY: `raw` is a live `MTLTexture` created on `self.metal` —
        // the `MTLDevice` behind `self.device` — `format` is its pixel
        // format, and nothing outlives the callback: the wrapper is
        // dropped at its end, after the composite that reads it was
        // submitted on the same queue the engine samples through.
        let texture = unsafe { metal::import_texture(&self.device, raw, format) };
        Self::Import {
            texture,
            uv: [
                0.0,
                0.0,
                (f64::from(visible_width) / f64::from(coded_width))
                    .to_f32()
                    .expect("CEF visible width exceeds f32"),
                (f64::from(visible_height) / f64::from(coded_height))
                    .to_f32()
                    .expect("CEF visible height exceeds f32"),
            ],
            size: (visible_width, visible_height),
        }
    }

    fn texture(import: &Self::Import) -> &wgpu::Texture {
        &import.texture
    }

    fn size(import: &Self::Import) -> (u32, u32) {
        import.size
    }

    fn source_uv(import: &Self::Import) -> [f32; 4] {
        import.uv
    }

    fn key(import: &Self::Import) -> Self::Key {
        import.size
    }

    fn view_key(key: Self::Key, _composited: bool) -> Self::Key {
        key
    }

    /// # Panics
    ///
    /// Panics when `CoreVideo` refuses the pixel buffer, when the extent
    /// does not fit `usize`, or when Metal refuses the surface.
    fn alloc(&self, (width, height): Self::Key) -> Self::Target {
        let empty = CFDictionary::<CFString, CFType>::from_slices(&[], &[]);
        // SAFETY: immutable framework keys and the CFBoolean singleton.
        let attributes = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(
                &[
                    kCVPixelBufferIOSurfacePropertiesKey,
                    kCVPixelBufferMetalCompatibilityKey,
                ],
                &[&empty, objc2_core_foundation::kCFBooleanTrue.expect("CFBoolean")],
            )
        };
        let mut raw = std::ptr::null_mut();
        // SAFETY: valid attributes, and an out pointer for the retained
        // buffer.
        let status = unsafe {
            CVPixelBufferCreate(
                None,
                width as usize,
                height as usize,
                kCVPixelFormatType_32BGRA,
                Some(attributes.as_opaque()),
                NonNull::from(&mut raw),
            )
        };
        assert_eq!(
            status, kCVReturnSuccess,
            "CEF pool pixel buffer allocation failed"
        );
        // SAFETY: success transfers one reference to the caller.
        let buffer = unsafe {
            CFRetained::<CVPixelBuffer>::from_raw(
                NonNull::new(raw).expect("successful pixel-buffer allocation"),
            )
        };
        let surface =
            CVPixelBufferGetIOSurface(Some(&buffer)).expect("IOSurface-backed pixel buffer");
        // The baseline reads once the pixel buffer's own hold is gone:
        // what stays is this target's `CFRetained` alone.
        drop(buffer);
        let free = surface.retain_count();
        SurfaceTarget {
            surface,
            free,
            frame: None,
        }
    }

    /// # Panics
    ///
    /// Panics when the surface's extent does not fit a `u32` or Metal
    /// refuses it.
    fn materialize<'a>(&self, target: &'a mut Self::Target) -> &'a wgpu::Texture {
        let width =
            u32::try_from(target.surface.width()).expect("pool surface width exceeds u32");
        let height =
            u32::try_from(target.surface.height()).expect("pool surface height exceeds u32");
        let raw = autoreleasepool(|_| {
            // SAFETY: the pixel format is a valid `MTLPixelFormat` and the
            // descriptor's extent is the surface's allocated one.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::BGRA8Unorm,
                    width as usize,
                    height as usize,
                    false,
                )
            };
            descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
            self.metal
                .newTextureWithDescriptor_iosurface_plane(&descriptor, &target.surface, 0)
                .expect("Metal rejected a pool IOSurface")
        });
        // SAFETY: `raw` is a live `MTLTexture` created on `self.metal`
        // above; the descriptor repeats its format, extent, mip level and
        // sample count exactly, and the target is fully rewritten before
        // anything reads it, so `UNINITIALIZED` is its actual state.
        let texture = unsafe {
            self.device.create_texture_from_hal::<wgpu::hal::metal::Api>(
                wgpu::hal::metal::Device::texture_from_raw(
                    raw,
                    wgpu::TextureFormat::Bgra8Unorm,
                    MTLTextureType::Type2D,
                    1,
                    1,
                    wgpu::hal::CopyExtent {
                        width,
                        height,
                        depth: 1,
                    },
                    None,
                ),
                &wgpu::TextureDescriptor {
                    label: Some("waterui_cef_pool_texture"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::wgt::TextureUses::UNINITIALIZED,
            )
        };
        // Replacing the slot drops the previous generation's clone; its
        // texture destruction is queued behind this write on the engine's
        // ordered queue.
        target.frame = Some(texture);
        target.frame.as_ref().expect("assigned just above")
    }

    fn current(target: &Self::Target) -> Option<&wgpu::Texture> {
        target.frame.as_ref()
    }

    fn released(target: &mut Self::Target) -> bool {
        // The sink's own clone only kept the generation alive for the
        // engine's lease; dropping it leaves the holds that count — an
        // engine-held frame's clone or a showing plane — so a surface
        // back at its baseline is free to write into.
        target.frame = None;
        target.surface.retain_count() <= target.free
    }

    fn frame(&self, target: &mut Self::Target) -> ExternalFrame {
        ExternalFrame::rgb(
            target
                .frame
                .as_ref()
                .expect("materialize ran first")
                .clone(),
            RgbAlpha::Premultiplied,
            FrameColor::SRGB,
        )
        .expect("a pool texture is a valid external frame")
    }
}

/// The `MTLPixelFormat` matching a wgpu 8-bit unorm format an `IOSurface`
/// carries.
fn metal_format(format: wgpu::TextureFormat) -> MTLPixelFormat {
    match format {
        wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        wgpu::TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        format => panic!("an IOSurface cannot carry the wgpu format {format:?}"),
    }
}

/// Creates the GPU view for one visible CEF page on macOS: an
/// [`ExternalFrameView`] whose source imports the page's shared
/// `IOSurface` frames on the layer's own device.
pub(super) fn gpu_view(page: CefPageHandle) -> ExternalFrameView {
    external_view::<MacBackend>(page)
}
