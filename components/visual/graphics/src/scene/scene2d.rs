use kurbo::{Affine, BezPath, Stroke};
use peniko::{BlendMode, Brush, Fill, ImageBrush};

/// Rendering-engine-independent 2D scene builder interface.
pub trait Scene2D {
    /// Fills a shape with the given brush and transform.
    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &BezPath);

    /// Strokes a shape with the given brush and transform.
    fn stroke(&mut self, stroke: &Stroke, transform: Affine, brush: &Brush, shape: &BezPath);

    /// Pushes a clipped compositing layer.
    fn push_layer(
        &mut self,
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    );

    /// Pushes a clip-only layer.
    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath);

    /// Pops the current layer.
    fn pop_layer(&mut self);

    /// Draws an image with transform.
    fn draw_image(&mut self, image: &ImageBrush, transform: Affine);

    /// Appends a pre-built Vello scene.
    fn append_vello_scene(&mut self, scene: &vello::Scene, transform: Option<Affine>);

    /// Encodes directly into the backing Vello scene when available.
    ///
    /// Other scene implementations preserve the same behavior by recording into
    /// a temporary scene and appending it after `encode` returns.
    #[doc(hidden)]
    fn encode_vello(&mut self, encode: &mut dyn FnMut(&mut vello::Scene)) {
        let mut scene = vello::Scene::new();
        encode(&mut scene);
        self.append_vello_scene(&scene, None);
    }

    /// Clears all recorded scene commands.
    fn reset(&mut self);
}
