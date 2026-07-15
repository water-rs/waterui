use waterui::navigation::{
    AnyNavigationTransition, NativeNavigationTransition, NavigationTransitionDirection,
    NavigationTransitionLayer,
};

use super::{NavigationCapturedScene, NavigationMatchedElement};

pub(crate) struct NavigationTransitionFrame<'a> {
    pub(crate) scene: &'a mut vello::Scene,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) style: AnyNavigationTransition,
    pub(crate) direction: NavigationTransitionDirection,
    pub(crate) progress: f64,
    pub(crate) from_scene: &'a NavigationCapturedScene,
    pub(crate) to_scene: &'a NavigationCapturedScene,
}

pub(crate) fn draw_navigation_transition(frame: NavigationTransitionFrame<'_>) {
    if let NativeNavigationTransition::Zoom(id) = frame.style.native() {
        draw_matched_navigation_transition(frame, id);
        return;
    }
    #[allow(clippy::cast_possible_truncation)]
    let resolved = frame.style.frame(frame.progress as f32, frame.direction);
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
