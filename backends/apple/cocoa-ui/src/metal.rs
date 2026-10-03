//! Metal and `IOSurface`, the presentation and capture plumbing of a GPU
//! surface.
//!
//! [`SurfaceBuffers`] is the double-buffered `IOSurface` pair a view presents
//! frames through: an `IOSurface` handed to `CALayer.contents` is composited
//! in place and — unlike a `CAMetalLayer` drawable, which only the pipeline
//! that presented it can read — every capture path (`cacheDisplay(in:to:)`,
//! `CARenderer`, `layer.render(in:)`) sees it.
//!
//! [`metal_to_wgpu_format`], [`wgpu_to_metal_format`], and
//! [`import_texture`] are the bridge a renderer living in wgpu uses to draw
//! into a Metal texture it does not own.
//!
//! # Safety
//!
//! `unsafe` here is the `IOSurface` Core Foundation constructor (its property
//! dictionary is built from the constants the header documents), Metal object
//! creation, and `wgpu-hal`'s raw-texture import, whose caller contract is
//! reproduced on each function's own `# Safety` section.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_foundation::{CFDictionary, CFRetained, CFType};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceExtendedLinearSRGB, kCGColorSpaceSRGB};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_io_surface::{
    IOSurfaceRef, kIOSurfaceAllocSize, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow,
    kIOSurfaceColorSpace, kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth,
};
use objc2_metal::{MTLDevice, MTLPixelFormat, MTLTexture, MTLTextureDescriptor, MTLTextureUsage};
use objc2_quartz_core::CALayer;

use crate::core_animation::without_animation;

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

/// The `IOSurface` pixel format and bytes per element matching a Metal
/// presentation format.
///
/// Only the formats the renderer presents in are mapped: the extended-range
/// half-float target and the 8-bit one everything else uses.
fn surface_format(format: MTLPixelFormat) -> (u32, usize) {
    match format {
        MTLPixelFormat::BGRA8Unorm | MTLPixelFormat::BGRA8Unorm_sRGB => (0x4247_5241, 4), // 'BGRA'
        MTLPixelFormat::RGBA16Float => (0x5247_6841, 8),                                  // 'RGhA'
        _ => panic!("cannot present Metal format {}", format.0),
    }
}

/// The colour space the compositor must read the surface in.
///
/// Core Animation has no other way to learn it: a plain `CALayer` carries no
/// colour space of its own, so an extended-range surface left unlabelled is
/// composited as if its values were display-referred sRGB and an HDR frame
/// comes out clipped and dark.
pub(crate) fn color_space(format: MTLPixelFormat) -> CFRetained<CGColorSpace> {
    let name = if format == MTLPixelFormat::RGBA16Float {
        // SAFETY: the colorspace statics are system constants.
        unsafe { kCGColorSpaceExtendedLinearSRGB }
    } else {
        // SAFETY: see above.
        unsafe { kCGColorSpaceSRGB }
    };
    CGColorSpace::with_name(Some(name)).expect("could not create the surface color space")
}

/// The colour space the compositor must read the surface in, serialized for
/// `IOSurfaceSetValue`.
///
/// `kIOSurfaceColorSpace` takes the serialized property list, unlike
/// `kCARendererColorSpace`, which takes the `CGColorSpace` itself.
pub(crate) fn surface_color_space(format: MTLPixelFormat) -> CFRetained<CFType> {
    color_space(format)
        .property_list()
        .expect("could not serialize the surface color space")
}

/// One `IOSurface` and the Metal texture that renders into it.
#[derive(Debug)]
struct Buffer {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
}

/// A buffer handed out to render into, and the pair it came from.
///
/// The generation is what ties a frame to the buffers that existed when it
/// started: a frame's completion can land after a resize replaced the pair,
/// and presenting "the current back buffer" at that point would show an
/// `IOSurface` nothing has ever drawn into.
#[derive(Debug)]
pub struct PendingFrame {
    /// The texture to render the frame into.
    pub texture: Retained<ProtocolObject<dyn MTLTexture>>,
    index: usize,
    generation: u64,
}

/// Presents rendered frames through `IOSurface`-backed textures on a plain
/// `CALayer`, instead of a `CAMetalLayer` swapchain.
///
/// Two buffers, never one: `contents` keeps whichever frame the compositor is
/// showing, so the next frame renders into the other one and swaps only after
/// its submission completes — a frame in flight is never on screen.
///
/// Not `Send`: `CALayer.contents` is a main-thread property.
#[derive(Debug)]
pub struct SurfaceBuffers {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    layer: Retained<CALayer>,
    buffers: Vec<(Buffer, CFRetained<IOSurfaceRef>)>,
    next_index: usize,
    generation: u64,
    has_presented_frame: bool,
    pixel_format: MTLPixelFormat,
    width: u32,
    height: u32,
}

impl SurfaceBuffers {
    /// A presenter that puts frames on `layer`, rendered by `device`.
    ///
    /// A plain `CALayer`: everything a `CAMetalLayer` was here for — the
    /// device, the drawable pool, the present — belongs to the surfaces
    /// instead.
    #[must_use]
    pub fn new(device: Retained<ProtocolObject<dyn MTLDevice>>, layer: Retained<CALayer>) -> Self {
        Self {
            device,
            layer,
            buffers: Vec::new(),
            next_index: 0,
            generation: 0,
            has_presented_frame: false,
            pixel_format: MTLPixelFormat::Invalid,
            width: 0,
            height: 0,
        }
    }

    /// Whether buffers exist for exactly this size and format.
    #[must_use]
    pub fn matches(&self, width: u32, height: u32, pixel_format: MTLPixelFormat) -> bool {
        !self.buffers.is_empty()
            && self.width == width
            && self.height == height
            && self.pixel_format == pixel_format
    }

    /// Makes the buffer pair for a size and format, replacing any earlier
    /// pair.
    ///
    /// Nothing is reused across a resize: an `IOSurface` is fixed at its
    /// creation size, so a new size means a new pair. Whatever the layer is
    /// showing stays on it, scaled by `contentsGravity`, until a frame at the
    /// new size is ready.
    ///
    /// # Panics
    ///
    /// On a zero-sized pair.
    pub fn configure(&mut self, width: u32, height: u32, pixel_format: MTLPixelFormat) {
        assert!(
            width > 0 && height > 0,
            "a presented surface must have a non-zero size"
        );
        if self.matches(width, height, pixel_format) {
            return;
        }
        self.buffers = (0..2)
            .map(|_| make_buffer(&self.device, width, height, pixel_format))
            .collect();
        self.width = width;
        self.height = height;
        self.pixel_format = pixel_format;
        self.next_index = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Drops the buffers and whatever the layer is showing.
    pub fn release(&mut self) {
        self.buffers.clear();
        self.width = 0;
        self.height = 0;
        self.pixel_format = MTLPixelFormat::Invalid;
        self.next_index = 0;
        self.generation = self.generation.wrapping_add(1);
        self.has_presented_frame = false;
        self.set_contents(None);
    }

    /// The buffer the next frame renders into: the one not being shown.
    #[must_use]
    pub fn next_frame(&self) -> Option<PendingFrame> {
        if self.buffers.is_empty() {
            return None;
        }
        Some(PendingFrame {
            texture: self.buffers[self.next_index].0.texture.clone(),
            index: self.next_index,
            generation: self.generation,
        })
    }

    /// Shows a frame once its submission says the GPU has finished writing
    /// it, and reports whether it reached the layer.
    ///
    /// Call this only from the completion of that frame's submission: showing
    /// a surface the GPU is still writing composites a half-drawn frame. A
    /// frame whose buffers have since been replaced is dropped rather than
    /// shown.
    pub fn present(&mut self, frame: &PendingFrame) -> bool {
        if frame.generation != self.generation || frame.index >= self.buffers.len() {
            return false;
        }
        self.set_contents(Some(&self.buffers[frame.index].1));
        self.has_presented_frame = true;
        self.next_index = (frame.index + 1) % self.buffers.len();
        true
    }

    /// The pixel format the buffers are configured at — the value the last
    /// `configure` established (`None` before the first configuration).
    #[must_use]
    pub const fn pixel_format(&self) -> MTLPixelFormat {
        self.pixel_format
    }

    /// Whether a rendered frame is on the layer right now.
    #[must_use]
    pub const fn has_presented_frame(&self) -> bool {
        self.has_presented_frame
    }

    fn set_contents(&self, contents: Option<&IOSurfaceRef>) {
        without_animation(|| {
            // SAFETY: `setContents:` takes any object; `IOSurfaceRef` is the
            // one Core Animation reads frames from.
            unsafe {
                let object = contents.map_or_else(std::ptr::null::<AnyObject>, |surface| {
                    core::ptr::from_ref(surface).cast::<AnyObject>()
                });
                let _: () = objc2::msg_send![&*self.layer, setContents: object];
            }
        });
    }
}

fn make_buffer(
    device: &ProtocolObject<dyn MTLDevice>,
    width: u32,
    height: u32,
    pixel_format: MTLPixelFormat,
) -> (Buffer, CFRetained<IOSurfaceRef>) {
    let surface = make_surface(width, height, pixel_format);
    // SAFETY: the class method creates a descriptor for exactly these values.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            pixel_format,
            width as usize,
            height as usize,
            false,
        )
    };
    // A frame both renders and, through the same surface, is composited —
    // and is sampled by any capture path that reads it back.
    descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
    let texture = device
        .newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, 0)
        .expect("could not make a Metal texture for the IOSurface");
    (Buffer { texture }, surface)
}

/// An `IOSurface` of `width` × `height` in `format`, labelled with the colour
/// space its pixels are in.
fn make_surface(width: u32, height: u32, format: MTLPixelFormat) -> CFRetained<IOSurfaceRef> {
    let (four_cc, bytes_per_element) = surface_format(format);
    // `IOSurface` wants its row bytes aligned to the device's own alignment;
    // an unaligned surface is either rejected or silently padded, and a
    // padded one read as tightly packed is sheared.
    let alignment = 16usize;
    let row_bytes = (width as usize * bytes_per_element).div_ceil(alignment) * alignment;
    let values = [
        NSNumber::new_u64(u64::from(width)),
        NSNumber::new_u64(u64::from(height)),
        NSNumber::new_u64(bytes_per_element as u64),
        NSNumber::new_u64(row_bytes as u64),
        NSNumber::new_u64((row_bytes * height as usize) as u64),
        NSNumber::new_u32(four_cc),
    ];
    // SAFETY: every `kIOSurface*` key is a system-constant `CFString` —
    // toll-free bridged to `NSString`, which `from_slices` requires.
    let keys: [&NSString; 6] = unsafe {
        [
            &*core::ptr::from_ref(kIOSurfaceWidth).cast::<NSString>(),
            &*core::ptr::from_ref(kIOSurfaceHeight).cast::<NSString>(),
            &*core::ptr::from_ref(kIOSurfaceBytesPerElement).cast::<NSString>(),
            &*core::ptr::from_ref(kIOSurfaceBytesPerRow).cast::<NSString>(),
            &*core::ptr::from_ref(kIOSurfaceAllocSize).cast::<NSString>(),
            &*core::ptr::from_ref(kIOSurfacePixelFormat).cast::<NSString>(),
        ]
    };
    let properties =
        NSDictionary::from_slices(&keys, &values.iter().map(|v| &**v).collect::<Vec<_>>());
    // SAFETY: the dictionary above is built from the keys and value types the
    // `IOSurface` header documents; the cast narrows its generic parameters
    // to `CFDictionary`'s `Opaque` defaults.
    let dictionary: &CFDictionary =
        unsafe { &*core::ptr::from_ref(&*properties).cast::<CFDictionary>() };
    // SAFETY: `properties` carries the keys and value types `IOSurfaceCreate`
    // documents.
    let surface = unsafe { IOSurfaceRef::new(dictionary) }.expect("could not create an IOSurface");
    // SAFETY: the colour space is the serialized property list
    // `kIOSurfaceColorSpace` requires.
    unsafe {
        surface.set_value(kIOSurfaceColorSpace, &surface_color_space(format));
    }
    surface
}
