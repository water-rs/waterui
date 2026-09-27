use super::{Brush, DrawContext};
use vello::kurbo::{
    Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Vec2,
};

/// Closes the scene's current clip layer, then re-arms the encoding's
/// transform/style force flags.
///
/// `pop_layer`/`encode_end_clip` emits a dummy `PATH` with no transform or
/// style — nothing re-arms the encoder's force flags at the boundary, so a
/// path drawn next dedups its transform against the encoding's last entry.
/// Once the resolver splices a glyph run's transform entries into the packed
/// stream ahead of that path, the path's `trans_ix` lands on the run's paint
/// transform instead of its own (water-rs/hydrolysis#250). Re-arming the
/// force flags keeps every post-clip path carrying its own transform.
pub(crate) fn pop_scene_layer(scene: &mut vello::Scene) {
    scene.pop_layer();
    scene.encoding_mut().force_next_transform_and_style();
}

/// Appends a child scene, then re-arms the encoding's transform/style force
/// flags: `Encoding::append` copies the child's `flags` verbatim, so a force
/// pending in this encoding is silently dropped for the next encode — the
/// same clip-boundary hole [`pop_scene_layer`] covers
/// (water-rs/hydrolysis#250).
pub(crate) fn append_scene(
    scene: &mut vello::Scene,
    child: &vello::Scene,
    transform: Option<Affine>,
) {
    scene.append(child, transform);
    scene.encoding_mut().force_next_transform_and_style();
}

pub struct VelloDrawContext<'a> {
    scene: &'a mut vello::Scene,
    transform_stack: Vec<Affine>,
}

impl<'a> VelloDrawContext<'a> {
    pub fn with_root_transform(scene: &'a mut vello::Scene, transform: Affine) -> Self {
        Self {
            scene,
            transform_stack: vec![Affine::IDENTITY, transform],
        }
    }

    fn transform(&self) -> Affine {
        *self
            .transform_stack
            .last()
            .expect("vello draw context transform stack is empty")
    }

    fn fill_shape(&mut self, shape: &impl Shape, brush: &Brush) {
        match brush {
            Brush::Solid(color) => {
                self.scene.fill(
                    vello::peniko::Fill::NonZero,
                    self.transform(),
                    color,
                    None,
                    shape,
                );
            }
            Brush::Gradient(gradient) => {
                self.scene.fill(
                    vello::peniko::Fill::NonZero,
                    self.transform(),
                    gradient,
                    None,
                    shape,
                );
            }
        }
    }

    fn stroke_shape(&mut self, shape: &impl Shape, brush: &Brush, width: f64) {
        let stroke = vello::kurbo::Stroke::new(width);
        match brush {
            Brush::Solid(color) => {
                self.scene
                    .stroke(&stroke, self.transform(), color, None, shape);
            }
            Brush::Gradient(gradient) => {
                self.scene
                    .stroke(&stroke, self.transform(), gradient, None, shape);
            }
        }
    }
}

impl DrawContext for VelloDrawContext<'_> {
    fn fill_rect(&mut self, rect: Rect, brush: &Brush) {
        self.fill_shape(&rect, brush);
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radii: RoundedRectRadii, brush: &Brush) {
        let rounded = RoundedRect::from_rect(rect, radii);
        self.fill_shape(&rounded, brush);
    }

    fn stroke_rect(&mut self, rect: Rect, brush: &Brush, width: f64) {
        self.stroke_shape(&rect, brush, width);
    }

    fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        brush: &Brush,
        width: f64,
    ) {
        let rounded = RoundedRect::from_rect(rect, radii);
        self.stroke_shape(&rounded, brush, width);
    }

    fn stroke_line(&mut self, from: Point, to: Point, brush: &Brush, width: f64) {
        let line = Line::new(from, to);
        self.stroke_shape(&line, brush, width);
    }

    fn stroke_circle(&mut self, center: Point, radius: f64, brush: &Brush, width: f64) {
        let circle = Circle::new(center, radius);
        self.stroke_shape(&circle, brush, width);
    }

    fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush) {
        let circle = Circle::new(center, radius);
        self.fill_shape(&circle, brush);
    }

    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        self.fill_shape(path, brush);
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, width: f64) {
        self.stroke_shape(path, brush, width);
    }

    fn draw_shadow(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        offset: Vec2,
        blur: f64,
        color: vello::peniko::Color,
    ) {
        let radius = radii
            .as_single_radius()
            .expect("vello blurred shadows require uniform corner radii");
        self.scene.draw_blurred_rounded_rect(
            self.transform(),
            rect + offset,
            color,
            radius,
            blur.max(0.0),
        );
    }

    fn push_layer(&mut self, alpha: f32, clip: Option<&Rect>) {
        let clip = clip
            .copied()
            .unwrap_or(Rect::new(-1.0e9, -1.0e9, 1.0e9, 1.0e9));
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            self.transform(),
            &clip,
        );
    }

    fn push_rounded_layer(&mut self, alpha: f32, clip: Rect, radii: RoundedRectRadii) {
        let clip = RoundedRect::from_rect(clip, radii);
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            self.transform(),
            &clip,
        );
    }

    fn pop_layer(&mut self) {
        pop_scene_layer(self.scene);
    }

    fn push_transform(&mut self, affine: Affine) {
        let current = self.transform();
        self.transform_stack.push(current * affine);
    }

    fn pop_transform(&mut self) {
        assert!(
            self.transform_stack.len() > 1,
            "vello draw context transform stack underflow"
        );
        self.transform_stack.pop();
    }
}
