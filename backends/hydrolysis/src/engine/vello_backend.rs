use super::{Brush, DrawContext};
use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, RoundedRectRadii};

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

    fn brush(brush: &Brush) -> &vello::peniko::Color {
        match brush {
            Brush::Solid(color) => color,
        }
    }
}

impl DrawContext for VelloDrawContext<'_> {
    fn fill_rect(&mut self, rect: Rect, brush: &Brush) {
        self.scene.fill(
            vello::peniko::Fill::NonZero,
            self.transform(),
            Self::brush(brush),
            None,
            &rect,
        );
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radii: RoundedRectRadii, brush: &Brush) {
        let rounded = RoundedRect::from_rect(rect, radii);
        self.scene.fill(
            vello::peniko::Fill::NonZero,
            self.transform(),
            Self::brush(brush),
            None,
            &rounded,
        );
    }

    fn stroke_rect(&mut self, rect: Rect, brush: &Brush, width: f64) {
        let stroke = vello::kurbo::Stroke::new(width);
        self.scene
            .stroke(&stroke, self.transform(), Self::brush(brush), None, &rect);
    }

    fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        brush: &Brush,
        width: f64,
    ) {
        let stroke = vello::kurbo::Stroke::new(width);
        let rounded = RoundedRect::from_rect(rect, radii);
        self.scene.stroke(
            &stroke,
            self.transform(),
            Self::brush(brush),
            None,
            &rounded,
        );
    }

    fn stroke_line(&mut self, from: Point, to: Point, brush: &Brush, width: f64) {
        let stroke = vello::kurbo::Stroke::new(width);
        let line = Line::new(from, to);
        self.scene
            .stroke(&stroke, self.transform(), Self::brush(brush), None, &line);
    }

    fn stroke_circle(&mut self, center: Point, radius: f64, brush: &Brush, width: f64) {
        let stroke = vello::kurbo::Stroke::new(width);
        let circle = Circle::new(center, radius);
        self.scene
            .stroke(&stroke, self.transform(), Self::brush(brush), None, &circle);
    }

    fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush) {
        let circle = Circle::new(center, radius);
        self.scene.fill(
            vello::peniko::Fill::NonZero,
            self.transform(),
            Self::brush(brush),
            None,
            &circle,
        );
    }

    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        self.scene.fill(
            vello::peniko::Fill::NonZero,
            self.transform(),
            Self::brush(brush),
            None,
            path,
        );
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, width: f64) {
        let stroke = vello::kurbo::Stroke::new(width);
        self.scene
            .stroke(&stroke, self.transform(), Self::brush(brush), None, path);
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
