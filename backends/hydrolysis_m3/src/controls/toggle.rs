use crate::dimensions::{
    TOGGLE_CHECKBOX_CONTAINER_SHAPE, TOGGLE_CHECKBOX_OUTLINE_WIDTH,
    TOGGLE_CHECKBOX_SELECTED_SCALE_START, TOGGLE_CHECKBOX_SIZE, TOGGLE_LABEL_SPACING,
    TOGGLE_SWITCH_HEIGHT, TOGGLE_SWITCH_ICON_SCALE_START, TOGGLE_SWITCH_ICON_SIZE,
    TOGGLE_SWITCH_OUTLINE_WIDTH, TOGGLE_SWITCH_PRESSED_HANDLE_SIZE,
    TOGGLE_SWITCH_SELECTED_HANDLE_SIZE, TOGGLE_SWITCH_THUMB_SELECTED_INSET,
    TOGGLE_SWITCH_THUMB_UNSELECTED_INSET, TOGGLE_SWITCH_UNSELECTED_HANDLE_SIZE,
    TOGGLE_SWITCH_WIDTH,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, ToggleMetrics, WidgetInteractionState, lerp_color};
use vello::kurbo::{Affine, BezPath, PathEl, Point, Rect};
use waterui_controls::toggle::ToggleStyle;

pub fn metrics(style: ToggleStyle) -> ToggleMetrics {
    match style {
        ToggleStyle::Automatic | ToggleStyle::Switch => ToggleMetrics::new(
            TOGGLE_SWITCH_WIDTH,
            TOGGLE_SWITCH_HEIGHT,
            TOGGLE_LABEL_SPACING,
        ),
        ToggleStyle::Checkbox => ToggleMetrics::new(
            TOGGLE_CHECKBOX_SIZE,
            TOGGLE_CHECKBOX_SIZE,
            TOGGLE_LABEL_SPACING,
        ),
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }
}

/// The switch thumb's center for the given animated `progress`: it slides
/// between the mdui resting centers — 14dp from the left edge unselected
/// (`left: 6` + half of the 16dp thumb) and 16dp from the right edge selected
/// (`left: 24` + half of the 24dp thumb in the 52dp track). Pressing grows the
/// thumb around this center without moving it.
fn switch_thumb_center(bounds: Rect, progress: f32) -> Point {
    let unselected_x = bounds.x0
        + TOGGLE_SWITCH_THUMB_UNSELECTED_INSET
        + TOGGLE_SWITCH_UNSELECTED_HANDLE_SIZE / 2.0;
    let selected_x = bounds.x1
        - (TOGGLE_SWITCH_THUMB_SELECTED_INSET + TOGGLE_SWITCH_SELECTED_HANDLE_SIZE / 2.0);
    Point::new(
        crate::lerp_f64(unselected_x, selected_x, progress),
        bounds.y0 + bounds.height() / 2.0,
    )
}

/// The thumb icon's opacity for one frame, per mdui's staged linear
/// transitions over the 200ms value animation: entering waits 50ms then ramps
/// over 150ms (progress 0.25→1.0); exiting ramps out during the first 50ms
/// (progress 1.0→0.75).
fn switch_icon_opacity(progress: f32, selected: bool) -> f32 {
    if selected {
        ((progress - 0.25) / 0.75).clamp(0.0, 1.0)
    } else {
        ((progress - 0.75) / 0.25).clamp(0.0, 1.0)
    }
}

pub fn draw_switch(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    progress: f32,
    selected: bool,
    state: WidgetInteractionState,
) {
    let progress = progress.clamp(0.0, 1.0);
    let track_color = lerp_color(
        colors.surface_container_highest.peniko(),
        colors.primary.peniko(),
        progress,
    );
    let handle_size = if state.pressed {
        TOGGLE_SWITCH_PRESSED_HANDLE_SIZE
    } else {
        crate::lerp_f64(
            TOGGLE_SWITCH_UNSELECTED_HANDLE_SIZE,
            TOGGLE_SWITCH_SELECTED_HANDLE_SIZE,
            progress,
        )
    };
    let handle_radius = handle_size / 2.0;
    let thumb_center = switch_thumb_center(bounds, progress);
    let track_radius = bounds.height() / 2.0;
    draw.fill_rounded_rect(bounds, track_radius.into(), &Brush::from(track_color));
    // mdui animates the unselected outline's border-width from 2dp to 0 (not
    // its opacity), with the border inside the 52x32 box (border-box sizing).
    let outline_width = TOGGLE_SWITCH_OUTLINE_WIDTH * f64::from(1.0 - progress);
    if outline_width > 0.01 {
        let inset = outline_width / 2.0;
        let outline_bounds = bounds.inflate(-inset, -inset);
        draw.stroke_rounded_rect(
            outline_bounds,
            (track_radius - inset).into(),
            &Brush::from(colors.outline.peniko()),
            outline_width,
        );
    }
    let thumb_color = if state.pressed || state.hovered || state.focus_visible {
        if selected {
            colors.primary_container.peniko()
        } else {
            colors.on_surface_variant.peniko()
        }
    } else {
        lerp_color(
            colors.outline.peniko(),
            colors.on_primary.peniko(),
            progress,
        )
    };
    draw.fill_circle(thumb_center, handle_radius, &Brush::from(thumb_color));

    // mdui's default checked icon: a 16dp checkmark on the thumb, colored
    // on-primary-container, scaling in from 0.92 as it fades.
    let icon_opacity = switch_icon_opacity(progress, selected);
    if icon_opacity > 0.0 {
        let icon_scale = crate::lerp_f64(TOGGLE_SWITCH_ICON_SCALE_START, 1.0, icon_opacity);
        let icon_bounds = Rect::from_center_size(
            thumb_center,
            (
                TOGGLE_SWITCH_ICON_SIZE * icon_scale,
                TOGGLE_SWITCH_ICON_SIZE * icon_scale,
            ),
        );
        draw.stroke_path(
            &check_glyph_path(icon_bounds),
            &Brush::from(
                colors
                    .on_primary_container
                    .peniko()
                    .with_alpha(icon_opacity),
            ),
            TOGGLE_CHECKBOX_OUTLINE_WIDTH,
        );
    }
}

pub fn draw_switch_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    progress: f32,
    selected: bool,
    state: WidgetInteractionState,
) {
    let center = switch_thumb_center(bounds, progress);
    // mdui switches the thumb's state-layer color with the checked attribute.
    let color = if selected {
        colors.primary.peniko()
    } else {
        colors.on_surface.peniko()
    };
    state_layer::draw_unbounded_circle(draw, center, 20.0, color, state);
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
    draw.stroke_path(
        &check_glyph_path(bounds),
        &Brush::from(colors.on_primary.peniko().with_alpha(progress)),
        TOGGLE_CHECKBOX_OUTLINE_WIDTH,
    );
    draw.pop_transform();
}

/// The Material `check` glyph as a stroked path filling `bounds`.
fn check_glyph_path(bounds: Rect) -> BezPath {
    BezPath::from_vec(vec![
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
    ])
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

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};

    use super::{
        MaterialColorScheme, WidgetInteractionState, draw_checkbox, draw_switch,
        switch_icon_opacity,
    };
    use crate::dimensions::{TOGGLE_LABEL_SPACING, TOGGLE_SWITCH_PRESSED_HANDLE_SIZE};
    use crate::{Brush, DrawContext};

    #[derive(Default)]
    struct RecordingDrawContext {
        rounded_stroke_count: usize,
        rounded_stroke_widths: Vec<f64>,
        rounded_fill_count: usize,
        path_stroke_count: usize,
        circle_centers: Vec<Point>,
        circle_radii: Vec<f64>,
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
            width: f64,
        ) {
            self.rounded_stroke_count += 1;
            self.rounded_stroke_widths.push(width);
        }

        fn stroke_line(&mut self, _from: Point, _to: Point, _brush: &Brush, _width: f64) {}

        fn stroke_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush, _width: f64) {}

        fn fill_circle(&mut self, center: Point, radius: f64, _brush: &Brush) {
            self.circle_centers.push(center);
            self.circle_radii.push(radius);
        }

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
        draw_switch(
            &colors,
            &mut unselected,
            bounds,
            0.0,
            false,
            WidgetInteractionState::NONE,
        );

        let mut selected = RecordingDrawContext::default();
        draw_switch(
            &colors,
            &mut selected,
            bounds,
            1.0,
            true,
            WidgetInteractionState::NONE,
        );

        assert_eq!(unselected.rounded_stroke_count, 1);
        assert_eq!(selected.rounded_stroke_count, 0);
    }

    #[test]
    fn switch_outline_width_shrinks_with_progress() {
        // mdui animates border-width 2dp → 0 with the value transition, not
        // the outline's opacity.
        let colors = MaterialColorScheme::baseline_light();
        let bounds = Rect::from_origin_size((0.0, 0.0), (52.0, 32.0));

        let mut mid = RecordingDrawContext::default();
        draw_switch(&colors, &mut mid, bounds, 0.5, true, WidgetInteractionState::NONE);

        assert_eq!(mid.rounded_stroke_widths, vec![1.0]);
    }

    #[test]
    fn switch_thumb_rests_at_mdui_centers() {
        // Unselected: left 6 + 16dp thumb → center 14dp. Selected: left 24 +
        // 24dp thumb → center 36dp (16dp from the right edge of the 52 track).
        let colors = MaterialColorScheme::baseline_light();
        let bounds = Rect::from_origin_size((0.0, 0.0), (52.0, 32.0));

        let mut unselected = RecordingDrawContext::default();
        draw_switch(
            &colors,
            &mut unselected,
            bounds,
            0.0,
            false,
            WidgetInteractionState::NONE,
        );
        assert_eq!(unselected.circle_centers, vec![Point::new(14.0, 16.0)]);

        let mut selected = RecordingDrawContext::default();
        draw_switch(
            &colors,
            &mut selected,
            bounds,
            1.0,
            true,
            WidgetInteractionState::NONE,
        );
        assert_eq!(selected.circle_centers, vec![Point::new(36.0, 16.0)]);
    }

    #[test]
    fn switch_checked_icon_stages_with_mdui_timing() {
        // Entering: 50ms delay then a 150ms linear ramp over the 200ms value
        // animation. Exiting: gone within the first 50ms.
        assert_eq!(switch_icon_opacity(0.25, true), 0.0);
        assert!((switch_icon_opacity(0.625, true) - 0.5).abs() < 1e-6);
        assert_eq!(switch_icon_opacity(1.0, true), 1.0);
        assert!((switch_icon_opacity(0.9, false) - 0.6).abs() < 1e-6);
        assert_eq!(switch_icon_opacity(0.75, false), 0.0);
        assert_eq!(switch_icon_opacity(0.2, false), 0.0);
    }

    #[test]
    fn selected_switch_draws_checkmark_icon_on_thumb() {
        let colors = MaterialColorScheme::baseline_light();
        let bounds = Rect::from_origin_size((0.0, 0.0), (52.0, 32.0));

        let mut selected = RecordingDrawContext::default();
        draw_switch(
            &colors,
            &mut selected,
            bounds,
            1.0,
            true,
            WidgetInteractionState::NONE,
        );
        assert_eq!(selected.path_stroke_count, 1, "checked thumb shows the check glyph");

        let mut unselected = RecordingDrawContext::default();
        draw_switch(
            &colors,
            &mut unselected,
            bounds,
            0.0,
            false,
            WidgetInteractionState::NONE,
        );
        assert_eq!(unselected.path_stroke_count, 0, "unselected thumb has no icon");
    }

    #[test]
    fn toggle_metrics_include_material_label_spacing() {
        let metrics = super::metrics(waterui_controls::toggle::ToggleStyle::Switch);

        assert_eq!(metrics.label_spacing, TOGGLE_LABEL_SPACING);
        assert_eq!(TOGGLE_LABEL_SPACING, 8.0);
    }

    #[test]
    fn pressed_material_switch_uses_large_handle() {
        let colors = MaterialColorScheme::baseline_light();
        let bounds = Rect::from_origin_size((0.0, 0.0), (52.0, 32.0));
        let mut draw = RecordingDrawContext::default();

        draw_switch(
            &colors,
            &mut draw,
            bounds,
            0.0,
            false,
            WidgetInteractionState {
                pressed: true,
                ..WidgetInteractionState::NONE
            },
        );

        assert_eq!(
            draw.circle_radii,
            vec![TOGGLE_SWITCH_PRESSED_HANDLE_SIZE / 2.0]
        );
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
