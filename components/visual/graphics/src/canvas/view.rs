//! The leaf a mounted [`Canvas`](super::Canvas) becomes.

use alloc::string::String;
use core::fmt;

use waterui_core::layout::{Size, StretchAxis};
use waterui_core::{Environment, Native, NativeView, View};

use super::Canvas;
use super::scene::CanvasScene;
use crate::draw::Recorder;
use crate::scene::resources::{RecordingResources, SceneResources};
use crate::scene_view::{
    NextFrame, SceneContent, SceneFrame, SceneInvalidator, SceneUpdate, scene_stretch_axis,
};

/// A mounted canvas, as a backend receives it: the raw leaf
/// [`Canvas`](super::Canvas) composes to.
///
/// A backend realizes it with the renderer that draws the screen around it —
/// a Core Animation layer tree on Apple, the scene mount of Hydrolysis's own
/// engine elsewhere. Its recording is installed once and edited in place as
/// operands move and regions re-record: [`update_scene`](Self::update_scene)
/// brings the installed recording up to date and answers
/// [`SceneUpdate::Record`] only when there is none, and
/// [`build_scene`](Self::build_scene) records it.
pub struct CanvasView {
    scene: CanvasScene,
}

impl fmt::Debug for CanvasView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanvasView").finish_non_exhaustive()
    }
}

impl CanvasView {
    /// Runs `canvas`'s closure — once, for this mount.
    pub(crate) fn mount(canvas: Canvas, env: &Environment) -> Self {
        Self {
            scene: CanvasScene::mount(canvas, env),
        }
    }

    /// Records the canvas afresh into `recorder` for a `width` by `height`
    /// box in logical points, registering its resources through
    /// `resources`. Returns the frames it needs next.
    pub fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> NextFrame {
        self.scene.build_scene(recorder, resources, width, height)
    }

    /// Brings the installed recording up to date for `frame`, in place.
    pub fn update_scene(&mut self, resources: &SceneResources, frame: SceneFrame) -> SceneUpdate {
        self.scene.update_scene(resources, frame)
    }

    /// Forgets the engine-bound state of the replaced engine; the next
    /// record installs everything again.
    pub fn rebuild_for_engine(&mut self) {
        self.scene.rebuild_for_engine();
    }

    /// Installs the callback a signal change asks for a frame through, or
    /// clears it on unmount.
    pub fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.scene.set_invalidator(invalidator);
    }

    /// The size the drawing naturally is, if it has one.
    #[must_use]
    pub fn intrinsic_size(&self) -> Option<Size> {
        self.scene.intrinsic_size()
    }

    /// The accessible name the drawing offers.
    #[must_use]
    pub fn accessibility_label(&self) -> Option<String> {
        self.scene.accessibility_label()
    }

    /// The accessible value the drawing offers.
    #[must_use]
    pub fn accessibility_value(&self) -> Option<String> {
        self.scene.accessibility_value()
    }
}

/// The form a backend with a generic scene mount hosts a canvas in.
impl SceneContent for CanvasView {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> NextFrame {
        Self::build_scene(self, recorder, resources, width, height)
    }

    fn rebuild_for_engine(&mut self) {
        Self::rebuild_for_engine(self);
    }

    fn update_scene(&mut self, resources: &SceneResources, frame: SceneFrame) -> SceneUpdate {
        Self::update_scene(self, resources, frame)
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        Self::set_invalidator(self, invalidator);
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Self::intrinsic_size(self)
    }

    fn accessibility_label(&self) -> Option<String> {
        Self::accessibility_label(self)
    }

    fn accessibility_value(&self) -> Option<String> {
        Self::accessibility_value(self)
    }
}

impl NativeView for CanvasView {
    fn stretch_axis(&self) -> StretchAxis {
        scene_stretch_axis(self.intrinsic_size())
    }
}

impl View for CanvasView {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        scene_stretch_axis(self.intrinsic_size())
    }
}
