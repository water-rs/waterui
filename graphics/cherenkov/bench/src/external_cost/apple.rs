//! `CVPixelBuffer` producer: a ring of `IOSurface`-backed buffers,
//! CPU-filled per frame — the Apple video-decode output model.

use super::{
    BenchError, ExternalFrame, FrameColor, RING, Ramps, SharedDevice, SlotDone, Spec, wgpu,
};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane,
    CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeightOfPlane, CVPixelBufferGetIOSurface,
    CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelBufferMetalCompatibilityKey, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange, kCVReturnSuccess,
};
use objc2_metal::{MTLDevice, MTLPixelFormat, MTLTextureDescriptor, MTLTextureUsage};

/// The producer ring.
pub struct Producer {
    buffers: Vec<CFRetained<CVPixelBuffer>>,
    /// The raw `MTLDevice` the engine's `wgpu::Device` wraps — plane
    /// textures are created on it.
    mtl: Retained<ProtocolObject<dyn MTLDevice>>,
    device: wgpu::Device,
    ramps: Ramps,
    spec: Spec,
    /// Armed after the submit that samples slot `f % RING`.
    done: SlotDone,
}

/// Unlocks a pixel buffer if the body panics before the explicit unlock.
struct PlaneLock<'a> {
    buffer: &'a CVPixelBuffer,
    flags: CVPixelBufferLockFlags,
    armed: bool,
}

impl Drop for PlaneLock<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY: paired with the successful lock in `with_planes`,
        // same flags, same live buffer. This runs only when the body
        // panics before the explicit unlock.
        unsafe { CVPixelBufferUnlockBaseAddress(self.buffer, self.flags) };
    }
}

/// A Metal-compatible, `IOSurface`-backed pixel buffer — the same
/// construction `gpu/tests/planes.rs::surface_buffer` uses.
fn surface_buffer(spec: &Spec) -> Result<CFRetained<CVPixelBuffer>, BenchError> {
    let format = if spec.bits == 8 {
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
    } else {
        kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange
    };
    let empty = CFDictionary::<CFString, CFType>::from_slices(&[], &[]);
    // SAFETY: CoreVideo's attribute keys and the boolean are immutable
    // statics.
    let attributes = unsafe {
        CFDictionary::<CFString, CFType>::from_slices(
            &[
                kCVPixelBufferIOSurfacePropertiesKey,
                kCVPixelBufferMetalCompatibilityKey,
            ],
            &[
                &empty,
                objc2_core_foundation::kCFBooleanTrue.expect("kCFBooleanTrue"),
            ],
        )
    };
    let mut out = std::ptr::null_mut();
    // SAFETY: `out` receives a +1 pixel buffer.
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            spec.width as usize,
            spec.height as usize,
            format,
            Some(attributes.as_opaque()),
            std::ptr::NonNull::from(&mut out),
        )
    };
    if status != kCVReturnSuccess || out.is_null() {
        return Err(BenchError::Engine(format!(
            "CVPixelBufferCreate failed (CVReturn {status})"
        )));
    }
    // SAFETY: the create call returned a +1 pixel buffer.
    Ok(unsafe { CFRetained::from_raw(std::ptr::NonNull::new_unchecked(out)) })
}

/// Locks `buffer`, runs `body`, unlocks, and checks both statuses.
///
/// `read_only` is [`CVPixelBufferLockFlags::ReadOnly`] — path `c` only
/// reads the planes out. `fill` takes the read-write lock.
fn with_planes<R>(
    buffer: &CVPixelBuffer,
    read_only: bool,
    body: impl FnOnce(&CVPixelBuffer) -> Result<R, BenchError>,
) -> Result<R, BenchError> {
    let flags = if read_only {
        CVPixelBufferLockFlags::ReadOnly
    } else {
        CVPixelBufferLockFlags(0)
    };
    // SAFETY: `buffer` is a live `CVPixelBuffer` this ring owns. The
    // unlock below is paired with this lock and uses the same flags.
    let status = unsafe { CVPixelBufferLockBaseAddress(buffer, flags) };
    if status != kCVReturnSuccess {
        return Err(BenchError::Engine(format!(
            "CVPixelBufferLockBaseAddress failed (CVReturn {status})"
        )));
    }
    let mut guard = PlaneLock {
        buffer,
        flags,
        armed: true,
    };
    let result = body(buffer);
    guard.armed = false;
    // SAFETY: paired with the lock above. The lock returned success
    // and `body` has released every pointer into the planes.
    let unlock = unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
    if unlock != kCVReturnSuccess {
        return Err(BenchError::Engine(format!(
            "CVPixelBufferUnlockBaseAddress failed (CVReturn {unlock})"
        )));
    }
    result
}

/// One locked plane: base, row stride, row count, and the tight byte
/// count the ramp occupies.
fn plane_row(
    buffer: &CVPixelBuffer,
    plane: usize,
    tight: usize,
) -> Result<(*mut u8, usize, usize), BenchError> {
    let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane).cast::<u8>();
    let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane);
    let rows = CVPixelBufferGetHeightOfPlane(buffer, plane);
    if base.is_null() {
        return Err(BenchError::Engine(format!(
            "external-cost: locked plane {plane} has no base address"
        )));
    }
    if stride < tight {
        return Err(BenchError::Engine(format!(
            "external-cost: plane {plane} row stride {stride} is shorter than {tight}"
        )));
    }
    Ok((base, stride, rows))
}

impl Producer {
    /// The ring on `shared`'s Metal device.
    pub fn new(spec: &Spec, shared: &SharedDevice) -> Result<Self, BenchError> {
        // SAFETY: the guard is dropped before the device.
        let mtl = unsafe { shared.device.as_hal::<wgpu::hal::metal::Api>() }
            .ok_or_else(|| BenchError::Gpu("external-cost: the engine device is not Metal".into()))?
            .raw_device()
            .clone();
        let buffers = (0..RING)
            .map(|_| surface_buffer(spec))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            buffers,
            mtl,
            device: shared.device.clone(),
            ramps: Ramps::new(spec),
            spec: *spec,
            done: SlotDone::new(),
        })
    }

    /// Writes frame `frame`'s gradient into buffer `frame % RING`,
    /// after the submit that last sampled that slot has completed.
    pub fn fill(&mut self, frame: u32) -> Result<(), BenchError> {
        self.done.wait(frame, &self.device)?;
        let buffer = &self.buffers[frame as usize % RING];
        let ramps = &self.ramps;
        let spec = self.spec;
        with_planes(buffer, false, |buffer| {
            for plane in 0..2 {
                let tight = if plane == 0 {
                    spec.width as usize * spec.code_bytes()
                } else {
                    spec.width.div_ceil(2) as usize * 2 * spec.code_bytes()
                };
                let (base, stride, rows) = plane_row(buffer, plane, tight)?;
                for row in 0..rows {
                    // SAFETY: the plane is locked, `row < rows`, and
                    // `stride >= tight`, so `row * stride` plus `tight`
                    // stays inside the plane.
                    let dst =
                        unsafe { std::slice::from_raw_parts_mut(base.add(row * stride), tight) };
                    let row = u32::try_from(row).expect("row index fits u32");
                    if plane == 0 {
                        ramps.luma_row(frame, row, dst);
                    } else {
                        ramps.chroma_row(frame, row, dst);
                    }
                }
            }
            Ok(())
        })
    }

    /// Copies the filled buffer's planes into `dst`. Path `c` calls
    /// this under a read-only lock; the GPU copy happens later, from
    /// the staging buffer, not from this `CVPixelBuffer`.
    pub fn copy_planes(
        &self,
        frame: u32,
        dst: &mut wgpu::BufferViewMut,
        luma_stride: usize,
        chroma_offset: usize,
        chroma_stride: usize,
    ) -> Result<(), BenchError> {
        let buffer = &self.buffers[frame as usize % RING];
        let spec = self.spec;
        with_planes(buffer, true, |buffer| {
            let luma_tight = spec.width as usize * spec.code_bytes();
            let chroma_tight = spec.width.div_ceil(2) as usize * 2 * spec.code_bytes();
            let (y_base, y_stride, y_rows) = plane_row(buffer, 0, luma_tight)?;
            let (c_base, c_stride, c_rows) = plane_row(buffer, 1, chroma_tight)?;
            let y_rows_u = u32::try_from(y_rows).expect("rows fit u32");
            let c_rows_u = u32::try_from(c_rows).expect("rows fit u32");
            if y_rows_u != spec.height || c_rows_u != spec.height.div_ceil(2) {
                return Err(BenchError::Engine(format!(
                    "external-cost: plane rows {y_rows}/{c_rows} do not match {}x{}",
                    spec.width, spec.height
                )));
            }
            let need = chroma_offset + c_rows * chroma_stride;
            if dst.len() < need {
                return Err(BenchError::Engine(format!(
                    "external-cost: staging buffer is {} bytes, chroma needs {need}",
                    dst.len()
                )));
            }
            for row in 0..y_rows {
                // SAFETY: locked plane 0, `row < y_rows`, stride checked
                // against `luma_tight`. The destination range is inside
                // `dst` because `chroma_offset` is the luma region.
                let src =
                    unsafe { std::slice::from_raw_parts(y_base.add(row * y_stride), luma_tight) };
                super::write_tight(dst, row * luma_stride, src);
            }
            for row in 0..c_rows {
                // SAFETY: locked plane 1, same bound as the luma loop.
                let src =
                    unsafe { std::slice::from_raw_parts(c_base.add(row * c_stride), chroma_tight) };
                super::write_tight(dst, chroma_offset + row * chroma_stride, src);
            }
            Ok(())
        })
    }

    /// Plane `plane` of `buffer`'s `IOSurface` as a texture on the
    /// engine's device — `gpu/tests/planes.rs::plane_texture`.
    fn plane_texture(&self, buffer: &CVPixelBuffer, plane: usize) -> wgpu::Texture {
        let (mtl_format, format) = match (self.spec.bits, plane) {
            (8, 0) => (MTLPixelFormat::R8Uint, wgpu::TextureFormat::R8Uint),
            (8, _) => (MTLPixelFormat::RG8Uint, wgpu::TextureFormat::Rg8Uint),
            (_, 0) => (MTLPixelFormat::R16Uint, wgpu::TextureFormat::R16Uint),
            (_, _) => (MTLPixelFormat::RG16Uint, wgpu::TextureFormat::Rg16Uint),
        };
        let surface = CVPixelBufferGetIOSurface(Some(buffer)).expect("an IOSurface-backed buffer");
        // SAFETY: the descriptor is fully specified.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                mtl_format,
                CVPixelBufferGetWidthOfPlane(buffer, plane),
                CVPixelBufferGetHeightOfPlane(buffer, plane),
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        let raw = self
            .mtl
            .newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, plane)
            .expect("an IOSurface plane texture");
        // SAFETY: `raw` is an IOSurface-plane texture created on
        // `self.mtl`, the `MTLDevice` the engine's `wgpu::Device`
        // wraps, and `format` matches that plane's pixel format.
        // `import_texture` requires the texture to stay alive and
        // unwritten-except-by-the-producer for as long as a frame
        // referencing it can be in flight: the ring retains the
        // `CVPixelBuffer`, and `fill` waits until the submit that last
        // sampled slot `f % RING` has completed before writing it.
        unsafe { cherenkov_gpu::interop::metal::import_texture(&self.device, raw, format) }
    }

    /// Path `e`: the filled buffer's planes wrapped as an
    /// [`ExternalFrame`]. Apple always binds integer planes.
    pub fn external(
        &self,
        frame: u32,
        color: FrameColor,
    ) -> Result<(ExternalFrame, &'static str), BenchError> {
        let buffer = &self.buffers[frame as usize % RING];
        let frame = ExternalFrame::yuv(
            self.plane_texture(buffer, 0),
            self.plane_texture(buffer, 1),
            color,
        )
        .map_err(|e| BenchError::Engine(format!("external-cost: invalid frame: {e:?}")))?;
        Ok((frame, "planes"))
    }

    /// Arms the completion wait for the submit that just sampled `frame`.
    pub fn retire(&mut self, frame: u32, queue: &wgpu::Queue) {
        self.done.arm(frame, queue);
    }
}
