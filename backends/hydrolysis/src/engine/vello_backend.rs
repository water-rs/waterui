use super::{Brush, DrawContext};
use vello::kurbo::{
    Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, RoundedRectRadii, Shape,
};

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
        self.scene.pop_layer();
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
