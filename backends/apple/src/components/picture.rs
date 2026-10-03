//! The `picture` leaf: `Native<Picture>` rendered through the kit's image
//! view — `WuiPictureView`.
//!
//! The `Computed<PictureRecording>` is watched imperatively: every change
//! rasterizes the recording at the view's current backing scale through the
//! CPU rasteriser and repaints. Layout is aspect-fit — the same
//! `sizeThatFits` the Swift leaf measured with — and the image announces
//! itself through the picture's `label`/`value` and the image trait.

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

/// Rasterizes `recording` at `scale` into the platform image the view
/// displays — `captureDisplayScale` + the CPU raster path of
/// `updatePicture`.
///
/// # Panics
///
/// When the CPU surface or the render fails — a dropped frame silently
/// leaves stale pixels, so a rasterisation failure is an error, not a
/// skipped paint.
#[allow(clippy::cast_precision_loss)]
fn rasterize(picture: &Picture, recording: &PictureRecording, scale: f64, view: &ImageView) {
    let scale = scale.max(1.0);
    #[expect(clippy::cast_possible_truncation, reason = "display scales are small")]
    let (width, height) = picture.pixel_size(scale as f32);
    let mut rasterizer =
        Rasterizer::new(width, height).expect("CPU rasteriser surface creation failed");
    let bitmap = rasterizer
        .rasterize(recording, picture.transform_to(width as f32, height as f32))
        .expect("CPU rasterisation failed");
    let Some(image) = cocoa_ui::bitmap::image_from_rgba(
        bitmap.data(),
        bitmap.width() as usize,
        bitmap.height() as usize,
    ) else {
        return;
    };
    // The recording's own colours are the content: a template image would
    // discard them and recolour the alpha mask from the platform tint.
    #[cfg(target_os = "macos")]
    let platform_image = cocoa_ui::objc2_app_kit::NSImage::initWithCGImage_size(
        cocoa_ui::objc2_app_kit::NSImage::alloc(),
        &image,
        cocoa_ui::Size::new(
            f64::from(picture.size().width),
            f64::from(picture.size().height),
        )
        .into(),
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

        // First paint at the current backing scale; re-rasterize when the
        // window's backing changes — `viewDidMoveToWindow` +
        // `viewDidChangeBackingProperties`.
        let scale = view.backing_scale().unwrap_or_else(picture_display_scale);
        rasterize(&picture, &picture.recording().snapshot(), scale, &view);
        let mut leaf = NativeLeaf::new(
            as_view(&view),
            PictureSubView {
                size: picture.size(),
            },
        );
        leaf.bind(picture.recording(), {
            let view = view;
            let picture = picture.clone();
            move |recording| {
                let scale = view.backing_scale().unwrap_or_else(picture_display_scale);
                rasterize(&picture, &recording, scale, &view);
                crate::invalidation::invalidate_rendered_content(&view);
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
