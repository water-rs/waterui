//! The mesh gradient pass: bilinear-patch meshes painted by Metal into a
//! texture, or into an `IOSurface` a layer shows as its contents.
//!
//! A canvas realizes its Core Animation layers from the recording; a mesh
//! gradient is the one paint no layer property or Core Graphics call
//! expresses, so a small Metal pass paints it (`mesh_gradient.metal`) into
//! a layer that a shape layer masks. The pass is the Apple counterpart of
//! the Cherenkov engine's mesh paint and follows its contract
//! (`graphics/cherenkov/docs/mesh-paint.md`): premultiplied linear Display
//! P3 colours, the last patch owning an overlap, no padding beyond the
//! patches. The pixel's colour averages sub-pixel samples
//! ([`crate::patch`]), which antialiases the patches' boundary where the
//! engine leaves coverage to the shape alone.
//!
//! The layer shows a [`MeshSurface`] rather than being a `CAMetalLayer`:
//! an `IOSurface` set as `contents` is part of the layer tree, so every
//! rasterization of the tree — `renderInContext`, `cacheDisplayInRect`,
//! the capture and preview paths — draws the mesh, which a
//! `CAMetalLayer`'s drawable never reaches. On screen Core Animation
//! composites the surface without a copy, as it does a drawable.
//!
//! # Safety
//!
//! The `unsafe` here is Metal object creation and command encoding on live
//! objects. The `repr(C)` structs match the shader's `MeshPatch` and
//! `MeshUniforms` byte for byte, and each is copied into a command by
//! `setVertexBytes`/`setFragmentBytes` or into a buffer before the call
//! returns. An `IOSurface` is a valid `CALayer.contents` value.

use core::ffi::c_void;
use core::ptr::NonNull;
use std::sync::Mutex;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString};
use objc2_foundation::NSString;
use objc2_io_surface::{
    IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceColorSpace, kIOSurfaceHeight,
    kIOSurfacePixelFormat, kIOSurfaceWidth,
};
use objc2_metal::{
    MTLBuffer, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice,
    MTLLibrary, MTLLoadAction, MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder,
    MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState,
    MTLResourceOptions, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
    MTLViewport,
};
use objc2_quartz_core::CALayer;

use waterui_contract::contract_violation;

use crate::patch::patches;

/// The shader's metallib, precompiled at build time with
/// `-ftexture-write-rounding-mode=rte` like the capture compositor's.
const MESH_GRADIENT_METALLIB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/mesh_gradient.metallib"));

/// The format every mesh is painted in: half-float linear light, which
/// holds values above SDR white and outside the gamut.
pub const PIXEL_FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA16Float;

/// [`PIXEL_FORMAT`] as an `IOSurface` pixel format:
/// `kCVPixelFormatType_64RGBAHalf` (`'RGhA'`), four half floats per pixel.
const SURFACE_PIXEL_FORMAT: i64 = 0x5247_6841;

/// The bytes one [`PIXEL_FORMAT`] pixel takes.
const SURFACE_BYTES_PER_PIXEL: i64 = 8;

/// A mesh's pixels in a form a layer shows: an `IOSurface` and the Metal
/// texture over the same storage.
pub struct MeshSurface {
    surface: CFRetained<IOSurfaceRef>,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
}

impl core::fmt::Debug for MeshSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeshSurface")
            .field("surface", &self.surface)
            .field("width", &self.texture.width())
            .field("height", &self.texture.height())
            .finish()
    }
}

impl MeshSurface {
    /// The surface's size in pixels, `(width, height)`.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        (self.texture.width(), self.texture.height())
    }

    /// Makes the surface `layer`'s contents. Core Animation reads the
    /// surface whenever it draws the layer, so a surface is shown only
    /// once the paint into it has completed.
    pub fn show(&self, layer: &CALayer) {
        let surface: &IOSurfaceRef = &self.surface;
        // SAFETY: an IOSurface is a valid `CALayer.contents` value and
        // Core Animation retains it.
        unsafe {
            let contents: &AnyObject = &*core::ptr::from_ref(surface).cast::<AnyObject>();
            layer.setContents(Some(contents));
        }
    }
}

/// The pass's uniforms as the shader reads them.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Uniforms {
    rect: [f32; 4],
    linear: [f32; 4],
    offset: [f32; 2],
    count: u32,
    smooth: u32,
}

/// A mesh uploaded for painting.
pub struct Mesh {
    /// The patches' buffer; `None` for a mesh with no patches, which paints
    /// nothing.
    patches: Option<Retained<ProtocolObject<dyn MTLBuffer>>>,
    count: u32,
    smooth: bool,
}

impl core::fmt::Debug for Mesh {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mesh")
            .field("patches", &self.count)
            .field("smooth", &self.smooth)
            .finish()
    }
}

/// Where a mesh lands: the rectangle painted and the map from it into the
/// mesh's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The rectangle the target covers, in the space the placement maps
    /// from: `x`, `y`, `width`, `height`.
    pub rect: [f64; 4],
    /// The affine map from that space into the mesh's, as `kurbo`'s six
    /// coefficients.
    pub inverse: [f64; 6],
}

/// The Metal state shared by every mesh a canvas paints.
pub struct MeshPass {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
}

impl core::fmt::Debug for MeshPass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeshPass").finish_non_exhaustive()
    }
}

/// Why the mesh pass could not paint: the system or its Metal device
/// refused something a mesh needs. Operational, never a caller's mistake —
/// the caller logs it and the mesh is not drawn.
#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    /// The system has no Metal device.
    #[error("the system has no Metal device")]
    NoDevice,
    /// The device refused a command queue.
    #[error("the Metal device refused a command queue")]
    CommandQueue,
    /// The device refused to compile the mesh gradient pipeline.
    #[error("the Metal device refused the mesh gradient pipeline: {0}")]
    Pipeline(String),
    /// The queue refused a command buffer.
    #[error("the Metal queue refused a command buffer")]
    CommandBuffer,
    /// The command buffer refused a render encoder.
    #[error("the Metal command buffer refused a render encoder")]
    RenderEncoder,
    /// `IOSurface` refused the mesh surface.
    #[error("IOSurface refused a {width}x{height} half-float mesh surface")]
    Surface {
        /// The requested width in pixels.
        width: usize,
        /// The requested height in pixels.
        height: usize,
    },
    /// The device refused a texture over the mesh surface.
    #[error("the Metal device refused a texture over the {width}x{height} mesh surface")]
    SurfaceTexture {
        /// The surface's width in pixels.
        width: usize,
        /// The surface's height in pixels.
        height: usize,
    },
}

impl MeshPass {
    /// Opens the system's Metal device and builds the pass.
    ///
    /// # Errors
    ///
    /// [`MeshError::NoDevice`] when the system has no Metal device,
    /// [`MeshError::CommandQueue`] when the device refuses a queue, and
    /// [`MeshError::Pipeline`] when it refuses the pipeline.
    ///
    /// # Panics
    ///
    /// When the in-tree shader fails to load or lacks an entry point: the
    /// metallib is compiled into the binary by `build.rs`.
    pub fn new() -> Result<Self, MeshError> {
        let device = objc2_metal::MTLCreateSystemDefaultDevice().ok_or(MeshError::NoDevice)?;
        let queue = device.newCommandQueue().ok_or(MeshError::CommandQueue)?;
        let library = device
            .newLibraryWithData_error(&dispatch2::DispatchData::from_static_bytes(
                MESH_GRADIENT_METALLIB,
            ))
            .expect("the mesh gradient metallib is compiled by build.rs into the binary and loads");
        let vertex = library
            .newFunctionWithName(&NSString::from_str("mesh_gradient_vertex"))
            .expect("the mesh gradient metallib defines mesh_gradient_vertex");
        let fragment = library
            .newFunctionWithName(&NSString::from_str("mesh_gradient_fragment"))
            .expect("the mesh gradient metallib defines mesh_gradient_fragment");
        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(&vertex));
        descriptor.setFragmentFunction(Some(&fragment));
        // SAFETY: index 0 is the single color attachment.
        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setPixelFormat(PIXEL_FORMAT);
        // The pass stores: the mesh's own transparency is the answer.
        attachment.setBlendingEnabled(false);
        let pipeline = device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|error| MeshError::Pipeline(error.localizedDescription().to_string()))?;
        Ok(Self {
            device,
            queue,
            pipeline,
        })
    }

    /// The device meshes paint on, for a layer's own `device`.
    #[must_use]
    pub const fn device(&self) -> &Retained<ProtocolObject<dyn MTLDevice>> {
        &self.device
    }

    /// Uploads a mesh: `columns` by `rows` patches over a
    /// `(columns + 1) x (rows + 1)` vertex grid, row-major, with `points`
    /// in the mesh's space and `colors` as straight linear Display P3
    /// RGBA.
    ///
    /// Grids that do not match the dimensions, or a mesh with no patches,
    /// break the contract ([`patches`]): it is reported and the mesh paints
    /// nothing. So does a device that refuses the patch buffer, which is
    /// logged.
    ///
    /// # Panics
    ///
    /// Never in practice: the patch count of a buffer Metal just made fits
    /// `u32`.
    #[must_use]
    pub fn mesh(
        &self,
        columns: usize,
        rows: usize,
        points: &[[f64; 2]],
        colors: &[[f32; 4]],
        smooth: bool,
    ) -> Mesh {
        let mut patches = patches(columns, rows, points, colors);
        let buffer = NonNull::new(patches.as_mut_ptr().cast::<c_void>())
            .filter(|_| !patches.is_empty())
            .and_then(|bytes| {
                // SAFETY: `patches` is a live buffer of `repr(C)` patches;
                // Metal copies it into the new buffer.
                unsafe {
                    self.device.newBufferWithBytes_length_options(
                        bytes,
                        core::mem::size_of_val(patches.as_slice()),
                        MTLResourceOptions::StorageModeShared,
                    )
                }
            });
        if buffer.is_none() && !patches.is_empty() {
            tracing::error!(
                patches = patches.len(),
                "the device refused the mesh's patch buffer; the mesh paints nothing"
            );
        }
        let count = if buffer.is_some() {
            u32::try_from(patches.len())
                .expect("a mesh's patches fit the Metal buffer just made, far below 2^32")
        } else {
            0
        };
        Mesh {
            patches: buffer,
            count,
            smooth,
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the shader's uniforms are f32"
    )]
    fn uniforms(mesh: &Mesh, placement: Placement) -> Uniforms {
        let rect = placement.rect.map(|v| v as f32);
        let inverse = placement.inverse.map(|v| v as f32);
        Uniforms {
            rect,
            linear: [inverse[0], inverse[1], inverse[2], inverse[3]],
            offset: [inverse[4], inverse[5]],
            count: mesh.count,
            smooth: u32::from(mesh.smooth),
        }
    }

    /// Paints `mesh` over the whole of `target`, a `RGBA16Float` texture,
    /// into `command_buffer`, which the caller commits.
    fn encode(
        &self,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
        mesh: &Mesh,
        placement: Placement,
        target: &ProtocolObject<dyn MTLTexture>,
    ) -> Result<(), MeshError> {
        let descriptor = MTLRenderPassDescriptor::new();
        // SAFETY: index 0 is the single color attachment.
        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(target));
        attachment.setLoadAction(MTLLoadAction::Clear);
        attachment.setStoreAction(MTLStoreAction::Store);
        attachment.setClearColor(MTLClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 0.0,
        });
        let encoder = command_buffer
            .renderCommandEncoderWithDescriptor(&descriptor)
            .ok_or(MeshError::RenderEncoder)?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "texture extents are far below f64's integer range"
        )]
        encoder.setViewport(MTLViewport {
            originX: 0.0,
            originY: 0.0,
            width: target.width() as f64,
            height: target.height() as f64,
            znear: 0.0,
            zfar: 1.0,
        });
        encoder.setRenderPipelineState(&self.pipeline);
        let uniforms = Self::uniforms(mesh, placement);
        // SAFETY: `encoder` is a live render encoder; buffer 0 is the
        // uniforms both stages read and buffer 1 the patches, and
        // `set*Bytes` copy `uniforms` into the command.
        unsafe {
            let bytes = NonNull::from(&uniforms).cast::<c_void>();
            let size = core::mem::size_of::<Uniforms>();
            encoder.setVertexBytes_length_atIndex(bytes, size, 0);
            encoder.setFragmentBytes_length_atIndex(bytes, size, 0);
            encoder.setFragmentBuffer_offset_atIndex(mesh.patches.as_deref(), 0, 1);
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
        encoder.endEncoding();
        Ok(())
    }

    /// Paints `mesh` into `target` and returns once the GPU is done: a
    /// render a test or a one-off export reads back.
    ///
    /// # Errors
    ///
    /// [`MeshError::CommandBuffer`] when the queue cannot make a command
    /// buffer, [`MeshError::RenderEncoder`] when it cannot make an encoder.
    pub fn paint_to_texture(
        &self,
        mesh: &Mesh,
        placement: Placement,
        target: &ProtocolObject<dyn MTLTexture>,
    ) -> Result<(), MeshError> {
        let command_buffer = self.queue.commandBuffer().ok_or(MeshError::CommandBuffer)?;
        self.encode(&command_buffer, mesh, placement, target)?;
        command_buffer.commit();
        command_buffer.waitUntilCompleted();
        Ok(())
    }

    /// A `width` by `height` pixel surface for meshes, tagged extended
    /// linear Display P3 so Core Animation reads its half floats as the
    /// pass writes them.
    ///
    /// A zero `width` or `height` breaks the contract: it is reported and
    /// the side is one pixel.
    ///
    /// # Errors
    ///
    /// [`MeshError::Surface`] when `IOSurface` refuses the surface and
    /// [`MeshError::SurfaceTexture`] when the device refuses a texture over
    /// it.
    ///
    /// # Panics
    ///
    /// Never in practice: a surface side fits `i64`, and a named colour
    /// space has a property list.
    pub fn surface(&self, width: usize, height: usize) -> Result<MeshSurface, MeshError> {
        if width == 0 || height == 0 {
            contract_violation!(
                "a mesh surface has pixels, got {width}x{height}; a zero side is one pixel"
            );
        }
        let (width, height) = (width.max(1), height.max(1));
        let count = |value: usize| {
            CFNumber::new_i64(
                i64::try_from(value)
                    .expect("a surface side fits an i64: IOSurface limits it far lower"),
            )
        };
        // SAFETY: the statics are `CFString` constants IOSurface exports.
        let keys: [&CFString; 4] = unsafe {
            [
                kIOSurfaceWidth,
                kIOSurfaceHeight,
                kIOSurfaceBytesPerElement,
                kIOSurfacePixelFormat,
            ]
        };
        let values = [
            count(width),
            count(height),
            CFNumber::new_i64(SURFACE_BYTES_PER_PIXEL),
            CFNumber::new_i64(SURFACE_PIXEL_FORMAT),
        ];
        let values: [&CFNumber; 4] = values.each_ref().map(|value| &**value);
        let properties = CFDictionary::from_slices(&keys, &values);
        // SAFETY: every key is an IOSurface property whose value is a number.
        let surface = unsafe { IOSurfaceRef::new(properties.as_opaque()) }
            .ok_or(MeshError::Surface { width, height })?;
        let space = crate::metal::color_space(PIXEL_FORMAT)
            .property_list()
            .expect("a named colour space has a property list");
        // SAFETY: `kIOSurfaceColorSpace` takes a colour space property list.
        unsafe { surface.set_value(kIOSurfaceColorSpace, &space) };
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                PIXEL_FORMAT,
                width,
                height,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::RenderTarget);
        // The descriptor's format and size are the surface's, and plane 0
        // is its only plane.
        let texture = self
            .device
            .newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, 0)
            .ok_or(MeshError::SurfaceTexture { width, height })?;
        Ok(MeshSurface { surface, texture })
    }

    /// Paints `mesh` into `target` and commits; `done` runs on a Metal
    /// thread once the GPU has finished, after which `target` may be
    /// shown. The returned command buffer is the paint, for a capture
    /// that must wait for it.
    ///
    /// # Errors
    ///
    /// [`MeshError::CommandBuffer`] when the queue cannot make a command
    /// buffer, [`MeshError::RenderEncoder`] when it cannot make an encoder;
    /// nothing is committed and `done` never runs.
    ///
    /// # Panics
    ///
    /// Never in practice: the completion slot's lock is never held across a
    /// panic.
    pub fn paint_to_surface(
        &self,
        mesh: &Mesh,
        placement: Placement,
        target: &MeshSurface,
        done: impl FnOnce() + Send + 'static,
    ) -> Result<Retained<ProtocolObject<dyn MTLCommandBuffer>>, MeshError> {
        let command_buffer = self.queue.commandBuffer().ok_or(MeshError::CommandBuffer)?;
        self.encode(&command_buffer, mesh, placement, &target.texture)?;
        let done = Mutex::new(Some(done));
        let handler = RcBlock::new(move |_: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
            let done = done
                .lock()
                .expect("the completion slot is never locked across a panic")
                .take();
            if let Some(done) = done {
                done();
            }
        });
        // SAFETY: the block has the handler signature and is attached
        // before `commit`; Metal copies it.
        unsafe { command_buffer.addCompletedHandler(RcBlock::as_ptr(&handler)) };
        command_buffer.commit();
        Ok(command_buffer)
    }
}
