use crate::engine::WidgetTheme;
use waterui_core::Environment;

pub(crate) fn widget_theme(env: &Environment) -> &dyn WidgetTheme {
    env.get::<Box<dyn WidgetTheme>>()
        .map(Box::as_ref)
        .expect("hydrolysis widget theme is not installed in the environment")
}

pub(crate) fn lerp_f64(from: f64, to: f64, t: f32) -> f64 {
    from + (to - from) * f64::from(t)
}

pub(crate) fn lerp_color(from: [f32; 4], to: [f32; 4], t: f32) -> vello::peniko::Color {
    let lerp = |start: f32, end: f32| start + (end - start) * t;
    vello::peniko::Color::new([
        lerp(from[0], to[0]),
        lerp(from[1], to[1]),
        lerp(from[2], to[2]),
        lerp(from[3], to[3]),
    ])
}

pub(crate) fn inset_rect(rect: vello::kurbo::Rect, dx: f64, dy: f64) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        rect.x0 + dx,
        rect.y0 + dy,
        (rect.x1 - dx).max(rect.x0 + dx),
        (rect.y1 - dy).max(rect.y0 + dy),
    )
}
