use crate::dimensions::{
    PICKER_HORIZONTAL_INSET, PICKER_INDICATOR_SPACE, PICKER_LABEL_SPACING,
    PICKER_MENU_POPUP_CORNER_RADIUS, PICKER_MENU_POPUP_ROW_HEIGHT, PICKER_MENU_POPUP_TOP_SPACING,
    PICKER_MIN_HEIGHT, PICKER_MIN_WIDTH, PICKER_RADIO_INDICATOR_SIZE,
    PICKER_RADIO_INNER_DOT_RADIUS, PICKER_RADIO_LABEL_SPACING, PICKER_RADIO_OUTER_RING_WIDTH,
    PICKER_RADIO_ROW_SPACING, PICKER_VERTICAL_INSET,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, PickerMetrics, WidgetInteractionState};
use waterui_form::picker::PickerStyle;

pub fn metrics(style: PickerStyle) -> PickerMetrics {
    match style {
        PickerStyle::Automatic | PickerStyle::Menu | PickerStyle::Radio => material_metrics(),
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
    }
}

const fn material_metrics() -> PickerMetrics {
    PickerMetrics {
        min_width: PICKER_MIN_WIDTH,
        min_height: PICKER_MIN_HEIGHT,
        horizontal_inset: PICKER_HORIZONTAL_INSET,
        vertical_inset: PICKER_VERTICAL_INSET,
        label_spacing: PICKER_LABEL_SPACING,
        indicator_space: PICKER_INDICATOR_SPACE,
        radio_indicator_size: PICKER_RADIO_INDICATOR_SIZE,
        radio_label_spacing: PICKER_RADIO_LABEL_SPACING,
        radio_row_spacing: PICKER_RADIO_ROW_SPACING,
        popup_top_spacing: PICKER_MENU_POPUP_TOP_SPACING,
        popup_row_height: PICKER_MENU_POPUP_ROW_HEIGHT,
        popup_corner_radius: PICKER_MENU_POPUP_CORNER_RADIUS,
    }
}

pub fn draw_indicator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    let center_x = PICKER_INDICATOR_SPACE.mul_add(-0.5, bounds.x1 - PICKER_HORIZONTAL_INSET);
    let center_y = bounds.height().mul_add(0.5, bounds.y0);
    let chevron = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(center_x - 4.0, center_y - 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x, center_y + 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x + 4.0, center_y - 2.0)),
    ]);
    draw.stroke_path(
        &chevron,
        &Brush::from(colors.on_surface_variant.peniko()),
        1.5,
    );
}

pub fn draw_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(draw, bounds, 4.0.into(), colors.on_surface.peniko(), state);
}

pub fn draw_popup(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    popup_rect: vello::kurbo::Rect,
) {
    draw.fill_rounded_rect(
        popup_rect,
        PICKER_MENU_POPUP_CORNER_RADIUS.into(),
        &Brush::from(colors.surface_container.peniko()),
    );
    draw.stroke_rounded_rect(
        popup_rect,
        PICKER_MENU_POPUP_CORNER_RADIUS.into(),
        &Brush::from(colors.outline_variant.peniko()),
        1.0,
    );
}

pub fn draw_popup_row_background(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    row_rect: vello::kurbo::Rect,
    selected: bool,
) {
    if !selected {
        return;
    }
    let inset = vello::kurbo::Rect::new(
        row_rect.x0 + 2.0,
        row_rect.y0 + 1.0,
        row_rect.x1 - 2.0,
        row_rect.y1 - 1.0,
    );
    draw.fill_rect(
        inset,
        &Brush::from(colors.surface_container_highest.peniko()),
    );
}

pub fn draw_popup_row_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    row_rect: vello::kurbo::Rect,
    selected: bool,
    state: WidgetInteractionState,
) {
    let inset = vello::kurbo::Rect::new(
        row_rect.x0 + 2.0,
        row_rect.y0 + 1.0,
        row_rect.x1 - 2.0,
        row_rect.y1 - 1.0,
    );
    state_layer::draw_bounded(
        draw,
        inset,
        0.0.into(),
        if selected {
            colors.on_secondary_container.peniko()
        } else {
            colors.on_surface.peniko()
        },
        state,
    );
}

pub fn draw_separator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    separator: vello::kurbo::Rect,
) {
    draw.fill_rect(separator, &Brush::from(colors.surface_variant.peniko()));
}

pub fn draw_radio_indicator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    radius: f64,
    selected: bool,
) {
    let outer_ring_center_radius = radius - PICKER_RADIO_OUTER_RING_WIDTH / 2.0;
    draw.stroke_circle(
        center,
        outer_ring_center_radius,
        &Brush::from(if selected {
            colors.primary.peniko()
        } else {
            colors.on_surface_variant.peniko()
        }),
        PICKER_RADIO_OUTER_RING_WIDTH,
    );
    if selected {
        draw.fill_circle(
            center,
            PICKER_RADIO_INNER_DOT_RADIUS,
            &Brush::from(colors.primary.peniko()),
        );
    }
}

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
    use vello::peniko::Color;

    use super::{
        MaterialColorScheme, draw_popup_row_background, draw_radio_indicator, draw_separator,
        material_metrics,
    };
    use crate::dimensions::{
        PICKER_LABEL_SPACING, PICKER_MENU_POPUP_CORNER_RADIUS, PICKER_MENU_POPUP_ROW_HEIGHT,
        PICKER_RADIO_INDICATOR_SIZE, PICKER_RADIO_INNER_DOT_RADIUS, PICKER_RADIO_OUTER_RING_WIDTH,
    };
    use crate::{Brush, DrawContext};

    #[derive(Default)]
    struct RecordingDrawContext {
        circle_fills: Vec<f64>,
        circle_strokes: Vec<(f64, f64)>,
        rect_fills: Vec<Color>,
    }

    impl DrawContext for RecordingDrawContext {
        fn fill_rect(&mut self, _rect: Rect, brush: &Brush) {
            let Brush::Solid(color) = brush else {
                panic!("Material picker token fills must be solid colors");
            };
            self.rect_fills.push(*color);
        }

        fn fill_rounded_rect(&mut self, _rect: Rect, _radii: RoundedRectRadii, _brush: &Brush) {}

        fn stroke_rect(&mut self, _rect: Rect, _brush: &Brush, _width: f64) {}

        fn stroke_rounded_rect(
            &mut self,
            _rect: Rect,
            _radii: RoundedRectRadii,
            _brush: &Brush,
            _width: f64,
        ) {
        }

        fn stroke_line(&mut self, _from: Point, _to: Point, _brush: &Brush, _width: f64) {}

        fn stroke_circle(&mut self, _center: Point, radius: f64, _brush: &Brush, width: f64) {
            self.circle_strokes.push((radius, width));
        }

        fn fill_circle(&mut self, _center: Point, radius: f64, _brush: &Brush) {
            self.circle_fills.push(radius);
        }

        fn fill_path(&mut self, _path: &BezPath, _brush: &Brush) {}

        fn stroke_path(&mut self, _path: &BezPath, _brush: &Brush, _width: f64) {}

        fn push_layer(&mut self, _alpha: f32, _clip: Option<&Rect>) {}

        fn pop_layer(&mut self) {}

        fn push_transform(&mut self, _affine: Affine) {}

        fn pop_transform(&mut self) {}
    }

    #[test]
    fn radio_metrics_match_material_web_latest_tokens() {
        let metrics = material_metrics();

        assert_eq!(metrics.radio_indicator_size, PICKER_RADIO_INDICATOR_SIZE);
        assert_eq!(metrics.label_spacing, PICKER_LABEL_SPACING);
        assert_eq!(PICKER_RADIO_INDICATOR_SIZE, 20.0);
        assert_eq!(PICKER_RADIO_OUTER_RING_WIDTH, 2.0);
        assert_eq!(PICKER_RADIO_INNER_DOT_RADIUS, 5.0);
        assert_eq!(metrics.popup_row_height, PICKER_MENU_POPUP_ROW_HEIGHT);
        assert_eq!(metrics.popup_corner_radius, PICKER_MENU_POPUP_CORNER_RADIUS);
        assert_eq!(PICKER_MENU_POPUP_ROW_HEIGHT, 48.0);
        assert_eq!(PICKER_MENU_POPUP_CORNER_RADIUS, 4.0);
    }

    #[test]
    fn radio_indicator_draws_material_web_donut_icon() {
        let colors = MaterialColorScheme::baseline_light();
        let center = Point::new(10.0, 10.0);

        let mut unselected = RecordingDrawContext::default();
        draw_radio_indicator(&colors, &mut unselected, center, 10.0, false);

        let mut selected = RecordingDrawContext::default();
        draw_radio_indicator(&colors, &mut selected, center, 10.0, true);

        assert_eq!(unselected.circle_fills, Vec::<f64>::new());
        assert_eq!(unselected.circle_strokes, vec![(9.0, 2.0)]);
        assert_eq!(selected.circle_strokes, vec![(9.0, 2.0)]);
        assert_eq!(selected.circle_fills, vec![5.0]);
    }

    #[test]
    fn menu_selected_row_and_divider_use_filled_select_tokens() {
        let colors = MaterialColorScheme::baseline_light();
        let mut draw = RecordingDrawContext::default();

        draw_popup_row_background(&colors, &mut draw, Rect::new(0.0, 0.0, 120.0, 48.0), true);
        draw_separator(&colors, &mut draw, Rect::new(0.0, 48.0, 120.0, 49.0));

        assert_eq!(
            draw.rect_fills,
            vec![
                colors.surface_container_highest.peniko(),
                colors.surface_variant.peniko(),
            ]
        );
    }
}

pub fn draw_radio_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    _radius: f64,
    selected: bool,
    state: WidgetInteractionState,
) {
    state_layer::draw_unbounded_circle(
        draw,
        center,
        20.0,
        if selected {
            colors.primary.peniko()
        } else {
            colors.on_surface.peniko()
        },
        state,
    );
}
