use waterui_graphics::{Scene2D, SceneContent};

use crate::vello_renderer::SvgSceneData;

/// SceneView-backed SVG content.
pub struct SvgSceneContent {
    scene_data: SvgSceneData,
}

impl SvgSceneContent {
    #[must_use]
    pub fn new(svg_content: &str) -> Self {
        Self {
            scene_data: SvgSceneData::parse(svg_content),
        }
    }
}

impl SceneContent for SvgSceneContent {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) {
        let svg_scene = self.scene_data.build_scene(width, height);
        scene.append_vello_scene(&svg_scene, None);
    }
}
