use crate::num_cast::f64_as_f32;
use crate::theme::NavigationMotion;
use waterui::navigation::{
    AnyNavigationTransition, NavigationTransitionDirection, RetainedNavigationTransition,
};
use waterui_core::contract_violation;
use waterui_core::id::Id;
use waterui_core::layout::{LayoutDirection, Size};
use waterui_structural_transition::{
    Transition as _, TransitionGeometry, TransitionPhase, TransitionProperties, TransitionSpec,
};

/// How a transition presents its two pages this frame.
pub enum NavigationPresentation {
    /// Both pages cross-fade while the matched element pair flies between
    /// its two frames.
    Matched(Id),
    /// Each page draws under its own core [`TransitionProperties`].
    Properties {
        /// Properties applied to the departing page.
        outgoing: TransitionProperties,
        /// Properties applied to the arriving page.
        incoming: TransitionProperties,
    },
}

fn sample_spec_at(
    spec: &TransitionSpec,
    phase: TransitionPhase,
    absence: f32,
    geometry: TransitionGeometry,
    reduce_motion: bool,
) -> TransitionProperties {
    if reduce_motion {
        spec.transition()
            .reduced()
            .properties(phase, absence, geometry)
    } else {
        spec.transition().properties(phase, absence, geometry)
    }
}

pub fn navigation_presentation(
    style: &AnyNavigationTransition,
    motion: NavigationMotion,
    direction: NavigationTransitionDirection,
    progress: f64,
    viewport_size: kurbo::Size,
    layout_direction: LayoutDirection,
    reduce_motion: bool,
) -> NavigationPresentation {
    let progress = f64_as_f32(progress).clamp(0.0, 1.0);
    let (outgoing, incoming) = match style.retained() {
        RetainedNavigationTransition::MatchedGeometry(id) => {
            return NavigationPresentation::Matched(id);
        }
        RetainedNavigationTransition::PlatformDefault => material_shared_axis_x_properties(
            progress,
            direction,
            motion.shared_axis_slide_distance,
            viewport_size.width,
            motion.fade_through_threshold,
        ),
        RetainedNavigationTransition::None => (
            TransitionProperties::IDENTITY,
            TransitionProperties::IDENTITY,
        ),
        RetainedNavigationTransition::Transitions => {
            let pair = style.transitions(direction);
            let geometry = TransitionGeometry::new(
                Size::new(
                    f64_as_f32(viewport_size.width),
                    f64_as_f32(viewport_size.height),
                ),
                layout_direction,
            );
            (
                sample_spec_at(
                    &pair.outgoing,
                    TransitionPhase::Removal,
                    progress,
                    geometry,
                    reduce_motion,
                ),
                sample_spec_at(
                    &pair.incoming,
                    TransitionPhase::Insertion,
                    1.0 - progress,
                    geometry,
                    reduce_motion,
                ),
            )
        }
    };
    NavigationPresentation::Properties { outgoing, incoming }
}

fn material_shared_axis_x_properties(
    progress: f32,
    direction: NavigationTransitionDirection,
    slide_distance: f32,
    viewport_width: f64,
    fade_through_threshold: f32,
) -> (TransitionProperties, TransitionProperties) {
    if viewport_width <= 0.0 {
        contract_violation!(
            "navigation shared-axis viewport width must be positive; the pages show without a transition"
        );
        return (
            TransitionProperties::IDENTITY,
            TransitionProperties::IDENTITY,
        );
    }
    if !(slide_distance >= 0.0 && slide_distance.is_finite()) {
        contract_violation!(
            "navigation shared-axis slide distance must be finite and non-negative; the pages show without a transition"
        );
        return (
            TransitionProperties::IDENTITY,
            TransitionProperties::IDENTITY,
        );
    }
    if !(0.0..1.0).contains(&fade_through_threshold) {
        contract_violation!(
            "navigation fade-through threshold must be in 0.0..1.0; the pages show without a transition"
        );
        return (
            TransitionProperties::IDENTITY,
            TransitionProperties::IDENTITY,
        );
    }
    let sign = match direction {
        NavigationTransitionDirection::Push => -1.0,
        NavigationTransitionDirection::Pop => 1.0,
    };
    let outgoing_opacity = 1.0 - (progress / fade_through_threshold).clamp(0.0, 1.0);
    let incoming_opacity =
        ((progress - fade_through_threshold) / (1.0 - fade_through_threshold)).clamp(0.0, 1.0);

    (
        TransitionProperties {
            translation: [sign * slide_distance * progress, 0.0],
            opacity: outgoing_opacity,
            ..TransitionProperties::IDENTITY
        },
        TransitionProperties {
            translation: [-sign * slide_distance * (1.0 - progress), 0.0],
            opacity: incoming_opacity,
            ..TransitionProperties::IDENTITY
        },
    )
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
    use super::material_shared_axis_x_properties;
    use waterui::navigation::NavigationTransitionDirection;

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn material_shared_axis_push_uses_thirty_point_travel_and_fade_through() {
        let (start_out, start_in) = material_shared_axis_x_properties(
            0.0,
            NavigationTransitionDirection::Push,
            30.0,
            300.0,
            0.35,
        );
        assert_near(start_out.translation[0], 0.0);
        assert_near(start_out.opacity, 1.0);
        assert_near(start_in.translation[0], 30.0);
        assert_near(start_in.opacity, 0.0);

        let (threshold_out, threshold_in) = material_shared_axis_x_properties(
            0.35,
            NavigationTransitionDirection::Push,
            30.0,
            300.0,
            0.35,
        );
        assert_near(threshold_out.opacity, 0.0);
        assert_near(threshold_in.opacity, 0.0);

        let (end_out, end_in) = material_shared_axis_x_properties(
            1.0,
            NavigationTransitionDirection::Push,
            30.0,
            300.0,
            0.35,
        );
        assert_near(end_out.translation[0], -30.0);
        assert_near(end_out.opacity, 0.0);
        assert_near(end_in.translation[0], 0.0);
        assert_near(end_in.opacity, 1.0);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "viewport width must be positive")
    )]
    fn a_zero_viewport_shows_the_pages_without_a_transition() {
        let (outgoing, incoming) = material_shared_axis_x_properties(
            0.5,
            NavigationTransitionDirection::Push,
            30.0,
            0.0,
            0.35,
        );
        assert_near(outgoing.translation[0], 0.0);
        assert_near(outgoing.opacity, 1.0);
        assert_near(incoming.translation[0], 0.0);
        assert_near(incoming.opacity, 1.0);
    }

    #[test]
    fn material_shared_axis_pop_reverses_both_layers() {
        let (middle_out, middle_in) = material_shared_axis_x_properties(
            0.5,
            NavigationTransitionDirection::Pop,
            30.0,
            300.0,
            0.35,
        );

        assert_near(middle_out.translation[0], 15.0);
        assert_near(middle_in.translation[0], -15.0);
    }
}
