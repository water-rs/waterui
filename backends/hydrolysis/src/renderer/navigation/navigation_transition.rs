use waterui::navigation::{
    AnyNavigationTransition, NavigationTransitionDirection,
    NavigationTransitionFrame as ResolvedNavigationTransitionFrame, NavigationTransitionLayer,
    RetainedNavigationTransition,
};
use waterui_backend_core::widget::NavigationMotion;

/// How a transition presents its two pages this frame.
pub enum NavigationPresentation {
    /// Both pages cross-fade while the matched element pair flies between
    /// its two frames.
    Matched(waterui_core::id::Id),
    /// Each page draws under its own transition layer.
    Layers(ResolvedNavigationTransitionFrame),
}

pub fn navigation_presentation(
    style: &AnyNavigationTransition,
    motion: NavigationMotion,
    direction: NavigationTransitionDirection,
    progress: f64,
    viewport_width: f64,
) -> NavigationPresentation {
    let progress = crate::num_cast::f64_as_f32(progress);
    let resolved = match style.retained() {
        RetainedNavigationTransition::MatchedGeometry(id) => {
            return NavigationPresentation::Matched(id);
        }
        RetainedNavigationTransition::PlatformDefault => material_shared_axis_x_frame(
            progress,
            direction,
            motion.shared_axis_slide_distance,
            viewport_width,
            motion.fade_through_threshold,
        ),
        RetainedNavigationTransition::None => ResolvedNavigationTransitionFrame::IDENTITY,
        RetainedNavigationTransition::Frames => style.frame(progress, direction),
    };
    NavigationPresentation::Layers(resolved)
}

/// A transition layer's transform of its page: the offset as a fraction of
/// `paint_bounds`, then the scale about the centre of `bounds`.
pub fn transition_layer_transform(
    bounds: kurbo::Rect,
    paint_bounds: kurbo::Rect,
    layer: NavigationTransitionLayer,
) -> kurbo::Affine {
    let center = bounds.center();
    kurbo::Affine::translate((
        f64::from(layer.offset_x) * paint_bounds.width(),
        f64::from(layer.offset_y) * paint_bounds.height(),
    )) * kurbo::Affine::translate((center.x, center.y))
        * kurbo::Affine::scale(f64::from(layer.scale))
        * kurbo::Affine::translate((-center.x, -center.y))
}

fn material_shared_axis_x_frame(
    progress: f32,
    direction: NavigationTransitionDirection,
    slide_distance: f64,
    viewport_width: f64,
    fade_through_threshold: f32,
) -> ResolvedNavigationTransitionFrame {
    assert!(
        viewport_width > 0.0,
        "navigation shared-axis viewport width must be positive"
    );
    assert!(
        slide_distance >= 0.0 && slide_distance.is_finite(),
        "navigation shared-axis slide distance must be finite and non-negative"
    );
    assert!(
        (0.0..1.0).contains(&fade_through_threshold),
        "navigation fade-through threshold must be in 0.0..1.0"
    );
    let slide_fraction = crate::num_cast::f64_as_f32(slide_distance / viewport_width);
    let direction = match direction {
        NavigationTransitionDirection::Push => -1.0,
        NavigationTransitionDirection::Pop => 1.0,
    };
    let outgoing_opacity = 1.0 - (progress / fade_through_threshold).clamp(0.0, 1.0);
    let incoming_opacity =
        ((progress - fade_through_threshold) / (1.0 - fade_through_threshold)).clamp(0.0, 1.0);

    ResolvedNavigationTransitionFrame {
        outgoing: NavigationTransitionLayer {
            offset_x: direction * slide_fraction * progress,
            opacity: outgoing_opacity,
            ..NavigationTransitionLayer::IDENTITY
        },
        incoming: NavigationTransitionLayer {
            offset_x: -direction * slide_fraction * (1.0 - progress),
            opacity: incoming_opacity,
            ..NavigationTransitionLayer::IDENTITY
        },
    }
}

pub fn interpolate_rect(from: kurbo::Rect, to: kurbo::Rect, progress: f64) -> kurbo::Rect {
    let interpolate = |from: f64, to: f64| (to - from).mul_add(progress, from);
    kurbo::Rect::new(
        interpolate(from.x0, to.x0),
        interpolate(from.y0, to.y0),
        interpolate(from.x1, to.x1),
        interpolate(from.y1, to.y1),
    )
}

/// Maps a matched element's page-space `bounds` onto `target`.
pub fn matched_element_transform(bounds: kurbo::Rect, target: kurbo::Rect) -> kurbo::Affine {
    kurbo::Affine::translate((target.x0, target.y0))
        * kurbo::Affine::scale_non_uniform(
            target.width() / bounds.width(),
            target.height() / bounds.height(),
        )
        * kurbo::Affine::translate((-bounds.x0, -bounds.y0))
}

#[cfg(test)]
mod tests {
    use super::material_shared_axis_x_frame;
    use waterui::navigation::NavigationTransitionDirection;

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= f32::EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn material_shared_axis_push_uses_thirty_point_travel_and_fade_through() {
        let start = material_shared_axis_x_frame(
            0.0,
            NavigationTransitionDirection::Push,
            30.0,
            300.0,
            0.35,
        );
        assert_near(start.outgoing.offset_x, 0.0);
        assert_near(start.outgoing.opacity, 1.0);
        assert_near(start.incoming.offset_x, 0.1);
        assert_near(start.incoming.opacity, 0.0);

        let threshold = material_shared_axis_x_frame(
            0.35,
            NavigationTransitionDirection::Push,
            30.0,
            300.0,
            0.35,
        );
        assert_near(threshold.outgoing.opacity, 0.0);
        assert_near(threshold.incoming.opacity, 0.0);

        let end = material_shared_axis_x_frame(
            1.0,
            NavigationTransitionDirection::Push,
            30.0,
            300.0,
            0.35,
        );
        assert_near(end.outgoing.offset_x, -0.1);
        assert_near(end.outgoing.opacity, 0.0);
        assert_near(end.incoming.offset_x, 0.0);
        assert_near(end.incoming.opacity, 1.0);
    }

    #[test]
    fn material_shared_axis_pop_reverses_both_layers() {
        let middle = material_shared_axis_x_frame(
            0.5,
            NavigationTransitionDirection::Pop,
            30.0,
            300.0,
            0.35,
        );

        assert_near(middle.outgoing.offset_x, 0.05);
        assert_near(middle.incoming.offset_x, -0.05);
    }
}
