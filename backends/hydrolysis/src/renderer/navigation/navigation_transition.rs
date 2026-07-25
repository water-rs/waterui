use waterui::navigation::{
    AnyNavigationTransition, NativeNavigationTransition, NavigationTransitionDirection,
    NavigationTransitionFrame as ResolvedNavigationTransitionFrame, NavigationTransitionLayer,
};
use waterui_backend_core::widget::NavigationMotion;

use super::{NavigationCapturedScene, NavigationMatchedElement};

pub(crate) struct NavigationTransitionFrame<'a> {
    pub(crate) scene: &'a mut vello::Scene,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) style: AnyNavigationTransition,
    pub(crate) motion: NavigationMotion,
    pub(crate) direction: NavigationTransitionDirection,
    pub(crate) progress: f64,
    pub(crate) from_scene: &'a NavigationCapturedScene,
    pub(crate) to_scene: &'a NavigationCapturedScene,
}

pub(crate) fn draw_navigation_transition(frame: NavigationTransitionFrame<'_>) {
    let native = frame.style.native();
    if let NativeNavigationTransition::Zoom(id) = native {
        draw_matched_navigation_transition(frame, id);
        return;
    }
    #[allow(clippy::cast_possible_truncation)]
    let progress = frame.progress as f32;
    let resolved = match native {
        NativeNavigationTransition::Automatic => material_shared_axis_x_frame(
            progress,
            frame.direction,
            frame.motion.shared_axis_slide_distance,
            frame.bounds.width(),
            frame.motion.fade_through_threshold,
        ),
        _ => frame.style.frame(progress, frame.direction),
    };
    let from_scene = frame.from_scene.composed();
    let to_scene = frame.to_scene.composed();
    let outgoing = (&from_scene, resolved.outgoing);
    let incoming = (&to_scene, resolved.incoming);
    let layers = match frame.direction {
        NavigationTransitionDirection::Push => [outgoing, incoming],
        NavigationTransitionDirection::Pop => [incoming, outgoing],
    };
    for (scene, layer) in layers {
        append_scene_layer(frame.scene, frame.transform, frame.bounds, scene, layer);
    }
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
    let slide_fraction = (slide_distance / viewport_width) as f32;
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

fn draw_matched_navigation_transition(
    frame: NavigationTransitionFrame<'_>,
    id: waterui_core::id::Id,
) {
    let (from_element, to_element, from_is_source, to_is_source) = match frame.direction {
        NavigationTransitionDirection::Push => (
            frame.from_scene.sources.get(&id),
            frame.to_scene.destinations.get(&id),
            true,
            false,
        ),
        NavigationTransitionDirection::Pop => (
            frame.from_scene.destinations.get(&id),
            frame.to_scene.sources.get(&id),
            false,
            true,
        ),
    };
    let from_element = from_element.unwrap_or_else(|| {
        panic!("navigation zoom source {id:?} is not present in the outgoing page")
    });
    let to_element = to_element.unwrap_or_else(|| {
        panic!("navigation zoom destination {id:?} is not present in the incoming page")
    });
    assert!(
        from_element.bounds.width() > 0.0
            && from_element.bounds.height() > 0.0
            && to_element.bounds.width() > 0.0
            && to_element.bounds.height() > 0.0,
        "navigation zoom geometry must have a positive size"
    );

    let from_page = frame.from_scene.composed_without(from_is_source, id);
    let to_page = frame.to_scene.composed_without(to_is_source, id);
    append_scene_with_opacity(
        frame.scene,
        frame.transform,
        frame.bounds,
        &from_page,
        1.0 - frame.progress as f32,
    );
    append_scene_with_opacity(
        frame.scene,
        frame.transform,
        frame.bounds,
        &to_page,
        frame.progress as f32,
    );

    let bounds = interpolate_rect(from_element.bounds, to_element.bounds, frame.progress);
    append_matched_element(
        frame.scene,
        frame.transform,
        from_element,
        bounds,
        1.0 - frame.progress as f32,
    );
    append_matched_element(
        frame.scene,
        frame.transform,
        to_element,
        bounds,
        frame.progress as f32,
    );
}

fn interpolate_rect(
    from: vello::kurbo::Rect,
    to: vello::kurbo::Rect,
    progress: f64,
) -> vello::kurbo::Rect {
    let interpolate = |from: f64, to: f64| from + (to - from) * progress;
    vello::kurbo::Rect::new(
        interpolate(from.x0, to.x0),
        interpolate(from.y0, to.y0),
        interpolate(from.x1, to.x1),
        interpolate(from.y1, to.y1),
    )
}

fn append_matched_element(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    element: &NavigationMatchedElement,
    target: vello::kurbo::Rect,
    opacity: f32,
) {
    if opacity <= 0.0 {
        return;
    }
    let local = vello::kurbo::Affine::translate((target.x0, target.y0))
        * vello::kurbo::Affine::scale_non_uniform(
            target.width() / element.bounds.width(),
            target.height() / element.bounds.height(),
        )
        * vello::kurbo::Affine::translate((-element.bounds.x0, -element.bounds.y0));
    scene.push_layer(
        vello::peniko::Fill::NonZero,
        vello::peniko::BlendMode::default(),
        opacity,
        transform,
        &target,
    );
    scene.append(&element.scene, Some(transform * local));
    scene.pop_layer();
}

fn append_scene_with_opacity(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    clip_bounds: vello::kurbo::Rect,
    content: &vello::Scene,
    opacity: f32,
) {
    append_scene_layer(
        scene,
        transform,
        clip_bounds,
        content,
        NavigationTransitionLayer {
            opacity,
            ..NavigationTransitionLayer::IDENTITY
        },
    );
}

fn append_scene_layer(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    clip_bounds: vello::kurbo::Rect,
    content: &vello::Scene,
    layer: NavigationTransitionLayer,
) {
    if layer.opacity <= 0.0 {
        return;
    }
    let center = clip_bounds.center();
    let local = vello::kurbo::Affine::translate((
        f64::from(layer.offset_x) * clip_bounds.width(),
        f64::from(layer.offset_y) * clip_bounds.height(),
    )) * vello::kurbo::Affine::translate((center.x, center.y))
        * vello::kurbo::Affine::scale(f64::from(layer.scale))
        * vello::kurbo::Affine::translate((-center.x, -center.y));
    let transformed_bounds = local.transform_rect_bbox(clip_bounds);
    scene.push_layer(
        vello::peniko::Fill::NonZero,
        vello::peniko::BlendMode::default(),
        layer.opacity,
        transform,
        &transformed_bounds,
    );
    scene.append(content, Some(transform * local));
    scene.pop_layer();
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
