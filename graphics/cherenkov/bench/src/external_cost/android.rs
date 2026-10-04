//! `AHardwareBuffer` producer: a ring of GPU-sampleable AHBs, CPU-filled
//! per frame — the Android video-decode output model, the same buffer
//! kind `gpu/tests/vulkan_external_android.rs` imports.

use super::{
    BenchError, ExternalFrame, FrameColor, RING, Ramps, SharedDevice, SlotDone, Spec, wgpu,
};

use cherenkov_gpu::interop::{HdrMetadata, RgbAlpha, vulkan};

/// The producer ring.
pub struct Producer {
    buffers: Vec<std::ptr::NonNull<ndk_sys::AHardwareBuffer>>,
    /// The engine's native Vulkan device — per-frame AHB import.
    vulkan: vulkan::Device,
    device: wgpu::Device,
    ramps: Ramps,
    spec: Spec,
    /// Armed after the submit that samples slot `f % RING`.
    done: SlotDone,
}

/// Unlocks an `AHardwareBuffer` if the body panics before the explicit
/// unlock.
struct AhbLock {
    buffer: *mut ndk_sys::AHardwareBuffer,
    armed: bool,
}

impl Drop for AhbLock {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY: `buffer` is the AHB `with_planes` locked. This is the
        // matching unlock, and it runs only when the body panics before
        // the explicit unlock. The fence out-pointer is unused.
        unsafe { ndk_sys::AHardwareBuffer_unlock(self.buffer, std::ptr::null_mut()) };
    }
}

/// One allocated `AHardwareBuffer` of `spec`'s format — the usage set
/// `gpu/tests/vulkan_external_android.rs::alloc_ahb` asks for, plus CPU
/// read for the path-`c` copy.
fn alloc_ahb(spec: &Spec) -> Result<std::ptr::NonNull<ndk_sys::AHardwareBuffer>, BenchError> {
    let usage = ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE.0
        | ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0
        | ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN.0;
    let format = if spec.bits == 8 {
        ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_Y8Cb8Cr8_420.0
    } else {
        ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_YCbCr_P010.0
    };
    let desc = ndk_sys::AHardwareBuffer_Desc {
        width: spec.width,
        height: spec.height,
        layers: 1,
        format,
        usage,
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    // SAFETY: `desc` is fully initialized and outlives the call.
    if unsafe { ndk_sys::AHardwareBuffer_isSupported(&raw const desc) } == 0 {
        return Err(BenchError::Engine(format!(
            "external-cost: AHB format {format:#x} unsupported at {}x{}",
            spec.width, spec.height
        )));
    }
    let mut buffer = std::ptr::null_mut();
    // SAFETY: `buffer` receives the allocated buffer on success, and
    // `desc` is fully initialized.
    if unsafe { ndk_sys::AHardwareBuffer_allocate(&raw const desc, &raw mut buffer) } != 0
        || buffer.is_null()
    {
        return Err(BenchError::Engine(
            "external-cost: AHardwareBuffer_allocate failed".into(),
        ));
    }
    Ok(std::ptr::NonNull::new(buffer).expect("non-null after check"))
}

/// Locks `buffer`'s planes, runs `body`, unlocks, and checks both results.
fn with_planes<R>(
    buffer: *mut ndk_sys::AHardwareBuffer,
    write: bool,
    body: impl FnOnce(&ndk_sys::AHardwareBuffer_Planes) -> Result<R, BenchError>,
) -> Result<R, BenchError> {
    let usage = if write {
        ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0
    } else {
        ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_READ_OFTEN.0
    };
    let mut planes = ndk_sys::AHardwareBuffer_Planes {
        planeCount: 0,
        planes: [ndk_sys::AHardwareBuffer_Plane {
            data: std::ptr::null_mut(),
            pixelStride: 0,
            rowStride: 0,
        }; 4],
    };
    // SAFETY: `planes` is fully initialized and `buffer` is a live AHB
    // this ring allocated. The unlock below pairs with this lock.
    let rc = unsafe {
        ndk_sys::AHardwareBuffer_lockPlanes(
            buffer,
            u64::from(usage),
            -1,
            std::ptr::null_mut(),
            &raw mut planes,
        )
    };
    if rc != 0 {
        return Err(BenchError::Engine(format!(
            "external-cost: AHardwareBuffer_lockPlanes failed ({rc})"
        )));
    }
    let mut guard = AhbLock {
        buffer,
        armed: true,
    };
    let result = body(&planes);
    guard.armed = false;
    // SAFETY: paired with the successful lock above. `body` has copied
    // or written the plane bytes and no longer holds their pointers.
    let unlock = unsafe { ndk_sys::AHardwareBuffer_unlock(buffer, std::ptr::null_mut()) };
    if unlock != 0 {
        return Err(BenchError::Engine(format!(
            "external-cost: AHardwareBuffer_unlock failed ({unlock})"
        )));
    }
    result
}

/// # Safety
/// `plane.data` is a live locked plane, `row * rowStride + len` stays
/// inside it, and `len <= rowStride`.
unsafe fn plane_row<'a>(
    plane: &ndk_sys::AHardwareBuffer_Plane,
    row: usize,
    len: usize,
) -> &'a [u8] {
    // SAFETY: the caller holds the lock and promised `row * rowStride + len`
    // is inside the plane.
    unsafe {
        std::slice::from_raw_parts(
            plane.data.cast::<u8>().add(row * plane.rowStride as usize),
            len,
        )
    }
}

/// # Safety
/// Same as [`plane_row`], and the caller is the only writer of that row.
unsafe fn plane_row_mut<'a>(
    plane: &ndk_sys::AHardwareBuffer_Plane,
    row: usize,
    len: usize,
) -> &'a mut [u8] {
    // SAFETY: the caller holds the write lock and promised the range
    // stays inside the plane.
    unsafe {
        std::slice::from_raw_parts_mut(
            plane.data.cast::<u8>().add(row * plane.rowStride as usize),
            len,
        )
    }
}

/// Cb-first interleaved chroma: plane 2's base is one code past plane 1's,
/// and plane 1's pixel stride is one `(cb, cr)` pair.
fn require_cb_first(
    planes: &ndk_sys::AHardwareBuffer_Planes,
    spec: &Spec,
) -> Result<(), BenchError> {
    let bytes = spec.code_bytes();
    let pair = 2 * bytes;
    if planes.planeCount < 3 {
        return Err(BenchError::Engine(format!(
            "external-cost: AHB has {} planes; chroma must be Cb-first interleaved",
            planes.planeCount
        )));
    }
    let luma = &planes.planes[0];
    let p1 = &planes.planes[1];
    let p2 = &planes.planes[2];
    let luma_tight = spec.width as usize * bytes;
    let chroma_tight = spec.width.div_ceil(2) as usize * pair;
    let cb_first = p1.pixelStride as usize == pair
        && !p1.data.is_null()
        && p2.data.cast::<u8>() == p1.data.cast::<u8>().wrapping_add(bytes);
    if !cb_first {
        return Err(BenchError::Engine(format!(
            "external-cost: chroma is not Cb-first interleaved (planes {}, pixelStride {})",
            planes.planeCount, p1.pixelStride
        )));
    }
    if luma.data.is_null() || (luma.rowStride as usize) < luma_tight {
        return Err(BenchError::Engine(format!(
            "external-cost: luma rowStride {} is shorter than {luma_tight}",
            luma.rowStride
        )));
    }
    if (p1.rowStride as usize) < chroma_tight {
        return Err(BenchError::Engine(format!(
            "external-cost: chroma rowStride {} is shorter than {chroma_tight}",
            p1.rowStride
        )));
    }
    Ok(())
}

/// How an imported frame was bound. Path `e` records this; an
/// external-format import is the driver's YCbCr sampler, not
/// `ext_frame_yuv`.
fn import_form(frame: &vulkan::Frame) -> &'static str {
    match frame.repr() {
        vulkan::Repr::ExternalFormat { .. } => "external-format",
        vulkan::Repr::Planes { .. } => "planes",
        vulkan::Repr::Rgb { .. } => "rgb",
    }
}

impl Producer {
    /// The ring over `shared`'s Vulkan device.
    pub fn new(spec: &Spec, shared: &SharedDevice) -> Result<Self, BenchError> {
        let vulkan = vulkan::Device::new(shared)
            .map_err(|e| BenchError::Gpu(format!("external-cost vulkan device: {e}")))?;
        let buffers = (0..RING)
            .map(|_| alloc_ahb(spec))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            buffers,
            vulkan,
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
        let buffer = self.buffers[frame as usize % RING].as_ptr();
        let (ramps, spec) = (&self.ramps, &self.spec);
        with_planes(buffer, true, |planes| {
            require_cb_first(planes, spec)?;
            let luma = &planes.planes[0];
            let chroma = &planes.planes[1];
            let luma_tight = spec.width as usize * spec.code_bytes();
            let chroma_tight = spec.width.div_ceil(2) as usize * 2 * spec.code_bytes();
            for row in 0..spec.height as usize {
                // SAFETY: `require_cb_first` checked the luma stride, the
                // buffer is write-locked, and `row < height`.
                let dst = unsafe { plane_row_mut(luma, row, luma_tight) };
                ramps.luma_row(frame, u32::try_from(row).expect("row fits u32"), dst);
            }
            for row in 0..spec.height.div_ceil(2) as usize {
                // SAFETY: chroma plane 1 is the Cb-first interleaved row
                // `require_cb_first` accepted, write-locked, `row` in range.
                let dst = unsafe { plane_row_mut(chroma, row, chroma_tight) };
                ramps.chroma_row(frame, u32::try_from(row).expect("row fits u32"), dst);
            }
            Ok(())
        })
    }

    /// Copies the filled buffer's planes into `dst`. The chroma source
    /// is the Cb-first interleaved row; plane 2 is the Cr byte of that
    /// same row and is not read separately.
    pub fn copy_planes(
        &self,
        frame: u32,
        dst: &mut wgpu::BufferViewMut,
        luma_stride: usize,
        chroma_offset: usize,
        chroma_stride: usize,
    ) -> Result<(), BenchError> {
        let buffer = self.buffers[frame as usize % RING].as_ptr();
        let spec = self.spec;
        with_planes(buffer, false, |planes| {
            require_cb_first(planes, &spec)?;
            let luma = &planes.planes[0];
            let chroma = &planes.planes[1];
            let luma_tight = spec.width as usize * spec.code_bytes();
            let chroma_tight = spec.width.div_ceil(2) as usize * 2 * spec.code_bytes();
            let luma_rows = spec.height as usize;
            let chroma_rows = spec.height.div_ceil(2) as usize;
            let need = chroma_offset + chroma_rows * chroma_stride;
            if dst.len() < need {
                return Err(BenchError::Engine(format!(
                    "external-cost: staging buffer is {} bytes, chroma needs {need}",
                    dst.len()
                )));
            }
            for row in 0..luma_rows {
                // SAFETY: read-locked luma plane, stride checked, row in range.
                let src = unsafe { plane_row(luma, row, luma_tight) };
                super::write_tight(dst, row * luma_stride, src);
            }
            for row in 0..chroma_rows {
                // SAFETY: read-locked interleaved chroma plane, stride checked.
                let src = unsafe { plane_row(chroma, row, chroma_tight) };
                super::write_tight(dst, chroma_offset + row * chroma_stride, src);
            }
            Ok(())
        })
    }

    /// Path `e`: the filled buffer imported as a Vulkan-native
    /// [`ExternalFrame`]. The second value is the import form.
    pub fn external(
        &self,
        frame: u32,
        color: FrameColor,
    ) -> Result<(ExternalFrame, &'static str), BenchError> {
        let buffer = self.buffers[frame as usize % RING];
        let native = self
            .vulkan
            .import(vulkan::FrameSource::Ahb(Box::new(vulkan::Ahb {
                buffer: buffer.as_ptr().cast(),
                sync: None,
                release: None,
                color,
                alpha: RgbAlpha::Opaque,
                hdr: HdrMetadata::default(),
            })))
            .map_err(|e| BenchError::Engine(format!("external-cost: AHB import failed: {e}")))?;
        let form = import_form(&native);
        let frame = ExternalFrame::native(native).map_err(|e| {
            BenchError::Engine(format!("external-cost: invalid native frame: {e:?}"))
        })?;
        Ok((frame, form))
    }

    /// Arms the completion wait for the submit that just sampled `frame`.
    pub fn retire(&mut self, frame: u32, queue: &wgpu::Queue) {
        self.done.arm(frame, queue);
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        for buffer in self.buffers.drain(..) {
            // SAFETY: each buffer was allocated once in `alloc_ahb` and
            // is released once here. No lock is held.
            unsafe { ndk_sys::AHardwareBuffer_release(buffer.as_ptr()) };
        }
    }
}
