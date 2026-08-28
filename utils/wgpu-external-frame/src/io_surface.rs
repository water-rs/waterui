//! macOS `IOSurface` import.
//!
//! An `IOSurface` is the system's cross-process handle to GPU-shareable pixel
//! memory. Metal imports one directly with
//! `newTextureWithDescriptor:iosurface:plane:`, and `wgpu` adopts the resulting
//! `MTLTexture` through its `hal` layer, so no pixel is copied.
//!
//! Producers typically hand the surface over inside a callback and reclaim it
//! the moment that callback returns, so [`IoSurfaceFrame::retain`] takes a
//! reference on it first; the frame owns that reference until it is dropped.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLDevice as _, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureType,
    MTLTextureUsage,
};

/// A retained `IOSurface` and the extent it was allocated at.
pub struct IoSurfaceFrame {
    surface: CFRetained<IOSurfaceRef>,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

impl core::fmt::Debug for IoSurfaceFrame {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("IoSurfaceFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl IoSurfaceFrame {
    /// Takes a reference on `surface`, so that it stays valid past the callback
    /// that handed it over.
    ///
    /// `width` and `height` are the surface's allocated extent, which is what
    /// it must be imported as; a producer that padded its allocation should
    /// narrow the *copy* it makes out of the imported texture rather than
    /// claiming a smaller surface here.
    ///
    /// # Safety
    ///
    /// `surface` must point at a live `IOSurface` for the duration of this
    /// call, and `width`/`height`/`format` must describe it.
    #[must_use]
    pub unsafe fn retain(
        surface: NonNull<c_void>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        // SAFETY: the caller contract makes `surface` a live `IOSurface`;
        // retaining it here is what keeps it valid afterwards.
        let surface = unsafe { CFRetained::retain(surface.cast::<IOSurfaceRef>()) };
        Self {
            surface,
            width,
            height,
            format,
        }
    }

    /// The surface's allocated pixel width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The surface's allocated pixel height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The `wgpu` format the surface imports as.
    #[must_use]
    pub const fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Imports the surface as a `COPY_SRC` texture on `device`.
    ///
    /// The texture aliases the surface's memory rather than owning a copy of
    /// it, so the producer may overwrite it as soon as it takes the surface
    /// back; copy out of the returned texture before that happens.
    ///
    /// # Panics
    ///
    /// Panics unless `device` is a Metal device, and unless the frame's format
    /// is `Bgra8Unorm` or `Rgba8Unorm` — the two an `IOSurface` can carry here.
    #[must_use]
    pub fn import(&self, device: &wgpu::Device) -> wgpu::Texture {
        let descriptor = wgpu::TextureDescriptor {
            label: Some("wgpu_external_frame_io_surface"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let metal_format = match self.format {
            wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
            wgpu::TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
            format => panic!("an IOSurface cannot carry the wgpu format {format:?}"),
        };
        let hal_texture = objc2::rc::autoreleasepool(|_| {
            // SAFETY: the handle is only borrowed to read the raw `MTLDevice`, and it is
            // not kept past this closure.
            let hal_device = unsafe {
                device
                    .as_hal::<wgpu::hal::api::Metal>()
                    .expect("IOSurface import requires a Metal device")
            };
            let raw_device = hal_device.raw_device();
            let metal_descriptor = MTLTextureDescriptor::new();
            // SAFETY: plain setters on a descriptor this scope just allocated and owns.
            unsafe {
                metal_descriptor
                    .setWidth(usize::try_from(self.width).expect("IOSurface width exceeds usize"));
                metal_descriptor.setHeight(
                    usize::try_from(self.height).expect("IOSurface height exceeds usize"),
                );
            }
            metal_descriptor.setTextureType(MTLTextureType::Type2D);
            metal_descriptor.setPixelFormat(metal_format);
            metal_descriptor.setUsage(MTLTextureUsage::ShaderRead);
            metal_descriptor.setStorageMode(if raw_device.hasUnifiedMemory() {
                MTLStorageMode::Shared
            } else {
                MTLStorageMode::Managed
            });
            let texture = raw_device
                .newTextureWithDescriptor_iosurface_plane(&metal_descriptor, &self.surface, 0)
                .expect("Metal rejected the IOSurface import");
            // SAFETY: the texture was created from `metal_descriptor` immediately above,
            // so the format, type, mip and layer counts repeated here match it, and
            // ownership of the `MTLTexture` transfers to the returned hal texture.
            unsafe {
                <wgpu::hal::api::Metal as wgpu::hal::Api>::Device::texture_from_raw(
                    texture,
                    descriptor.format,
                    MTLTextureType::Type2D,
                    1,
                    1,
                    wgpu::hal::CopyExtent {
                        width: self.width,
                        height: self.height,
                        depth: 1,
                    },
                )
            }
        });
        // SAFETY: `hal_texture` was built for this device with `descriptor`'s format and
        // size, and is moved into the wgpu texture that now owns it.
        unsafe { device.create_texture_from_hal::<wgpu::hal::api::Metal>(hal_texture, &descriptor) }
    }
}
