use crate::dimensions::{
    TOGGLE_CHECKBOX_SIZE, TOGGLE_SWITCH_HEIGHT, TOGGLE_SWITCH_OUTLINE_WIDTH, TOGGLE_SWITCH_WIDTH,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, ToggleMetrics, WidgetInteractionState, lerp_color};
use waterui_controls::toggle::ToggleStyle;

pub fn metrics(style: ToggleStyle) -> ToggleMetrics {
    match style {
        ToggleStyle::Automatic | ToggleStyle::Switch => {
            ToggleMetrics::new(TOGGLE_SWITCH_WIDTH, TOGGLE_SWITCH_HEIGHT)
        }
        ToggleStyle::Checkbox => ToggleMetrics::new(TOGGLE_CHECKBOX_SIZE, TOGGLE_CHECKBOX_SIZE),
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }
}

pub fn draw_switch(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    progress: f32,
) {
    let track_color = lerp_color(
        colors.surface_container_highest.peniko(),
        colors.primary.peniko(),
        progress,
    );
    let handle_radius = crate::lerp_f64(8.0, 12.0, progress);
    let thumb_center_x = crate::lerp_f64(
        bounds.x0 + 4.0 + handle_radius,
        bounds.x1 - 4.0 - handle_radius,
        progress,
    );
    let thumb_center = vello::kurbo::Point::new(thumb_center_x, bounds.y0 + bounds.height() / 2.0);
    draw.fill_rounded_rect(bounds, 16.0.into(), &Brush::from(track_color));
    let outline_opacity = (1.0 - progress).clamp(0.0, 1.0);
    if outline_opacity > 0.0 {
        draw.stroke_rounded_rect(
            bounds,
            16.0.into(),
            &Brush::from(colors.outline.peniko().with_alpha(outline_opacity)),
            TOGGLE_SWITCH_OUTLINE_WIDTH,
        );
    }
    let thumb_color = lerp_color(
        colors.outline.peniko(),
        colors.on_primary.peniko(),
        progress,
    );
    draw.fill_circle(thumb_center, handle_radius, &Brush::from(thumb_color));
}

pub fn draw_switch_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    progress: f32,
    state: WidgetInteractionState,
) {
    let handle_radius = crate::lerp_f64(8.0, 12.0, progress);
    let thumb_center_x = crate::lerp_f64(
        bounds.x0 + 4.0 + handle_radius,
        bounds.x1 - 4.0 - handle_radius,
        progress,
    );
    let center = vello::kurbo::Point::new(thumb_center_x, bounds.y0 + bounds.height() / 2.0);
    let color = if progress > 0.5 {
        colors.primary.peniko()
    } else {
        colors.on_surface.peniko()
    };
    state_layer::draw_unbounded_circle(draw, center, 20.0, color, state);
}

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};

    use super::{MaterialColorScheme, draw_switch};
    use crate::{Brush, DrawContext};

    #[derive(Default)]
    struct RecordingDrawContext {
        rounded_stroke_count: usize,
    }

    impl DrawContext for RecordingDrawContext {
        fn fill_rect(&mut self, _rect: Rect, _brush: &Brush) {}

        fn fill_rounded_rect(&mut self, _rect: Rect, _radii: RoundedRectRadii, _brush: &Brush) {}

        fn stroke_rect(&mut self, _rect: Rect, _brush: &Brush, _width: f64) {}

        fn stroke_rounded_rect(
            &mut self,
            _rect: Rect,
            _radii: RoundedRectRadii,
            _brush: &Brush,
            _width: f64,
        ) {
            self.rounded_stroke_count += 1;
        }

        fn stroke_line(&mut self, _from: Point, _to: Point, _brush: &Brush, _width: f64) {}

        fn stroke_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush, _width: f64) {}

        fn fill_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush) {}

        fn fill_path(&mut self, _path: &BezPath, _brush: &Brush) {}

        fn stroke_path(&mut self, _path: &BezPath, _brush: &Brush, _width: f64) {}

        fn push_layer(&mut self, _alpha: f32, _clip: Option<&Rect>) {}

        fn pop_layer(&mut self) {}

        fn push_transform(&mut self, _affine: Affine) {}

        fn pop_transform(&mut self) {}
    }

    #[test]
    fn selected_material_switch_track_has_no_outline() {
        let colors = MaterialColorScheme::baseline_light();
        let bounds = Rect::from_origin_size((0.0, 0.0), (52.0, 32.0));

        let mut unselected = RecordingDrawContext::default();
        draw_switch(&colors, &mut unselected, bounds, 0.0);

        let mut selected = RecordingDrawContext::default();
        draw_switch(&colors, &mut selected, bounds, 1.0);

        assert_eq!(unselected.rounded_stroke_count, 1);
        assert_eq!(selected.rounded_stroke_count, 0);
    }
}

pub fn draw_checkbox(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    progress: f32,
) {
    draw.fill_rounded_rect(
        bounds,
        2.0.into(),
        &Brush::from(lerp_color(
            colors.surface.peniko(),
            colors.primary.peniko(),
            progress,
        )),
    );
    draw.stroke_rounded_rect(
        bounds,
        2.0.into(),
        &Brush::from(lerp_color(
            colors.on_surface_variant.peniko(),
            colors.primary.peniko(),
            progress,
        )),
        2.0,
    );
    if progress <= 0.0 {
        return;
    }
    let check = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(
            bounds.width().mul_add(0.25, bounds.x0),
            bounds.height().mul_add(0.55, bounds.y0),
        )),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
            bounds.width().mul_add(0.45, bounds.x0),
            bounds.height().mul_add(0.75, bounds.y0),
        )),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
            bounds.width().mul_add(0.78, bounds.x0),
            bounds.height().mul_add(0.3, bounds.y0),
        )),
    ]);
    draw.stroke_path(
        &check,
        &Brush::from(colors.on_primary.peniko().with_alpha(progress)),
        2.0,
    );
}

pub fn draw_checkbox_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    progress: f32,
    state: WidgetInteractionState,
) {
    let center = vello::kurbo::Point::new(
        bounds.x0 + bounds.width() / 2.0,
        bounds.y0 + bounds.height() / 2.0,
    );
    let color = if progress > 0.0 {
        colors.primary.peniko()
    } else {
        colors.on_surface.peniko()
    };
    state_layer::draw_unbounded_circle(draw, center, 20.0, color, state);
}
