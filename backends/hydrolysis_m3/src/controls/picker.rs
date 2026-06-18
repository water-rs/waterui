#![allow(clippy::cast_precision_loss, reason = "intentional lossy numeric cast in rendering/layout code")]
use crate::dimensions::{
    PICKER_HORIZONTAL_INSET, PICKER_INDICATOR_SPACE, PICKER_LABEL_SPACING,
    PICKER_MENU_POPUP_CORNER_RADIUS, PICKER_MENU_POPUP_ROW_HEIGHT, PICKER_MENU_POPUP_TOP_SPACING,
    PICKER_MIN_HEIGHT, PICKER_MIN_WIDTH, PICKER_RADIO_INDICATOR_SIZE,
    PICKER_RADIO_INNER_DOT_RADIUS, PICKER_RADIO_LABEL_SPACING, PICKER_RADIO_OUTER_RING_WIDTH,
    PICKER_RADIO_ROW_SPACING, PICKER_SEGMENTED_CONTAINER_RADIUS, PICKER_SEGMENTED_HORIZONTAL_INSET,
    PICKER_SEGMENTED_MIN_HEIGHT, PICKER_SEGMENTED_OUTLINE_WIDTH, PICKER_VERTICAL_INSET,
};
use crate::theme::colors::{MaterialColorScheme, MaterialRoleColor};
use crate::theme::state_layer;
use crate::{Brush, DrawContext, PickerMetrics, RadioIndicatorState, WidgetInteractionState};
use waterui_form::picker::PickerStyle;

pub fn metrics(style: PickerStyle) -> PickerMetrics {
    match style {
        PickerStyle::Automatic | PickerStyle::Menu | PickerStyle::Radio => material_metrics(),
        PickerStyle::Segmented => segmented_metrics(),
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

const fn segmented_metrics() -> PickerMetrics {
    PickerMetrics {
        min_width: PICKER_MIN_WIDTH,
        min_height: PICKER_SEGMENTED_MIN_HEIGHT,
        horizontal_inset: PICKER_SEGMENTED_HORIZONTAL_INSET,
        vertical_inset: 0.0,
        label_spacing: PICKER_LABEL_SPACING,
        indicator_space: 0.0,
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
    state: RadioIndicatorState,
) {
    let outer_selected_progress = state.outer_selected_progress.clamp(0.0, 1.0);
    let inner_scale = state.inner_scale.clamp(0.0, 1.0);
    let inner_opacity = state.inner_opacity.clamp(0.0, 1.0);
    let outer_ring_center_radius = radius - PICKER_RADIO_OUTER_RING_WIDTH / 2.0;
    draw.stroke_circle(
        center,
        outer_ring_center_radius,
        &Brush::from(blend_role_color(
            colors.on_surface_variant,
            colors.primary,
            outer_selected_progress,
        )),
        PICKER_RADIO_OUTER_RING_WIDTH,
    );
    let inner_radius = PICKER_RADIO_INNER_DOT_RADIUS * f64::from(inner_scale);
    if inner_radius > 0.0 && inner_opacity > 0.0 {
        draw.fill_circle(
            center,
            inner_radius,
            &Brush::from(colors.primary.peniko().with_alpha(inner_opacity)),
        );
    }
}

fn blend_role_color(
    from: MaterialRoleColor,
    to: MaterialRoleColor,
    progress: f32,
) -> vello::peniko::Color {
    let progress = progress.clamp(0.0, 1.0);
    let from = from.argb();
    let to = to.argb();
    vello::peniko::Color::new([
        blend_channel(from.red(), to.red(), progress),
        blend_channel(from.green(), to.green(), progress),
        blend_channel(from.blue(), to.blue(), progress),
        blend_channel(from.alpha(), to.alpha(), progress),
    ])
}

fn blend_channel(from: u8, to: u8, progress: f32) -> f32 {
    let from = f32::from(from) / 255.0;
    let to = f32::from(to) / 255.0;
    from.mul_add(1.0 - progress, to * progress)
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

pub fn segmented_label_color(
    colors: &MaterialColorScheme,
    selected: bool,
) -> waterui_graphics::color::Color {
    if selected {
        colors.on_secondary_container.view_color()
    } else {
        colors.on_surface.view_color()
    }
}

pub fn draw_segmented_container(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    segment_count: usize,
) {
    draw.stroke_rounded_rect(
        bounds,
        PICKER_SEGMENTED_CONTAINER_RADIUS.into(),
        &Brush::from(colors.outline.peniko()),
        PICKER_SEGMENTED_OUTLINE_WIDTH,
    );
    if segment_count <= 1 {
        return;
    }
    let segment_width = bounds.width() / segment_count as f64;
    for index in 1..segment_count {
        let x = segment_width.mul_add(index as f64, bounds.x0);
        draw.stroke_line(
            vello::kurbo::Point::new(x, bounds.y0),
            vello::kurbo::Point::new(x, bounds.y1),
            &Brush::from(colors.outline.peniko()),
            PICKER_SEGMENTED_OUTLINE_WIDTH,
        );
    }
}

pub fn draw_segmented_segment(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    selected: bool,
    is_first: bool,
    is_last: bool,
) {
    if !selected {
        return;
    }
    if is_first && is_last {
        draw.fill_rounded_rect(
            bounds,
            PICKER_SEGMENTED_CONTAINER_RADIUS.into(),
            &Brush::from(colors.secondary_container.peniko()),
        );
        return;
    }
    draw.fill_rect(bounds, &Brush::from(colors.secondary_container.peniko()));
}

pub fn draw_segmented_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    selected: bool,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(
        draw,
        bounds,
        PICKER_SEGMENTED_CONTAINER_RADIUS.into(),
        if selected {
            colors.on_secondary_container.peniko()
        } else {
            colors.on_surface.peniko()
        },
        state,
    );
}

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
    use vello::peniko::Color;

    use super::{
        MaterialColorScheme, RadioIndicatorState, blend_role_color, draw_popup_row_background,
        draw_radio_indicator, draw_segmented_container, draw_segmented_segment, draw_separator,
        material_metrics, segmented_metrics,
    };
    use crate::dimensions::{
        PICKER_LABEL_SPACING, PICKER_MENU_POPUP_CORNER_RADIUS, PICKER_MENU_POPUP_ROW_HEIGHT,
        PICKER_RADIO_INDICATOR_SIZE, PICKER_RADIO_INNER_DOT_RADIUS, PICKER_RADIO_OUTER_RING_WIDTH,
        PICKER_SEGMENTED_CONTAINER_RADIUS, PICKER_SEGMENTED_HORIZONTAL_INSET,
        PICKER_SEGMENTED_MIN_HEIGHT,
    };
    use crate::{Brush, DrawContext};

    #[derive(Default)]
    struct RecordingDrawContext {
        circle_fills: Vec<(f64, Color)>,
        circle_strokes: Vec<(f64, f64, Color)>,
        rect_fills: Vec<Color>,
        rounded_strokes: Vec<(RoundedRectRadii, Color, f64)>,
        line_strokes: Vec<(Color, f64)>,
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
            radii: RoundedRectRadii,
            brush: &Brush,
            width: f64,
        ) {
            let Brush::Solid(color) = brush else {
                panic!("Material picker token strokes must be solid colors");
            };
            self.rounded_strokes.push((radii, *color, width));
        }

        fn stroke_line(&mut self, _from: Point, _to: Point, brush: &Brush, width: f64) {
            let Brush::Solid(color) = brush else {
                panic!("Material picker token strokes must be solid colors");
            };
            self.line_strokes.push((*color, width));
        }

        fn stroke_circle(&mut self, _center: Point, radius: f64, brush: &Brush, width: f64) {
            let Brush::Solid(color) = brush else {
                panic!("Material picker token circle strokes must be solid colors");
            };
            self.circle_strokes.push((radius, width, *color));
        }

        fn fill_circle(&mut self, _center: Point, radius: f64, brush: &Brush) {
            let Brush::Solid(color) = brush else {
                panic!("Material picker token circle fills must be solid colors");
            };
            self.circle_fills.push((radius, *color));
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
        draw_radio_indicator(
            &colors,
            &mut unselected,
            center,
            10.0,
            RadioIndicatorState {
                selected: false,
                outer_selected_progress: 0.0,
                inner_scale: 1.0,
                inner_opacity: 0.0,
            },
        );

        let mut selected = RecordingDrawContext::default();
        draw_radio_indicator(
            &colors,
            &mut selected,
            center,
            10.0,
            RadioIndicatorState {
                selected: true,
                outer_selected_progress: 1.0,
                inner_scale: 1.0,
                inner_opacity: 1.0,
            },
        );

        assert_eq!(unselected.circle_fills, Vec::<(f64, Color)>::new());
        assert_eq!(
            unselected.circle_strokes,
            vec![(9.0, 2.0, colors.on_surface_variant.peniko())]
        );
        assert_eq!(
            selected.circle_strokes,
            vec![(9.0, 2.0, colors.primary.peniko())]
        );
        assert_eq!(selected.circle_fills, vec![(5.0, colors.primary.peniko())]);
    }

    #[test]
    fn radio_indicator_inner_dot_uses_material_web_scale_and_opacity() {
        let colors = MaterialColorScheme::baseline_light();
        let center = Point::new(10.0, 10.0);
        let mut draw = RecordingDrawContext::default();

        draw_radio_indicator(
            &colors,
            &mut draw,
            center,
            10.0,
            RadioIndicatorState {
                selected: true,
                outer_selected_progress: 1.0,
                inner_scale: 0.4,
                inner_opacity: 0.25,
            },
        );

        assert_eq!(draw.circle_fills.len(), 1);
        assert!((draw.circle_fills[0].0 - 2.0).abs() < 0.000_001);
        assert_eq!(
            draw.circle_fills[0].1,
            colors.primary.peniko().with_alpha(0.25)
        );
    }

    #[test]
    fn radio_indicator_outer_ring_color_interpolates_with_material_web_transition() {
        let colors = MaterialColorScheme::baseline_light();
        let center = Point::new(10.0, 10.0);
        let mut draw = RecordingDrawContext::default();

        draw_radio_indicator(
            &colors,
            &mut draw,
            center,
            10.0,
            RadioIndicatorState {
                selected: true,
                outer_selected_progress: 0.5,
                inner_scale: 0.0,
                inner_opacity: 0.0,
            },
        );

        assert_eq!(
            draw.circle_strokes,
            vec![(
                9.0,
                2.0,
                blend_role_color(colors.on_surface_variant, colors.primary, 0.5)
            )]
        );
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

    #[test]
    fn segmented_metrics_match_material_web_latest_tokens() {
        let metrics = segmented_metrics();

        assert_eq!(metrics.min_height, PICKER_SEGMENTED_MIN_HEIGHT);
        assert_eq!(metrics.horizontal_inset, PICKER_SEGMENTED_HORIZONTAL_INSET);
        assert_eq!(PICKER_SEGMENTED_MIN_HEIGHT, 40.0);
        assert_eq!(PICKER_SEGMENTED_CONTAINER_RADIUS, 20.0);
    }

    #[test]
    fn segmented_container_and_selected_segment_use_material_tokens() {
        let colors = MaterialColorScheme::baseline_light();
        let mut draw = RecordingDrawContext::default();

        draw_segmented_segment(
            &colors,
            &mut draw,
            Rect::new(0.0, 0.0, 80.0, 40.0),
            true,
            false,
            false,
        );
        draw_segmented_container(&colors, &mut draw, Rect::new(0.0, 0.0, 240.0, 40.0), 3);

        assert_eq!(draw.rect_fills, vec![colors.secondary_container.peniko()]);
        assert_eq!(draw.rounded_strokes.len(), 1);
        assert_eq!(draw.rounded_strokes[0].1, colors.outline.peniko());
        assert_eq!(draw.rounded_strokes[0].2, 1.0);
        assert_eq!(
            draw.line_strokes,
            vec![
                (colors.outline.peniko(), 1.0),
                (colors.outline.peniko(), 1.0)
            ]
        );
    }
}
