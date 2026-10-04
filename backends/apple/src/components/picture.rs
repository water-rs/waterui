//! The `picture` leaf: `Native<Picture>` rendered through the kit's image
//! view — `WuiPictureView`.
//!
//! The `Computed<PictureRecording>` is watched imperatively: every change
//! rasterizes the recording through the CPU rasteriser and repaints. The
//! bitmap is sized to the view's laid-out bounds times the backing scale —
//! the layout, window-move and backing-change hooks all re-rasterize, so a
//! picture stretched past its declared size stays sharp instead of
//! upscaling a fixed bitmap. Layout is aspect-fit — the same
//! `sizeThatFits` the Swift leaf measured with — and the image announces
//! itself through the picture's `label`/`value` and the image trait.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::objc2::AllocAnyThread;
use waterui::graphics::picture::{Picture, PictureRecording};
use waterui::graphics::raster::Rasterizer;
use waterui::reactive::Signal;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::ImageView;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::ImageView;
}

use platform::ImageView;

/// The image view's measure: the Swift `sizeThatFits` — aspect-preserving
/// with the picture's own size as the answer when a side is unproposed.
fn fit(proposal: ProposalSize, point_size: Size) -> Size {
    let (pw, ph) = (point_size.width, point_size.height);
    match (proposal.width, proposal.height) {
        (None, None) => point_size,
        (Some(w), Some(h)) => Size::new(w, h),
        (Some(w), None) => Size::new(w, w * ph / pw),
        (None, Some(h)) => Size::new(h * pw / ph, h),
    }
}

/// The image view as its platform view, for the leaf.
fn as_view(view: &ImageView) -> &cocoa_ui::PlatformView {
    view
}

/// The pixel size `points` occupies at `scale` pixels per point — the
/// raster target for a view laid out at `points`. `None` while the bounds
/// are empty, which is how a view that has never been laid out answers.
///
/// # Panics
///
/// When a side would exceed the 65535 pixels a rasteriser addresses — the
/// same bound `Picture::pixel_size` asserts.
fn raster_pixels(points: cocoa_ui::Size, scale: f64) -> Option<(u32, u32)> {
    if !(points.width.is_finite()
        && points.height.is_finite()
        && points.width > 0.0
        && points.height > 0.0)
    {
        return None;
    }
    let width = (points.width * scale).round().max(1.0);
    let height = (points.height * scale).round().max(1.0);
    assert!(
        width <= f64::from(u16::MAX) && height <= f64::from(u16::MAX),
        "a picture is at most 65535 pixels a side, got {width}x{height}"
    );
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded to an integer and range-checked just above"
    )]
    let pixels = (width as u32, height as u32);
    Some(pixels)
}

/// The aspect-fit the leaf applies, folded into the raster: the
/// recording's point space scaled uniformly into the `width × height`
/// pixel target and centered, so the image view's `ScaleMode::Fit` shows
/// it 1:1 rather than re-fitting a bitmap of a different aspect.
fn fit_transform(picture: &Picture, width: u32, height: u32) -> kurbo::Affine {
    let size = picture.size();
    let scale =
        (f64::from(width) / f64::from(size.width)).min(f64::from(height) / f64::from(size.height));
    let x = f64::from(size.width).mul_add(-scale, f64::from(width)) / 2.0;
    let y = f64::from(size.height).mul_add(-scale, f64::from(height)) / 2.0;
    kurbo::Affine::translate(kurbo::Vec2::new(x, y)) * kurbo::Affine::scale(scale)
}

/// Rasterizes `recording` into the platform image the view displays —
/// the CPU raster path shared by the first paint and every re-raster.
/// `bounds` is the view's laid-out size in points; `pixels` is the
/// raster target it maps to at `scale`.
///
/// # Panics
///
/// When the CPU surface or the render fails — a dropped frame silently
/// leaves stale pixels, so a rasterisation failure is an error, not a
/// skipped paint.
#[expect(
    unused_variables,
    reason = "each platform's image type needs one of the two: UIImage takes the scale, NSImage the point size"
)]
fn rasterize(
    picture: &Picture,
    recording: &PictureRecording,
    bounds: cocoa_ui::Size,
    pixels: (u32, u32),
    scale: f64,
    view: &ImageView,
) {
    let (width, height) = pixels;
    let mut rasterizer =
        Rasterizer::new(width, height).expect("CPU rasteriser surface creation failed");
    let bitmap = rasterizer
        .rasterize(recording, fit_transform(picture, width, height))
        .expect("CPU rasterisation failed");
    let Some(image) = cocoa_ui::bitmap::image_from_rgba(
        bitmap.data(),
        bitmap.width() as usize,
        bitmap.height() as usize,
    ) else {
        return;
    };
    // The recording's own colours are the content: a template image would
    // discard them and recolour the alpha mask from the platform tint. The
    // image's point size is the laid-out bounds the bitmap was rasterized
    // for, so the image view draws it without rescaling.
    #[cfg(target_os = "macos")]
    let platform_image = cocoa_ui::objc2_app_kit::NSImage::initWithCGImage_size(
        cocoa_ui::objc2_app_kit::NSImage::alloc(),
        &image,
        cocoa_ui::Size::new(bounds.width, bounds.height).into(),
    );
    #[cfg(target_os = "ios")]
    let platform_image = cocoa_ui::objc2_ui_kit::UIImage::initWithCGImage_scale_orientation(
        cocoa_ui::objc2_ui_kit::UIImage::alloc(),
        &image,
        scale,
        cocoa_ui::objc2_ui_kit::UIImageOrientation::Up,
    );
    view.set_image(Some(&platform_image));
}

/// What the leaf repaints from: the latest recording and the pixel size
/// the view's bitmap was rasterized at — a layout or backing event landing
/// on the same pixels leaves the bitmap alone.
struct RasterState {
    recording: PictureRecording,
    pixels: Option<(u32, u32)>,
}

/// Re-rasterizes when the laid-out bounds change the pixel size the
/// bitmap needs — the layout, window-move and backing-change hooks all
/// land here.
fn repaint(picture: &Picture, view: &ImageView, state: &Rc<RefCell<RasterState>>) {
    let bounds = cocoa_ui::view::bounds(view).size;
    let scale = view
        .backing_scale()
        .unwrap_or_else(picture_display_scale)
        .max(1.0);
    let Some(pixels) = raster_pixels(bounds, scale) else {
        return;
    };
    let mut guard = state.borrow_mut();
    if guard.pixels == Some(pixels) {
        return;
    }
    guard.pixels = Some(pixels);
    rasterize(picture, &guard.recording, bounds, pixels, scale, view);
    drop(guard);
    crate::invalidation::invalidate_rendered_content(view);
}

/// The image view's layout face: intrinsic and aspect-fit.
struct PictureSubView {
    size: Size,
}

impl core::fmt::Debug for PictureSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PictureSubView")
            .field("size", &self.size)
            .finish()
    }
}

impl SubView for PictureSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(fit(proposal, self.size))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `picture` handler.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Picture>(|picture, ctx| {
        let mtm = ctx.mtm();
        let view = ImageView::new(mtm);
        view.set_scale_mode(cocoa_ui::ScaleMode::Fit);

        // Accessibility: the offered label and value, plus the image trait
        // while there is an image to describe — `updateAccessibility`.
        if picture.label().is_some() || picture.value().is_some() {
            view.set_image_trait();
            if let Some(label) = picture.label() {
                view.set_accessibility_label(Some(label.as_str()));
            }
            if let Some(value) = picture.value() {
                #[cfg(target_os = "macos")]
                view.set_accessibility_value(value.as_str());
                #[cfg(target_os = "ios")]
                view.set_accessibility_value(Some(value.as_str()));
            }
        }

        let state = Rc::new(RefCell::new(RasterState {
            recording: picture.recording().snapshot(),
            pixels: None,
        }));

        // First paint at the declared size and current backing scale —
        // `captureDisplayScale` + the CPU raster path of `updatePicture`;
        // the layout hook re-rasterizes once real bounds arrive.
        let scale = view
            .backing_scale()
            .unwrap_or_else(picture_display_scale)
            .max(1.0);
        let declared = cocoa_ui::Size::new(
            f64::from(picture.size().width),
            f64::from(picture.size().height),
        );
        if let Some(pixels) = raster_pixels(declared, scale) {
            state.borrow_mut().pixels = Some(pixels);
            rasterize(
                &picture,
                &state.borrow().recording,
                declared,
                pixels,
                scale,
                &view,
            );
        }

        // Re-rasterize on every bounds-affecting event the kit reports —
        // `layout`/`layoutSubviews`, `viewDidMoveToWindow`/`didMoveToWindow`
        // and `viewDidChangeBackingProperties`/`traitCollectionDidChange:`,
        // the same hooks the Swift leaf re-rasterized on. The handlers take
        // the view as their argument: capturing it here would retain it
        // through its own ivars.
        view.set_layout_handler({
            let state = state.clone();
            let picture = picture.clone();
            move |view| repaint(&picture, view, &state)
        });
        view.set_window_handler({
            let state = state.clone();
            let picture = picture.clone();
            move |view| repaint(&picture, view, &state)
        });
        view.set_backing_changed_handler({
            let state = state.clone();
            let picture = picture.clone();
            move |view| repaint(&picture, view, &state)
        });

        let mut leaf = NativeLeaf::new(
            as_view(&view),
            PictureSubView {
                size: picture.size(),
            },
        );
        leaf.watch(picture.recording(), {
            let view = view;
            let picture = picture.clone();
            move |change| {
                {
                    let mut guard = state.borrow_mut();
                    guard.recording = change.into_value();
                    // The dedupe key is the pixel size, not the content:
                    // a new drawing re-rasterizes at the same size too.
                    guard.pixels = None;
                }
                repaint(&picture, &view, &state);
            }
        });
        leaf
    });
}

/// The scale the Swift leaf rasterized at when the view was off-window:
/// the main screen's backing factor.
fn picture_display_scale() -> f64 {
    #[cfg(target_os = "macos")]
    {
        cocoa_ui::appkit::main_screen_scale()
    }
    #[cfg(target_os = "ios")]
    {
        cocoa_ui::uikit::main_screen_scale()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_matches_the_swift_table() {
        let size = Size::new(100.0, 50.0);
        assert_eq!(
            fit(
                ProposalSize {
                    width: None,
                    height: None
                },
                size
            ),
            size
        );
        assert_eq!(
            fit(
                ProposalSize {
                    width: Some(200.0),
                    height: Some(80.0)
                },
                size
            ),
            Size::new(200.0, 80.0)
        );
        assert_eq!(
            fit(
                ProposalSize {
                    width: Some(200.0),
                    height: None
                },
                size
            ),
            Size::new(200.0, 100.0)
        );
        assert_eq!(
            fit(
                ProposalSize {
                    width: None,
                    height: Some(100.0)
                },
                size
            ),
            Size::new(200.0, 100.0)
        );
    }
}
