use crate::dimensions::{
    TOGGLE_CHECKBOX_CONTAINER_SHAPE, TOGGLE_CHECKBOX_OUTLINE_WIDTH,
    TOGGLE_CHECKBOX_SELECTED_SCALE_START, TOGGLE_CHECKBOX_SIZE, TOGGLE_SWITCH_HEIGHT,
    TOGGLE_SWITCH_OUTLINE_WIDTH, TOGGLE_SWITCH_WIDTH,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, ToggleMetrics, WidgetInteractionState, lerp_color};
use vello::kurbo::{Affine, BezPath, PathEl, Point, Rect};
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
    bounds: Rect,
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
    let thumb_center = Point::new(thumb_center_x, bounds.y0 + bounds.height() / 2.0);
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
    bounds: Rect,
    progress: f32,
    state: WidgetInteractionState,
) {
    let handle_radius = crate::lerp_f64(8.0, 12.0, progress);
    let thumb_center_x = crate::lerp_f64(
        bounds.x0 + 4.0 + handle_radius,
        bounds.x1 - 4.0 - handle_radius,
        progress,
    );
    let center = Point::new(thumb_center_x, bounds.y0 + bounds.height() / 2.0);
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

    use super::{MaterialColorScheme, draw_checkbox, draw_switch};
    use crate::{Brush, DrawContext};

    #[derive(Default)]
    struct RecordingDrawContext {
        rounded_stroke_count: usize,
        rounded_fill_count: usize,
        path_stroke_count: usize,
        transform_depth: usize,
    }

    impl DrawContext for RecordingDrawContext {
        fn fill_rect(&mut self, _rect: Rect, _brush: &Brush) {}

        fn fill_rounded_rect(&mut self, _rect: Rect, _radii: RoundedRectRadii, _brush: &Brush) {
            self.rounded_fill_count += 1;
        }

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

        fn stroke_path(&mut self, _path: &BezPath, _brush: &Brush, _width: f64) {
            self.path_stroke_count += 1;
        }

        fn push_layer(&mut self, _alpha: f32, _clip: Option<&Rect>) {}

        fn pop_layer(&mut self) {}

        fn push_transform(&mut self, _affine: Affine) {
            self.transform_depth += 1;
        }

        fn pop_transform(&mut self) {
            self.transform_depth -= 1;
        }
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

    #[test]
    fn selected_material_checkbox_has_no_outline() {
        let colors = MaterialColorScheme::baseline_light();
        let bounds = Rect::from_origin_size((0.0, 0.0), (18.0, 18.0));

        let mut unselected = RecordingDrawContext::default();
        draw_checkbox(&colors, &mut unselected, bounds, 0.0);

        let mut selected = RecordingDrawContext::default();
        draw_checkbox(&colors, &mut selected, bounds, 1.0);

        assert_eq!(unselected.rounded_stroke_count, 1);
        assert_eq!(unselected.rounded_fill_count, 0);
        assert_eq!(selected.rounded_stroke_count, 0);
        assert_eq!(selected.rounded_fill_count, 1);
        assert_eq!(selected.path_stroke_count, 1);
        assert_eq!(selected.transform_depth, 0);
    }
}

pub fn draw_checkbox(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    progress: f32,
) {
    let progress = progress.clamp(0.0, 1.0);
    let outline_opacity = 1.0 - progress;
    if outline_opacity > 0.0 {
        draw.stroke_rounded_rect(
            bounds,
            TOGGLE_CHECKBOX_CONTAINER_SHAPE.into(),
            &Brush::from(
                colors
                    .on_surface_variant
                    .peniko()
                    .with_alpha(outline_opacity),
            ),
            TOGGLE_CHECKBOX_OUTLINE_WIDTH,
        );
    }
    if progress <= 0.0 {
        return;
    }

    let selected_scale = crate::lerp_f64(TOGGLE_CHECKBOX_SELECTED_SCALE_START, 1.0, progress);
    let selected_transform = Affine::translate((bounds.center().x, bounds.center().y))
        * Affine::scale(selected_scale)
        * Affine::translate((-bounds.center().x, -bounds.center().y));
    draw.push_transform(selected_transform);
    draw.fill_rounded_rect(
        bounds,
        TOGGLE_CHECKBOX_CONTAINER_SHAPE.into(),
        &Brush::from(colors.primary.peniko().with_alpha(progress)),
    );
    let check = BezPath::from_vec(vec![
        PathEl::MoveTo(Point::new(
            bounds.width().mul_add(0.25, bounds.x0),
            bounds.height().mul_add(0.55, bounds.y0),
        )),
        PathEl::LineTo(Point::new(
            bounds.width().mul_add(0.45, bounds.x0),
            bounds.height().mul_add(0.75, bounds.y0),
        )),
        PathEl::LineTo(Point::new(
            bounds.width().mul_add(0.78, bounds.x0),
            bounds.height().mul_add(0.3, bounds.y0),
        )),
    ]);
    draw.stroke_path(
        &check,
        &Brush::from(colors.on_primary.peniko().with_alpha(progress)),
        TOGGLE_CHECKBOX_OUTLINE_WIDTH,
    );
    draw.pop_transform();
}

pub fn draw_checkbox_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    progress: f32,
    state: WidgetInteractionState,
) {
    let center = Point::new(
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
