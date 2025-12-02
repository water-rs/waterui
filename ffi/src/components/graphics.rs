use core::slice;

use crate::ffi_view;
use waterui_graphics::{RendererBufferFormat, RendererCpuSurface, RendererSurface, RendererView};

opaque!(WuiRendererView, RendererView);

/// Pixel formats supported by the renderer bridge FFI.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WuiRendererBufferFormat {
    /// 8-bit per channel RGBA pixels in native byte order.
    Rgba8888 = 0,
}

impl From<WuiRendererBufferFormat> for RendererBufferFormat {
    fn from(value: WuiRendererBufferFormat) -> Self {
        match value {
            WuiRendererBufferFormat::Rgba8888 => RendererBufferFormat::Rgba8888,
        }
    }
}

impl From<RendererBufferFormat> for WuiRendererBufferFormat {
    fn from(value: RendererBufferFormat) -> Self {
        match value {
            RendererBufferFormat::Rgba8888 => WuiRendererBufferFormat::Rgba8888,
        }
    }
}

ffi_view!(RendererView, *mut WuiRendererView, renderer_view);

/// Gets the width of the renderer view.
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_renderer_view_width(view: *const WuiRendererView) -> f32 {
    assert!(
        !view.is_null(),
        "waterui_renderer_view_width: received null pointer"
    );
    unsafe { (&(*view)).width }
}

/// Gets the height of the renderer view.
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_renderer_view_height(view: *const WuiRendererView) -> f32 {
    assert!(
        !view.is_null(),
        "waterui_renderer_view_height: received null pointer"
    );
    unsafe { (&(*view)).height }
}

/// Gets the preferred buffer format for the renderer view.
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_renderer_view_preferred_format(
    _view: *const WuiRendererView,
) -> WuiRendererBufferFormat {
    WuiRendererBufferFormat::Rgba8888
}

/// Renders the view to a CPU buffer.
///
/// # Safety
/// The caller must ensure that `view` and `pixels` are valid pointers, and that the
/// pixel buffer has sufficient capacity for the given dimensions and stride.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_renderer_view_render_cpu(
    view: *mut WuiRendererView,
    pixels: *mut u8,
    width: u32,
    height: u32,
    stride: usize,
    format: WuiRendererBufferFormat,
) -> bool {
    assert!(
        !view.is_null(),
        "waterui_renderer_view_render_cpu: received null view pointer"
    );
    assert!(
        !pixels.is_null(),
        "waterui_renderer_view_render_cpu: received null pixel pointer"
    );

    let expected = match stride.checked_mul(height as usize) {
        Some(value) => value,
        None => return false,
    };

    let format = RendererBufferFormat::from(format);
    if format != RendererBufferFormat::Rgba8888 {
        return false;
    }

    let buffer = unsafe { slice::from_raw_parts_mut(pixels, expected) };
    let surface = RendererCpuSurface::new(buffer, width, height, stride, format);

    let view = unsafe { &mut *view };
    let callback = &mut view.on_render;
    callback(RendererSurface::Cpu(surface));
    true
}
