//! `AHardwareBuffer` plumbing for the procedural frames in [`pattern`].

use std::mem::MaybeUninit;
use std::ptr;
use std::sync::Once;

use ndk_sys::{AHardwareBuffer, AHardwareBuffer_Desc, AHardwareBuffer_Planes};

use crate::logcat;
use crate::pattern::{self, Plane};
use crate::scenario::Format;

pub use pattern::{HEIGHT, WIDTH};

/// `AHardwareBuffer` usage for a video: GPU-sampled and CPU-written;
/// `overlay` adds the `COMPOSER_OVERLAY` bit plane promotion requires.
#[must_use]
pub const fn usage(overlay: bool) -> u64 {
    use ndk_sys::AHardwareBuffer_UsageFlags as U;
    let mut flags =
        U::AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE.0 | U::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0;
    if overlay {
        flags |= U::AHARDWAREBUFFER_USAGE_COMPOSER_OVERLAY.0;
    }
    flags
}

/// Allocates one buffer, returning the owned reference.
///
/// # Panics
/// When the device refuses a format or usage this harness requires.
#[must_use]
pub fn alloc(format: Format, overlay: bool) -> *mut AHardwareBuffer {
    let format_code = match format {
        Format::Nv12 => ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_Y8Cb8Cr8_420.0,
        Format::P010 => ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_YCbCr_P010.0,
    };
    let desc = AHardwareBuffer_Desc {
        width: WIDTH,
        height: HEIGHT,
        layers: 1,
        format: format_code,
        usage: usage(overlay),
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    assert_ne!(
        unsafe { ndk_sys::AHardwareBuffer_isSupported(&raw const desc) },
        0,
        "{format:?} AHB is not supported at {WIDTH}x{HEIGHT}"
    );
    let mut buffer = ptr::null_mut();
    assert_eq!(
        unsafe { ndk_sys::AHardwareBuffer_allocate(&raw const desc, &raw mut buffer) },
        0,
        "{format:?} AHB allocation failed"
    );
    buffer
}

/// Releases the reference `alloc` returned.
///
/// # Safety
/// `buffer` is a live, owned `AHardwareBuffer` reference.
pub unsafe fn release(buffer: *mut AHardwareBuffer) {
    unsafe { ndk_sys::AHardwareBuffer_release(buffer) };
}

/// Logs the buffer's `AHardwareBuffer_describe` and, for the flexible
/// format, its resolved `AHardwareBuffer_lockPlanes` layout — once per
/// process, on the first fill.
fn dump_layout(
    format: Format,
    buffer: *mut AHardwareBuffer,
    planes: *const AHardwareBuffer_Planes,
) {
    let mut desc = AHardwareBuffer_Desc {
        width: 0,
        height: 0,
        layers: 0,
        format: 0,
        usage: 0,
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    unsafe { ndk_sys::AHardwareBuffer_describe(buffer, &raw mut desc) };
    logcat::line(&format!(
        "ahb describe {format:?}: {}x{} layers={} format={:#x} usage={:#x} stride={}",
        desc.width, desc.height, desc.layers, desc.format, desc.usage, desc.stride
    ));
    if planes.is_null() {
        return;
    }
    let planes = unsafe { &*planes };
    logcat::line(&format!(
        "ahb lockPlanes {format:?}: planeCount={}",
        planes.planeCount
    ));
    for (i, plane) in planes
        .planes
        .iter()
        .take(planes.planeCount as usize)
        .enumerate()
    {
        logcat::line(&format!(
            "ahb plane[{i}]: data={:#x} rowStride={} pixelStride={} offset_from_plane0={}",
            plane.data as usize,
            plane.rowStride,
            plane.pixelStride,
            (plane.data as usize).wrapping_sub(planes.planes[0].data as usize)
        ));
    }
}

static NV12_DUMPED: Once = Once::new();
static P010_DUMPED: Once = Once::new();

/// Writes frame `frame` of the pattern into `buffer`.
///
/// # Safety
/// `buffer` is a live `AHardwareBuffer` of `format` allocated with CPU
/// write usage, and nothing else is writing it.
pub unsafe fn fill(format: Format, buffer: *mut AHardwareBuffer, frame: u64) {
    match format {
        Format::Nv12 => unsafe { fill_nv12(buffer, frame) },
        Format::P010 => unsafe { fill_p010(buffer, frame) },
    }
}

/// # Safety
/// As [`fill`].
unsafe fn fill_nv12(buffer: *mut AHardwareBuffer, frame: u64) {
    let mut planes = MaybeUninit::<AHardwareBuffer_Planes>::uninit();
    let rc = unsafe {
        ndk_sys::AHardwareBuffer_lockPlanes(
            buffer,
            ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0,
            -1,
            ptr::null_mut(),
            planes.as_mut_ptr(),
        )
    };
    assert_eq!(rc, 0, "AHB lockPlanes");
    let planes = unsafe { planes.assume_init() };
    NV12_DUMPED.call_once(|| dump_layout(Format::Nv12, buffer, &raw const planes));
    let mapped: Vec<Plane> = planes.planes[..planes.planeCount as usize]
        .iter()
        .map(|plane| Plane {
            data: plane.data.cast::<u8>(),
            row_stride: plane.rowStride as usize,
            pixel_stride: plane.pixelStride as usize,
        })
        .collect();
    unsafe { pattern::fill_nv12(&mapped, frame) };
    unsafe { ndk_sys::AHardwareBuffer_unlock(buffer, ptr::null_mut()) };
}

/// # Safety
/// As [`fill`].
unsafe fn fill_p010(buffer: *mut AHardwareBuffer, frame: u64) {
    let mut addr = ptr::null_mut();
    let rc = unsafe {
        ndk_sys::AHardwareBuffer_lock(
            buffer,
            ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0,
            -1,
            ptr::null(),
            &raw mut addr,
        )
    };
    assert_eq!(rc, 0, "P010 AHB lock");
    let mut desc = AHardwareBuffer_Desc {
        width: 0,
        height: 0,
        layers: 0,
        format: 0,
        usage: 0,
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    unsafe { ndk_sys::AHardwareBuffer_describe(buffer, &raw mut desc) };
    P010_DUMPED.call_once(|| dump_layout(Format::P010, buffer, ptr::null()));
    // `stride` is the luma row stride in pixels (u16s).
    unsafe { pattern::fill_p010(addr.cast(), desc.stride as usize, frame) };
    unsafe { ndk_sys::AHardwareBuffer_unlock(buffer, ptr::null_mut()) };
}
