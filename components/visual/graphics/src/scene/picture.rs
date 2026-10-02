//! A recorded drawing shown as a static image.

use alloc::rc::Rc;
use core::fmt;

use cherenkov::kurbo::Affine;
use cherenkov::{Draw, Fixed, Recorder, StaticRecorder};
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal};
use waterui_core::Str;
use waterui_core::layout::Size;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::scene::resources::{HeldResources, RecordingResources, SceneResources};
use crate::scene_view::{
    SceneContent, SceneInvalidator, SceneView, SceneViewMergeToParent, invalidate_on_change,
};

/// A static drawing together with the engine registrations it names.
///
/// What a [`Picture`] shows. [`Picture::record`] makes one that names no
/// engine resource; [`Picture::record_with`] makes one that draws fonts,
/// images or shader paints registered through an engine's
/// [`SceneResources`], and it holds those registrations for as long as it —
/// or any recording that draws it — is alive, so the code that recorded it
/// can let its own handles go.
///
/// The display list never leaves it on its own. Everything that draws a
/// recording takes the recording whole — a scene merging a [`Picture`], a
/// `Rasterizer`, an `OffscreenRenderer` — and holds its
/// registrations for as long as it may draw them, so the ids in the list
/// cannot outlive what they name.
type ResourcePictureRecipe = dyn for<'a> Fn(&mut StaticRecorder, &mut RecordingResources<'a>);

#[derive(Clone)]
pub struct PictureRecording {
    picture: cherenkov::Picture,
    held: HeldResources,
    recipe: Option<Rc<ResourcePictureRecipe>>,
}

impl fmt::Debug for PictureRecording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PictureRecording")
            .field("picture", &self.picture)
            .field("held", &self.held)
            .field("replayable", &self.recipe.is_some())
            .finish()
    }
}

impl PictureRecording {
    /// The display list, for a drawer that holds [`Self::held`] beside it.
    pub(crate) const fn picture(&self) -> &cherenkov::Picture {
        &self.picture
    }

    /// The registrations the display list names; a recording that draws
    /// this one holds them through [`RecordingResources::hold`].
    pub(crate) const fn held(&self) -> &HeldResources {
        &self.held
    }

    /// Re-records this picture against `resources` while preserving its
    /// semantic source. Resource-free pictures retain their exact immutable
    /// snapshot; resource-bearing pictures must have been created with the
    /// replayable [`Picture::record_with`] factory.
    pub(crate) fn rebuild_for_engine(&self, resources: &SceneResources) -> Self {
        let Some(recipe) = &self.recipe else {
            return Self {
                picture: self.picture.clone(),
                held: HeldResources::empty(),
                recipe: None,
            };
        };
        let mut names = resources.recording();
        let picture = cherenkov::Picture::record(|recorder| recipe(recorder, &mut names));
        Self {
            picture,
            held: names.finish(),
            recipe: Some(Rc::clone(recipe)),
        }
    }
}

/// A recorded drawing shown as a static image.
///
/// The picture is a signal of [`PictureRecording`]s, so a drawing that
/// follows a signal (an icon tinted from the foreground colour) replaces its
/// commands without replacing the view. Backends that draw their
/// own pixels mount the picture on a layer of their engine; backends built on
/// platform views rasterise it once at the display's scale and show the
/// pixels in the platform's image view, which is what keeps a static drawing
/// from costing a GPU surface of its own.
#[derive(Clone)]
pub struct Picture {
    recording: Computed<PictureRecording>,
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
    pub fn new(size: Size, recording: impl IntoComputed<PictureRecording>) -> Self {
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

    /// Records `draw` into a fresh static picture that names no engine
    /// resource.
    ///
    /// The closure draws into a [`StaticRecorder`], so every operand it names
    /// is a constant of the resulting display list — a picture has no live
    /// signals of its own. A drawing that should follow a signal belongs in
    /// the `recording` signal instead: re-record a new picture when the value
    /// changes, which is exactly what a `Computed` recording does.
    #[must_use]
    pub fn record(draw: impl FnOnce(&mut StaticRecorder)) -> PictureRecording {
        PictureRecording {
            picture: cherenkov::Picture::record(draw),
            held: HeldResources::empty(),
            recipe: None,
        }
    }

    /// Records `draw` into a fresh static picture that draws resources
    /// registered through `resources`.
    ///
    /// The closure names each resource it draws through the
    /// [`RecordingResources`] it is handed, and the recording holds every
    /// registration it names; see [`PictureRecording`]. The closure owns the
    /// semantic source it needs to register, so the recording can replay it
    /// when a host replaces its engine generation. Capturing a
    /// [`Registered`](crate::scene::resources::Registered) from `resources`
    /// would retain an old-engine handle and is rejected when replayed.
    #[must_use]
    pub fn record_with(
        resources: &SceneResources,
        draw: impl for<'a> Fn(&mut StaticRecorder, &mut RecordingResources<'a>) + 'static,
    ) -> PictureRecording {
        let recipe: Rc<ResourcePictureRecipe> = Rc::new(draw);
        let mut names = resources.recording();
        let picture = cherenkov::Picture::record(|recorder| recipe(recorder, &mut names));
        PictureRecording {
            picture,
            held: names.finish(),
            recipe: Some(recipe),
        }
    }

    /// The picture's size in points.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The drawing, as a signal.
    #[must_use]
    pub const fn recording(&self) -> &Computed<PictureRecording> {
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
                current: None,
                rebuild_recording: false,
            }));
        }
        AnyView::new(Native::new(self))
    }
}

/// Scene content drawing a picture's current recording, for backends that
/// own their own engine layer tree.
struct RecordedScene {
    picture: Picture,
    watcher: Option<BoxWatcherGuard>,
    current: Option<PictureRecording>,
    rebuild_recording: bool,
}

impl SceneContent for RecordedScene {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        let snapshot = self.picture.recording.snapshot();
        let changed = self
            .current
            .as_ref()
            .is_none_or(|current| current.picture() != snapshot.picture());
        if changed {
            self.current = Some(if self.rebuild_recording {
                snapshot.rebuild_for_engine(resources.scene_resources())
            } else {
                snapshot
            });
        }
        let recording = self
            .current
            .as_ref()
            .expect("a picture recording is selected before drawing");
        resources.hold(recording.held());
        recorder.picture(
            recording.picture(),
            Fixed(self.picture.transform_to(width, height)),
        );
        false
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.watcher = invalidator
            .map(|invalidator| invalidate_on_change(&invalidator, &self.picture.recording));
    }

    fn rebuild_for_engine(self: Box<Self>) -> Box<dyn SceneContent> {
        let Self {
            picture,
            watcher: _,
            current: _,
            rebuild_recording: _,
        } = *self;
        Box::new(Self {
            picture,
            watcher: None,
            current: None,
            rebuild_recording: true,
        })
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
    use cherenkov::kurbo::{Rect, Shape};
    use cherenkov::testing::Event;
    use cherenkov::{ImageId, Sampling, WorkingColor};
    use nami::{SignalExt, binding, constant};
    use waterui_core::layout::StretchAxis;

    fn square(color: WorkingColor) -> PictureRecording {
        Picture::record(|scene| {
            scene.fill(Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1), color);
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
    fn a_backend_that_draws_its_own_scene_gets_a_scene_view_and_the_rest_a_raw_picture() {
        let picture = || Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)));
        let merged =
            AnyView::new(picture().body(&Environment::new().extending(SceneViewMergeToParent)));
        assert!(merged.downcast::<SceneView>().is_ok());
        let raw = AnyView::new(picture().body(&Environment::new()));
        assert!(raw.downcast::<Native<Picture>>().is_ok());
    }

    #[test]
    fn a_labeled_picture_offers_its_name_and_an_unlabeled_one_stays_quiet() {
        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)));
        assert_eq!(picture.label(), None);
        assert_eq!(picture.value(), None);
        let quiet = RecordedScene {
            picture,
            watcher: None,
            current: None,
            rebuild_recording: false,
        };
        assert_eq!(quiet.accessibility_label(), None);
        assert_eq!(quiet.accessibility_value(), None);

        let picture = Picture::new(Size::new(10.0, 10.0), constant(square(WorkingColor::BLACK)))
            .labeled("Warning");
        assert_eq!(picture.label().map(Str::as_str), Some("Warning"));
        let named = RecordedScene {
            picture,
            watcher: None,
            current: None,
            rebuild_recording: false,
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
            current: None,
            rebuild_recording: false,
        };
        assert_eq!(described.accessibility_label().as_deref(), Some("Warning"));
        assert_eq!(
            described.accessibility_value().as_deref(),
            Some("A triangle with an exclamation mark"),
            "the description reaches the scene's value channel, not its name"
        );
    }

    /// Content that draws a plain fill and names nothing.
    struct Blank;

    impl SceneContent for Blank {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            recorder.fill(
                Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                WorkingColor::BLACK,
            );
            false
        }

        fn rebuild_for_engine(self: Box<Self>) -> Box<dyn SceneContent> {
            self
        }
    }

    #[test]
    fn a_picture_holds_the_resources_it_names_after_its_recorder_lets_go() {
        use crate::scene::resources::tests::{Mount, one_pixel, removed_images};

        let mount = Mount::new();
        let source = alloc::sync::Arc::<[u8]>::from([255, 0, 0, 255]);
        let named = Rc::new(core::cell::RefCell::new(None));
        let named_for_recipe = Rc::clone(&named);
        let recording = Picture::record_with(&mount.resources, move |recorder, resources| {
            let image = resources
                .image(
                    cherenkov::ImageData::<cherenkov::Rgba8>::new(
                        1,
                        1,
                        alloc::sync::Arc::clone(&source),
                    )
                    .expect("image source"),
                )
                .expect("image registration");
            let id = resources.name(&image);
            *named_for_recipe.borrow_mut() = Some(id);
            recorder.image(id, Rect::new(0.0, 0.0, 10.0, 10.0), Sampling::Nearest);
        });
        let id = named.borrow().expect("the picture named its image");
        assert_eq!(removed_images(&mount.render()), Vec::<ImageId>::new());

        let replacement = Mount::new();
        let rebuilt = recording.rebuild_for_engine(&replacement.resources);
        let mut replacement_content = RecordedScene {
            picture: Picture::new(Size::new(10.0, 10.0), constant(rebuilt)),
            watcher: None,
            current: None,
            rebuild_recording: false,
        };
        let replacement_frame = replacement.frame(&mut replacement_content);
        assert!(
            replacement_frame
                .events
                .iter()
                .any(|event| matches!(event, Event::AddImage(_))),
            "rebuilding a picture must register its source on the replacement engine"
        );
        assert!(
            replacement_frame
                .events
                .iter()
                .any(|event| matches!(event, Event::SetContent(..))),
            "the replacement recording must be installed before rendering"
        );

        let mut content = RecordedScene {
            picture: Picture::new(Size::new(10.0, 10.0), constant(recording)),
            watcher: None,
            current: None,
            rebuild_recording: false,
        };
        let shown = mount.frame(&mut content);
        assert_eq!(
            removed_images(&shown.events),
            Vec::<ImageId>::new(),
            "the image a mounted picture draws was released: {:?}",
            shown.events
        );
        // The picture view goes; the recording that draws it is still
        // installed, and still holds the image.
        drop(content);
        assert_eq!(removed_images(&mount.render()), Vec::<ImageId>::new());

        let replaced = mount.frame(&mut Blank);
        assert_eq!(removed_images(&replaced.events), [id]);
    }

    #[test]
    fn a_new_recording_reaches_the_recorded_content_without_a_new_view() {
        let (resources, _events) = crate::scene::resources::tests::null_resources();
        let tint = binding(WorkingColor::BLACK);
        let picture = Picture::new(Size::new(10.0, 10.0), tint.map(square));
        let mut content = RecordedScene {
            picture,
            watcher: None,
            current: None,
            rebuild_recording: false,
        };
        let mut recorder = Recorder::new();
        content.build_scene(&mut recorder, &mut resources.recording(), 20.0, 20.0);
        let first = recorder.finish();
        assert_eq!(first.len(), 1);
        tint.set(WorkingColor::WHITE);
        let mut recorder = Recorder::new();
        content.build_scene(&mut recorder, &mut resources.recording(), 20.0, 20.0);
        let second = recorder.finish();
        assert_eq!(second.len(), 1);
    }
}
