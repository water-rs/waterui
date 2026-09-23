use crate::dimensions::{
    SLIDER_HANDLE_HEIGHT, SLIDER_HANDLE_WIDTH, SLIDER_HORIZONTAL_INSET, SLIDER_HORIZONTAL_SPACING,
    SLIDER_MIN_TRACK_WIDTH, SLIDER_PRESSED_HANDLE_WIDTH, SLIDER_STATE_LAYER_RADIUS,
    SLIDER_TRACK_HEIGHT, SLIDER_VERTICAL_SPACING,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, SliderMetrics, WidgetInteractionState};

pub const fn metrics() -> SliderMetrics {
    SliderMetrics::new(
        SLIDER_HORIZONTAL_INSET,
        SLIDER_HORIZONTAL_SPACING,
        SLIDER_VERTICAL_SPACING,
        SLIDER_MIN_TRACK_WIDTH,
        SLIDER_TRACK_HEIGHT,
        SLIDER_HANDLE_HEIGHT / 2.0,
    )
}

pub fn draw_track(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    track_rect: vello::kurbo::Rect,
    fill_rect: vello::kurbo::Rect,
    state: WidgetInteractionState,
) {
    // MD3 disabled slider: the inactive track drops to on-surface at 12% and
    // the active track to on-surface at 38%.
    let (track_color, fill_color) = if state.disabled {
        (
            colors.on_surface.peniko_disabled_container(),
            colors.on_surface.peniko_disabled_content(),
        )
    } else {
        (colors.secondary_container.peniko(), colors.primary.peniko())
    };
    draw.fill_rounded_rect(
        track_rect,
        (SLIDER_TRACK_HEIGHT / 2.0).into(),
        &Brush::from(track_color),
    );
    draw.fill_rounded_rect(
        fill_rect,
        (SLIDER_TRACK_HEIGHT / 2.0).into(),
        &Brush::from(fill_color),
    );
}

pub fn draw_thumb(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    _radius: f64,
    state: WidgetInteractionState,
) {
    let width = if state.pressed || state.focus_visible {
        SLIDER_PRESSED_HANDLE_WIDTH
    } else {
        SLIDER_HANDLE_WIDTH
    };
    let bounds = vello::kurbo::Rect::from_center_size(center, (width, SLIDER_HANDLE_HEIGHT));
    // MD3 disabled slider handle: on-surface at 38% over an opaque surface
    // underlay, so content behind the semi-transparent handle cannot bleed
    // through (mdui paints the handle over the background role).
    if state.disabled {
        draw.fill_rounded_rect(
            bounds,
            (width / 2.0).into(),
            &Brush::from(colors.surface.peniko()),
        );
        draw.fill_rounded_rect(
            bounds,
            (width / 2.0).into(),
            &Brush::from(colors.on_surface.peniko_disabled_content()),
        );
        return;
    }
    draw.fill_rounded_rect(
        bounds,
        (width / 2.0).into(),
        &Brush::from(colors.primary.peniko()),
    );
}

pub fn draw_thumb_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    _radius: f64,
    state: WidgetInteractionState,
) {
    state_layer::draw_unbounded_circle(
        draw,
        center,
        SLIDER_STATE_LAYER_RADIUS,
        colors.primary.peniko(),
        state,
    );
}

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};

    use super::{MaterialColorScheme, WidgetInteractionState, draw_thumb, draw_track, metrics};
    use crate::dimensions::{
        SLIDER_HANDLE_HEIGHT, SLIDER_HANDLE_WIDTH, SLIDER_PRESSED_HANDLE_WIDTH,
        SLIDER_STATE_LAYER_RADIUS, SLIDER_TRACK_HEIGHT,
    };
    use crate::{Brush, DrawContext};

    #[derive(Default)]
    struct RecordingDrawContext {
        rounded_fills: Vec<(Rect, Brush)>,
        circle_fills: usize,
    }

    impl DrawContext for RecordingDrawContext {
        fn fill_rect(&mut self, _rect: Rect, _brush: &Brush) {}

        fn fill_rounded_rect(&mut self, rect: Rect, _radii: RoundedRectRadii, brush: &Brush) {
            self.rounded_fills.push((rect, brush.clone()));
        }

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

        fn stroke_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush, _width: f64) {}

        fn fill_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush) {
            self.circle_fills += 1;
        }

        fn fill_path(&mut self, _path: &BezPath, _brush: &Brush) {}

        fn stroke_path(&mut self, _path: &BezPath, _brush: &Brush, _width: f64) {}

        fn push_layer(&mut self, _alpha: f32, _clip: Option<&Rect>) {}

        fn pop_layer(&mut self) {}

        fn push_transform(&mut self, _affine: Affine) {}

        fn pop_transform(&mut self) {}
    }

    #[test]
    fn slider_metrics_match_material_web_latest_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.track_height, SLIDER_TRACK_HEIGHT);
        assert_eq!(metrics.thumb_radius, SLIDER_HANDLE_HEIGHT / 2.0);
        assert_eq!(SLIDER_TRACK_HEIGHT, 16.0);
        assert_eq!(SLIDER_HANDLE_WIDTH, 4.0);
        assert_eq!(SLIDER_HANDLE_HEIGHT, 44.0);
        assert_eq!(SLIDER_STATE_LAYER_RADIUS, 20.0);
    }

    #[test]
    fn slider_thumb_draws_material_vertical_handle() {
        let mut draw = RecordingDrawContext::default();
        draw_thumb(
            &MaterialColorScheme::baseline_light(),
            &mut draw,
            Point::new(64.0, 48.0),
            SLIDER_HANDLE_HEIGHT / 2.0,
            WidgetInteractionState::NONE,
        );

        assert_eq!(draw.circle_fills, 0);
        assert_eq!(draw.rounded_fills.len(), 1);
        assert_eq!(draw.rounded_fills[0].0.width(), SLIDER_HANDLE_WIDTH);
        assert_eq!(draw.rounded_fills[0].0.height(), SLIDER_HANDLE_HEIGHT);
    }

    #[test]
    fn slider_pressed_thumb_uses_material_narrow_handle() {
        let mut draw = RecordingDrawContext::default();
        draw_thumb(
            &MaterialColorScheme::baseline_light(),
            &mut draw,
            Point::new(64.0, 48.0),
            SLIDER_HANDLE_HEIGHT / 2.0,
            WidgetInteractionState {
                pressed: true,
                ..WidgetInteractionState::NONE
            },
        );

        assert_eq!(draw.rounded_fills.len(), 1);
        assert_eq!(draw.rounded_fills[0].0.width(), SLIDER_PRESSED_HANDLE_WIDTH);
        assert_eq!(draw.rounded_fills[0].0.height(), SLIDER_HANDLE_HEIGHT);
    }

    #[test]
    fn disabled_slider_uses_disabled_palette() {
        // MD3 disabled slider: inactive track on-surface at 12%, active track
        // on-surface at 38%, handle on-surface at 38% over an opaque surface
        // underlay.
        let colors = MaterialColorScheme::baseline_light();
        let disabled = WidgetInteractionState {
            disabled: true,
            ..WidgetInteractionState::NONE
        };

        let mut track = RecordingDrawContext::default();
        draw_track(
            &colors,
            &mut track,
            Rect::new(0.0, 0.0, 120.0, SLIDER_TRACK_HEIGHT),
            Rect::new(0.0, 0.0, 72.0, SLIDER_TRACK_HEIGHT),
            disabled,
        );
        assert!(matches!(
            &track.rounded_fills[0].1,
            Brush::Solid(color) if *color == colors.on_surface.peniko_disabled_container()
        ));
        assert!(matches!(
            &track.rounded_fills[1].1,
            Brush::Solid(color) if *color == colors.on_surface.peniko_disabled_content()
        ));

        let mut thumb = RecordingDrawContext::default();
        draw_thumb(
            &colors,
            &mut thumb,
            Point::new(64.0, 48.0),
            SLIDER_HANDLE_HEIGHT / 2.0,
            disabled,
        );
        assert_eq!(
            thumb.rounded_fills.len(),
            2,
            "surface underlay then 38% handle"
        );
        assert!(matches!(
            &thumb.rounded_fills[0].1,
            Brush::Solid(color) if *color == colors.surface.peniko()
        ));
        assert!(matches!(
            &thumb.rounded_fills[1].1,
            Brush::Solid(color) if *color == colors.on_surface.peniko_disabled_content()
        ));
    }

    #[test]
    fn slider_track_uses_material_role_colors() {
        let colors = MaterialColorScheme::baseline_light();
        let mut draw = RecordingDrawContext::default();
        draw_track(
            &colors,
            &mut draw,
            Rect::new(0.0, 0.0, 120.0, SLIDER_TRACK_HEIGHT),
            Rect::new(0.0, 0.0, 72.0, SLIDER_TRACK_HEIGHT),
            WidgetInteractionState::NONE,
        );

        assert_eq!(draw.rounded_fills.len(), 2);
        assert!(matches!(
            &draw.rounded_fills[0].1,
            Brush::Solid(color) if *color == colors.secondary_container.peniko()
        ));
        assert!(matches!(
            &draw.rounded_fills[1].1,
            Brush::Solid(color) if *color == colors.primary.peniko()
        ));
    }
}
