use waterui_core::layout::Size;
use waterui_graphics::{Scene2D, SceneContent};

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
