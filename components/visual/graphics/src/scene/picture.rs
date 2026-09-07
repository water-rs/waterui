//! A recorded drawing shown as a static image.

use alloc::sync::Arc;
use core::fmt;

use kurbo::Affine;
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal};
use waterui_core::layout::Size;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::scene_view::{
    SceneContent, SceneInvalidator, SceneView, SceneViewMergeToParent, invalidate_on_change,
};
use crate::scene2d::{Scene2D, SceneRecording};

/// A recorded drawing shown as a static image.
///
/// The recording is a signal, so a drawing that follows a signal (an icon
/// tinted from the foreground colour) replaces its commands without replacing
/// the view. Backends that draw their own pixels replay the recording into
/// their scene; backends built on platform views rasterise it once at the
/// display's scale and show the pixels in the platform's image view, which is
/// what keeps a static drawing from costing a GPU surface of its own.
#[derive(Clone)]
pub struct Picture {
    recording: Computed<Arc<SceneRecording>>,
    size: Size,
}

impl fmt::Debug for Picture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Picture")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Picture {
    /// A picture of `size` points showing `recording`, whose commands are
    /// authored in the same point space.
    ///
    /// # Panics
    ///
    /// When `size` is not finite and positive: a picture with no area is an
    /// authoring error, not something to lay out.
    pub fn new(size: Size, recording: impl IntoComputed<Arc<SceneRecording>>) -> Self {
        assert!(
            size.width.is_finite()
                && size.height.is_finite()
                && size.width > 0.0
                && size.height > 0.0,
            "a Picture needs a finite, positive size, got {}x{}",
            size.width,
            size.height
        );
        Self {
            recording: recording.into_computed(),
            size,
        }
    }

    /// Records `draw` into a fresh recording.
    #[must_use]
    pub fn record(draw: impl FnOnce(&mut dyn Scene2D)) -> Arc<SceneRecording> {
        let mut recording = SceneRecording::new();
        draw(&mut recording);
        Arc::new(recording)
    }

    /// The picture's size in points.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The drawing, as a signal.
    #[must_use]
    pub const fn recording(&self) -> &Computed<Arc<SceneRecording>> {
        &self.recording
    }

    /// The pixel size of this picture at `scale` pixels per point, each side
    /// rounded and at least one pixel.
    ///
    /// # Panics
    ///
    /// When `scale` is not finite and positive, or a side would exceed the
    /// 65535 pixels a rasteriser addresses.
    #[must_use]
    pub fn pixel_size(&self, scale: f32) -> (u32, u32) {
        assert!(
            scale.is_finite() && scale > 0.0,
            "a picture is rasterised at a finite, positive scale, got {scale}"
        );
        let pixels = |points: f32| {
            let rounded = (points * scale).round().max(1.0);
            assert!(
                rounded <= f32::from(u16::MAX),
                "a picture is at most 65535 pixels a side, got {rounded}"
            );
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "rounded to an integer and range-checked just above"
            )]
            let pixels = rounded as u32;
            pixels
        };
        (pixels(self.size.width), pixels(self.size.height))
    }

    /// The transform that maps the picture's points onto a `width × height`
    /// pixel target.
    #[must_use]
    pub fn transform_to(&self, width: f32, height: f32) -> Affine {
        Affine::scale_non_uniform(
            f64::from(width / self.size.width),
            f64::from(height / self.size.height),
        )
    }
}

impl NativeView for Picture {}

impl View for Picture {
    fn body(self, env: &Environment) -> impl View {
        if env.get::<SceneViewMergeToParent>().is_some() {
            return AnyView::new(SceneView::new(RecordedScene {
                picture: self,
                watcher: None,
            }));
        }
        AnyView::new(Native::new(self))
    }
}

/// Scene content replaying a picture's current recording, for backends that
/// draw their own scene.
struct RecordedScene {
    picture: Picture,
    watcher: Option<BoxWatcherGuard>,
}

impl SceneContent for RecordedScene {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        let recording = self.picture.recording.get();
        recording.replay(scene, Some(self.picture.transform_to(width, height)));
        false
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.watcher = invalidator
            .map(|invalidator| invalidate_on_change(&invalidator, &self.picture.recording));
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(self.picture.size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Rect, Shape};
    use nami::{SignalExt, binding, constant};
    use peniko::{Brush, Color, Fill};
    use waterui_core::layout::StretchAxis;

    fn square(color: Color) -> Arc<SceneRecording> {
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
    fn a_picture_has_its_own_size() {
        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(Color::BLACK)));
        assert_eq!(NativeView::stretch_axis(&picture), StretchAxis::None);
        assert_eq!(picture.size(), Size::new(10.0, 10.0));
        assert_eq!(picture.pixel_size(2.5), (25, 25));
    }

    #[test]
    fn a_backend_that_draws_its_own_scene_gets_a_scene_view_and_the_rest_a_raw_picture() {
        let picture = || Picture::new(Size::new(10.0, 10.0), constant(square(Color::BLACK)));
        let merged =
            AnyView::new(picture().body(&Environment::new().extending(SceneViewMergeToParent)));
        assert!(merged.downcast::<SceneView>().is_ok());
        let raw = AnyView::new(picture().body(&Environment::new()));
        assert!(raw.downcast::<Native<Picture>>().is_ok());
    }

    #[test]
    fn a_new_recording_reaches_the_replayed_scene_without_a_new_view() {
        let tint = binding(Color::BLACK);
        let picture = Picture::new(Size::new(10.0, 10.0), tint.map(square));
        let mut content = RecordedScene {
            picture,
            watcher: None,
        };
        let mut scene = SceneRecording::new();
        content.build_scene(&mut scene, 20.0, 20.0);
        assert_eq!(scene.len(), 1);
        tint.set(Color::WHITE);
        let mut scene = SceneRecording::new();
        content.build_scene(&mut scene, 20.0, 20.0);
        assert_eq!(scene.len(), 1);
    }
}
