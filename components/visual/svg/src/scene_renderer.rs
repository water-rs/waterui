use alloc::boxed::Box;
use alloc::rc::Rc;
use core::{any::Any, cell::Cell};

use waterui_core::Signal;
use waterui_core::layout::Size;
use waterui_graphics::{Scene2D, SceneContent, SceneInvalidator};

use crate::scene_data::SvgSceneData;

/// Scene content that draws one SVG document.
#[derive(Debug)]
pub struct SvgSceneContent {
    scene_data: SvgSceneData,
}

impl SvgSceneContent {
    /// Parses `svg_content` into content that draws it.
    #[must_use]
    pub fn new(svg_content: &str) -> Self {
        Self {
            scene_data: SvgSceneData::parse(svg_content),
        }
    }
}

impl SceneContent for SvgSceneContent {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        self.scene_data.draw(scene, width, height);
        false
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(self.scene_data.intrinsic_size())
    }
}

/// Scene content whose tint is driven by a signal without rebuilding the view tree.
pub struct ReactiveSvgSceneContent<S>
where
    S: Signal<Output = alloc::string::String> + 'static,
    S::Guard: 'static,
{
    svg_template: alloc::string::String,
    tint: S,
    scene_data: Option<SvgSceneData>,
    current_color: Option<alloc::string::String>,
    pending_update: Rc<Cell<bool>>,
    watcher_guard: Option<Box<dyn Any>>,
}

impl<S> core::fmt::Debug for ReactiveSvgSceneContent<S>
where
    S: Signal<Output = alloc::string::String> + 'static,
    S::Guard: 'static,
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ReactiveSvgSceneContent")
            .field("current_color", &self.current_color)
            .finish_non_exhaustive()
    }
}

impl<S> ReactiveSvgSceneContent<S>
where
    S: Signal<Output = alloc::string::String> + 'static,
    S::Guard: 'static,
{
    /// Content drawing `svg_template`, with its colour placeholder replaced by
    /// whatever `tint` currently reads.
    ///
    /// The document is parsed here rather than at the first draw: its geometry is
    /// the same whatever the tint is, and layout asks for the size before anything
    /// has drawn.
    #[must_use]
    pub fn new(svg_template: alloc::string::String, tint: S) -> Self {
        let mut content = Self {
            svg_template,
            tint,
            scene_data: None,
            current_color: None,
            pending_update: Rc::new(Cell::new(true)),
            watcher_guard: None,
        };
        content.ensure_scene_data();
        content
    }

    fn ensure_scene_data(&mut self) {
        let next_color = self.tint.get();
        let needs_rebuild = self.pending_update.replace(false)
            || self.current_color.as_ref() != Some(&next_color)
            || self.scene_data.is_none();
        if !needs_rebuild {
            return;
        }

        let svg_content = self
            .svg_template
            .replace(crate::scene_data::SVG_COLOR_PLACEHOLDER, &next_color);
        self.scene_data = Some(SvgSceneData::parse(&svg_content));
        self.current_color = Some(next_color);
    }
}

impl<S> SceneContent for ReactiveSvgSceneContent<S>
where
    S: Signal<Output = alloc::string::String> + 'static,
    S::Guard: 'static,
{
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        self.ensure_scene_data();
        self.scene_data
            .as_ref()
            .expect("reactive svg scene data must be initialized")
            .draw(scene, width, height);
        false
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(
            self.scene_data
                .as_ref()
                .expect("reactive svg scene data is parsed when the content is created")
                .intrinsic_size(),
        )
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        if self.watcher_guard.is_some() {
            return;
        }

        let pending_update = Rc::clone(&self.pending_update);
        let invalidator = invalidator.expect("ReactiveSvgSceneContent requires an invalidator");
        let guard = self.tint.watch(move |_context| {
            pending_update.set(true);
            invalidator();
        });
        self.watcher_guard = Some(Box::new(guard));
    }
}
