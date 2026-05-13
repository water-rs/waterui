use waterui::color::ResolvedColor;
use waterui::layout::Point;
use waterui::{Environment, View, ViewExt as _};
use waterui_canvas::{Canvas, DrawingContext, LineCap, LineJoin};
use waterui_core::resolve::Resolvable;

/// A shared Material checkmark drawn with WaterUI canvas primitives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CheckmarkIcon<ColorToken> {
    color: ColorToken,
    size: f32,
    line_width: f32,
    container_height: f32,
}

impl<ColorToken> CheckmarkIcon<ColorToken> {
    pub(crate) const fn new(color: ColorToken, size: f32, line_width: f32) -> Self {
        Self {
            color,
            size,
            line_width,
            container_height: size,
        }
    }

    pub(crate) const fn container_height(mut self, container_height: f32) -> Self {
        self.container_height = container_height;
        self
    }
}

impl<ColorToken> View for CheckmarkIcon<ColorToken>
where
    ColorToken: Resolvable<Resolved = ResolvedColor> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let stroke = self.color.resolve(env);
        Canvas::with_signal(stroke, move |ctx: &mut DrawingContext<'_>, stroke| {
            let x = (ctx.width - self.size) * 0.5;
            let y = (self.container_height - self.size) * 0.5;
            let mut path = ctx.begin_path();
            path.move_to(Point::new(x + 4.25, y + 9.25));
            path.line_to(Point::new(x + 7.25, y + 12.25));
            path.line_to(Point::new(x + 14.0, y + 5.5));
            ctx.set_stroke_style(stroke);
            ctx.set_line_width(self.line_width);
            ctx.set_line_cap(LineCap::Round);
            ctx.set_line_join(LineJoin::Round);
            ctx.stroke_path(&path);
        })
        .size(self.size, self.size)
    }
}
