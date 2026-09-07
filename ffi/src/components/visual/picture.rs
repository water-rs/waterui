//! FFI for [`Picture`]: a recorded drawing a backend shows in its image view.
//!
//! The backend keeps the picture handle for the view's lifetime and asks it for
//! a bitmap signal at its own display scale; the signal re-rasterises whenever
//! the drawing changes, so a tint that follows a signal reaches the image view
//! without a new view. Pixels are premultiplied RGBA8, rows top to bottom.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::mem::ManuallyDrop;

use nami::SignalExt;
use waterui_graphics::Picture;
use waterui_graphics::scene2d_cpu::{Rasterizer, RgbaBitmap};

#[cfg(feature = "c-api")]
use crate::reactive::WuiComputed;
use crate::{IntoFFI, ffi_computed};

/// Opaque handle owning a `Picture`.
pub struct WuiPictureHandle(pub(crate) Picture);

impl core::fmt::Debug for WuiPictureHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WuiPictureHandle").finish_non_exhaustive()
    }
}

/// FFI representation of a `Picture` view: the handle plus its size in points.
#[repr(C)]
#[derive(Debug)]
pub struct WuiPicture {
    /// Owned handle; release it with `waterui_drop_picture`.
    pub picture: *mut WuiPictureHandle,
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
}

impl IntoFFI for Picture {
    type FFI = WuiPicture;

    fn into_ffi(self) -> Self::FFI {
        let size = self.size();
        WuiPicture {
            picture: Box::into_raw(Box::new(WuiPictureHandle(self))),
            width: size.width,
            height: size.height,
        }
    }
}

ffi_view!(Picture, WuiPicture, picture);

/// Premultiplied RGBA8 pixels, `width * height * 4` bytes, owned by the
/// receiver until `waterui_drop_bitmap`.
#[repr(C)]
#[derive(Debug)]
pub struct WuiBitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The pixel bytes.
    pub data: *mut u8,
    /// Number of pixel bytes.
    pub len: usize,
    /// Allocation size behind `data`; `waterui_drop_bitmap` needs it back.
    pub capacity: usize,
}

impl IntoFFI for RgbaBitmap {
    type FFI = WuiBitmap;

    fn into_ffi(self) -> Self::FFI {
        let (width, height) = (self.width(), self.height());
        let mut data = ManuallyDrop::new(self.into_data());
        WuiBitmap {
            width,
            height,
            data: data.as_mut_ptr(),
            len: data.len(),
            capacity: data.capacity(),
        }
    }
}

/// Releases a bitmap handed out by a bitmap computed.
///
/// # Safety
///
/// `bitmap` must come from this library and must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_bitmap(bitmap: WuiBitmap) {
    // SAFETY: the fields are the parts of the `Vec<u8>` `IntoFFI` took apart.
    drop(unsafe { Vec::from_raw_parts(bitmap.data, bitmap.len, bitmap.capacity) });
}

ffi_computed!(RgbaBitmap, WuiBitmap, bitmap);

/// The picture rasterised at `scale` pixels per point, as a signal that
/// follows the drawing. One rasteriser serves every re-draw of the picture.
pub(crate) fn bitmap_signal(picture: &Picture, scale: f32) -> waterui::Computed<RgbaBitmap> {
    let (width, height) = picture.pixel_size(scale);
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel_size bounds both sides to 65535, which f32 holds exactly"
    )]
    let transform = picture.transform_to(width as f32, height as f32);
    let rasterizer = Rc::new(RefCell::new(Rasterizer::new(width, height)));
    picture
        .recording()
        .map(move |recording| rasterizer.borrow_mut().rasterize(&recording, transform))
        .computed()
}

/// The picture rasterised at `scale` pixels per point, as a signal.
///
/// It re-rasterises whenever the drawing changes. Drop it with
/// `waterui_drop_computed_bitmap`, and ask again when the display scale
/// changes.
///
/// # Safety
///
/// `picture` must be a live handle from `waterui_force_as_picture`.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_picture_bitmap(
    picture: *const WuiPictureHandle,
    scale: f32,
) -> *mut WuiComputed<RgbaBitmap> {
    // SAFETY: the caller keeps the handle alive for the view's lifetime.
    let picture = unsafe { &(*picture).0 };
    bitmap_signal(picture, scale).into_ffi()
}

/// Releases a picture handle.
///
/// # Safety
///
/// `picture` must come from `waterui_force_as_picture` and must not be used
/// afterwards.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_picture(picture: *mut WuiPictureHandle) {
    // SAFETY: ownership returns here exactly once by the caller contract.
    drop(unsafe { Box::from_raw(picture) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Affine, Rect, Shape};
    use peniko::{Brush, Color, Fill};
    use waterui::Signal;
    use waterui_core::layout::Size;
    use waterui_core::{binding, constant};

    fn square(color: Color) -> alloc::sync::Arc<waterui_graphics::SceneRecording> {
        Picture::record(|scene| {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &Brush::Solid(color),
                None,
                &Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1),
            );
        })
    }

    #[test]
    fn a_bitmap_signal_rasterises_at_the_display_scale() {
        let picture = Picture::new(Size::new(10.0, 5.0), constant(square(Color::BLACK)));
        let bitmap = bitmap_signal(&picture, 3.0).get();
        assert_eq!((bitmap.width(), bitmap.height()), (30, 15));
        assert_eq!(bitmap.data().len(), 30 * 15 * 4);
        assert_eq!(&bitmap.data()[..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn a_new_drawing_reaches_the_bitmap_signal() {
        let tint = binding(Color::BLACK);
        let picture = Picture::new(Size::new(10.0, 10.0), tint.map(square));
        let bitmaps = bitmap_signal(&picture, 1.0);
        assert_eq!(&bitmaps.get().data()[..4], &[0, 0, 0, 255]);
        tint.set(Color::WHITE);
        assert_eq!(&bitmaps.get().data()[..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn a_bitmap_crosses_the_boundary_and_comes_back_whole() {
        let picture = Picture::new(Size::new(2.0, 2.0), constant(square(Color::BLACK)));
        let ffi = bitmap_signal(&picture, 1.0).get().into_ffi();
        assert_eq!((ffi.width, ffi.height, ffi.len), (2, 2, 16));
        // SAFETY: `ffi` came from `into_ffi` just above and is dropped once.
        unsafe { waterui_drop_bitmap(ffi) };
    }
}
