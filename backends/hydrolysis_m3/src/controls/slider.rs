use crate::dimensions::{
    SLIDER_HANDLE_HEIGHT, SLIDER_HANDLE_PADDING, SLIDER_HANDLE_WIDTH, SLIDER_HORIZONTAL_INSET,
    SLIDER_HORIZONTAL_SPACING, SLIDER_MIN_TRACK_WIDTH, SLIDER_PRESSED_HANDLE_WIDTH,
    SLIDER_STOP_INDICATOR_SIZE, SLIDER_TRACK_HEIGHT, SLIDER_VERTICAL_SPACING,
};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, SliderMetrics, WidgetInteractionState};

pub const fn metrics() -> SliderMetrics {
    SliderMetrics::new(
        SLIDER_HORIZONTAL_INSET,
        SLIDER_HORIZONTAL_SPACING,
        SLIDER_VERTICAL_SPACING,
        SLIDER_MIN_TRACK_WIDTH,
        SLIDER_TRACK_HEIGHT,
        SLIDER_HANDLE_WIDTH,
        SLIDER_HANDLE_HEIGHT,
    )
}

/// Draws the Expressive slider track: two bars separated by a gap around the
/// handle, with a stop indicator marking the far end.
///
/// Earlier Material ran a single hairline under a circular thumb. Expressive
/// splits the track at the handle — `ActiveHandlePadding` of clear space on each
/// side — so the handle reads as sitting *in* the track rather than on top of it.
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
        (
            colors.surface_container_highest.peniko(),
            colors.primary.peniko(),
        )
    };
    let radius = (SLIDER_TRACK_HEIGHT / 2.0).into();

    // Inactive remainder, starting clear of the handle.
    let inactive_start = (fill_rect.x1 + SLIDER_HANDLE_PADDING).min(track_rect.x1);
    if inactive_start < track_rect.x1 {
        draw.fill_rounded_rect(
            vello::kurbo::Rect::new(inactive_start, track_rect.y0, track_rect.x1, track_rect.y1),
            radius,
            &Brush::from(track_color),
        );
    }

    // Active portion, stopping clear of the handle.
    let active_end = (fill_rect.x1 - SLIDER_HANDLE_PADDING).max(track_rect.x0);
    if active_end > track_rect.x0 {
        draw.fill_rounded_rect(
            vello::kurbo::Rect::new(track_rect.x0, track_rect.y0, active_end, track_rect.y1),
            radius,
            &Brush::from(fill_color),
        );
    }

    // Stop indicator: a dot at the track's far end, hidden once the handle
    // reaches it so the two never overlap.
    let indicator_center = vello::kurbo::Point::new(
        track_rect.x1 - SLIDER_HANDLE_PADDING - SLIDER_STOP_INDICATOR_SIZE / 2.0,
        track_rect.y0 + track_rect.height() / 2.0,
    );
    if indicator_center.x > inactive_start {
        draw.fill_circle(
            indicator_center,
            SLIDER_STOP_INDICATOR_SIZE / 2.0,
            &Brush::from(if state.disabled {
                colors.on_surface.peniko_disabled_content()
            } else {
                colors.on_secondary_container.peniko()
            }),
        );
    }
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
            &Brush::from(colors.background.peniko()),
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

/// The Expressive slider handle has no state layer.
///
/// `SliderTokens` carries no `StateLayerSize`: the feedback is the handle
/// itself narrowing from `HandleWidth` to `PressedHandleWidth` under the
/// finger. Drawing a ripple here would put a circle around a bar that Material
/// never shows.
pub const fn draw_thumb_state_layer(
    _colors: &MaterialColorScheme,
    _draw: &mut dyn DrawContext,
    _center: vello::kurbo::Point,
    _radius: f64,
    _state: WidgetInteractionState,
) {
}

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};

    use super::{MaterialColorScheme, WidgetInteractionState, draw_thumb, draw_track, metrics};
    use crate::dimensions::{
        SLIDER_HANDLE_HEIGHT, SLIDER_HANDLE_PADDING, SLIDER_HANDLE_WIDTH,
        SLIDER_PRESSED_HANDLE_WIDTH, SLIDER_STOP_INDICATOR_SIZE, SLIDER_TRACK_HEIGHT,
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

    /// Values from `androidx.compose.material3.tokens.SliderTokens`.
    #[test]
    fn slider_metrics_match_compose_slider_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.track_height, SLIDER_TRACK_HEIGHT);
        assert_eq!(metrics.handle_width, SLIDER_HANDLE_WIDTH);
        assert_eq!(metrics.handle_height, SLIDER_HANDLE_HEIGHT);
        // InactiveTrackHeight / ActiveTrackHeight
        assert_eq!(SLIDER_TRACK_HEIGHT, 16.0);
        // HandleWidth / HandleHeight
        assert_eq!(SLIDER_HANDLE_WIDTH, 4.0);
        assert_eq!(SLIDER_HANDLE_HEIGHT, 44.0);
        // PressedHandleWidth
        assert_eq!(SLIDER_PRESSED_HANDLE_WIDTH, 2.0);
        // ActiveHandlePadding
        assert_eq!(SLIDER_HANDLE_PADDING, 6.0);
        // StopIndicatorSize
        assert_eq!(SLIDER_STOP_INDICATOR_SIZE, 4.0);
    }

    /// The Expressive handle is a bar: 4dp wide and 44dp tall, not a circle.
    #[test]
    fn slider_thumb_draws_the_expressive_bar_handle() {
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

    /// Pressing narrows the handle rather than growing a state layer.
    #[test]
    fn slider_pressed_thumb_narrows() {
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
            Brush::Solid(color) if *color == colors.background.peniko()
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
            Brush::Solid(color) if *color == colors.surface_container_highest.peniko()
        ));
        assert!(matches!(
            &draw.rounded_fills[1].1,
            Brush::Solid(color) if *color == colors.primary.peniko()
        ));
        // The track is split around the handle, so the active bar stops short
        // of the value position by one `ActiveHandlePadding`.
        assert_eq!(draw.rounded_fills[1].0.x1, 72.0 - SLIDER_HANDLE_PADDING);
        assert_eq!(draw.rounded_fills[0].0.x0, 72.0 + SLIDER_HANDLE_PADDING);
    }
}
