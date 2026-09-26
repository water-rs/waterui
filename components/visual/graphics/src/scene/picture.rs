//! A recorded drawing shown as a static image.
use cherenkov::Draw;

use core::fmt;

use cherenkov::kurbo::Affine;
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal};
use waterui_core::Str;
use waterui_core::layout::Size;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, NativeView, View};

use crate::scene::resources::Scene;
use crate::scene_view::{SceneContent, SceneInvalidator, SceneView, invalidate_on_change};

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
    recording: Computed<cherenkov::Picture>,
    size: Size,
    label: Option<Str>,
    value: Option<Str>,
}

impl fmt::Debug for Picture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Picture")
            .field("size", &self.size)
            .field("label", &self.label)
            .field("value", &self.value)
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
    pub fn new(size: Size, recording: impl IntoComputed<cherenkov::Picture>) -> Self {
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
            label: None,
            value: None,
        }
    }

    /// Names the drawing for a screen reader.
    ///
    /// This is the name the picture *offers* — an SVG's `<title>`, an icon's
    /// meaning — and it reaches the accessibility tree only where the
    /// application has not named the view itself: an `.a11y_label(…)` on the
    /// picture or on any ancestor still wins, because the application knows
    /// what the drawing is for and the drawing only knows what it shows.
    #[must_use]
    pub fn labeled(mut self, label: impl Into<Str>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The name the drawing offers a screen reader, if it has one.
    #[must_use]
    pub const fn label(&self) -> Option<&Str> {
        self.label.as_ref()
    }

    /// Describes the drawing's content for a screen reader.
    ///
    /// This is the semantic payload the picture *offers* — an SVG's `<desc>`,
    /// a diagram's summary — announced after its name. It lives on the value
    /// channel, so an `.a11y_label(…)` that names the view does not have to
    /// stand in for what the drawing says, and an `.a11y_value(…)` the
    /// application sets wins over it wherever both exist.
    #[must_use]
    pub fn described(mut self, value: impl Into<Str>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// The semantic content the drawing offers a screen reader, if it has any.
    #[must_use]
    pub const fn value(&self) -> Option<&Str> {
        self.value.as_ref()
    }

    /// Records `draw` into a fresh recording.
    #[must_use]
    pub fn record(draw: impl FnOnce(&mut cherenkov::StaticRecorder)) -> cherenkov::Picture {
        cherenkov::Picture::record(draw)
    }

    /// The picture's size in points.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The drawing, as a signal.
    #[must_use]
    pub const fn recording(&self) -> &Computed<cherenkov::Picture> {
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
    fn body(self, _env: &Environment) -> impl View {
        SceneView::new(RecordedScene {
            picture: self,
            watcher: None,
        })
    }
}

/// Scene content replaying a picture's current recording, for backends that
/// draw their own scene.
struct RecordedScene {
    picture: Picture,
    watcher: Option<BoxWatcherGuard>,
}

impl SceneContent for RecordedScene {
    fn record(&mut self, scene: &mut Scene<'_>) -> bool {
        let recording = self.picture.recording.snapshot();
        let transform = self.picture.transform_to(scene.width(), scene.height());
        scene.recorder().picture(&recording, transform);
        false
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.watcher = invalidator
            .map(|invalidator| invalidate_on_change(&invalidator, &self.picture.recording));
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(self.picture.size)
    }

    fn accessibility_label(&self) -> Option<String> {
        self.picture
            .label
            .as_ref()
            .map(|label| label.as_str().to_owned())
    }

    fn accessibility_value(&self) -> Option<String> {
        self.picture
            .value
            .as_ref()
            .map(|value| value.as_str().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov::{Draw, WorkingColor};
    use cherenkov::kurbo::Rect;
    use nami::{SignalExt, binding, constant};
    use waterui_core::AnyView;
    use waterui_core::layout::StretchAxis;

    fn square(color: WorkingColor) -> cherenkov::Picture {
        Picture::record(|scene| {
            scene.fill(Rect::new(0.0, 0.0, 10.0, 10.0), color);
        })
    }

    #[test]
    fn a_picture_has_its_own_size() {
        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)));
        assert_eq!(NativeView::stretch_axis(&picture), StretchAxis::None);
        assert_eq!(picture.size(), Size::new(10.0, 10.0));
        assert_eq!(picture.pixel_size(2.5), (25, 25));
    }

    #[test]
    fn a_picture_is_a_scene_view_that_knows_its_size() {
        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)));
        let view = AnyView::new(picture.body(&Environment::new()));
        let scene = view
            .downcast::<SceneView>()
            .unwrap_or_else(|_| panic!("a picture renders as a scene view"));
        assert_eq!(scene.intrinsic_size(), Some(Size::new(10.0, 10.0)));
    }

    #[test]
    fn a_labeled_picture_offers_its_name_and_an_unlabeled_one_stays_quiet() {
        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)));
        assert_eq!(picture.label(), None);
        assert_eq!(picture.value(), None);
        let quiet = RecordedScene {
            picture,
            watcher: None,
        };
        assert_eq!(quiet.accessibility_label(), None);
        assert_eq!(quiet.accessibility_value(), None);

        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)))
            .labeled("Warning");
        assert_eq!(picture.label().map(Str::as_str), Some("Warning"));
        let named = RecordedScene {
            picture,
            watcher: None,
        };
        assert_eq!(named.accessibility_label().as_deref(), Some("Warning"));
    }

    #[test]
    fn a_described_picture_keeps_its_content_on_the_value_channel() {
        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)))
            .labeled("Warning")
            .described("A triangle with an exclamation mark");
        assert_eq!(
            picture.value().map(Str::as_str),
            Some("A triangle with an exclamation mark")
        );
        let described = RecordedScene {
            picture,
            watcher: None,
        };
        assert_eq!(described.accessibility_label().as_deref(), Some("Warning"));
        assert_eq!(
            described.accessibility_value().as_deref(),
            Some("A triangle with an exclamation mark"),
            "the description reaches the scene's value channel, not its name"
        );
    }

    #[test]
    fn a_new_recording_reaches_the_replayed_scene_without_a_new_view() {
        let tint = binding(WorkingColor::BLACK);
        let picture = Picture::new(Size::new(10.0, 10.0), tint.map(square));
        let mut content = RecordedScene {
            picture,
            watcher: None,
        };
        let resources = crate::scene::resources::testing::none();
        let mut first = cherenkov::Content::record(|recorder| {
            content.record(&mut Scene::new(recorder, &resources, 20.0, 20.0));
        });
        tint.set(WorkingColor::WHITE);
        let mut second = cherenkov::Content::record(|recorder| {
            content.record(&mut Scene::new(recorder, &resources, 20.0, 20.0));
        });
        assert_eq!(first.snapshot().commands().len(), 1);
        assert_eq!(second.snapshot().commands().len(), 1);
    }
}
