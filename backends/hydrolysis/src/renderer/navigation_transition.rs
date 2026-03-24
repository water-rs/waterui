use super::*;

pub(crate) fn draw_navigation_transition(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    style: NavigationTransition,
    direction: NavigationTransitionDirection,
    progress: f64,
    from_scene: &vello::Scene,
    to_scene: &vello::Scene,
) {
    let width = bounds.width();
    match style {
        NavigationTransition::PushPop => {
            let (from_x, to_x) = match direction {
                NavigationTransitionDirection::Push => (
                    -width * NAVIGATION_PUSHPOP_PARALLAX_FACTOR * progress,
                    width * (1.0 - progress),
                ),
                NavigationTransitionDirection::Pop => (
                    width * progress,
                    -width * NAVIGATION_PUSHPOP_PARALLAX_FACTOR * (1.0 - progress),
                ),
            };
            append_scene_with_alpha(scene, transform, bounds, from_scene, from_x, 1.0);
            append_scene_with_alpha(scene, transform, bounds, to_scene, to_x, 1.0);
        }
        NavigationTransition::Fade => {
            append_scene_with_alpha(
                scene,
                transform,
                bounds,
                from_scene,
                0.0,
                1.0f32 - progress as f32,
            );
            append_scene_with_alpha(scene, transform, bounds, to_scene, 0.0, progress as f32);
        }
        NavigationTransition::None => {
            scene.append(to_scene, Some(transform));
        }
    }
}

fn append_scene_with_alpha(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    clip_bounds: vello::kurbo::Rect,
    content: &vello::Scene,
    offset_x: f64,
    alpha: f32,
) {
    if alpha <= 0.0 {
        return;
    }
    scene.push_layer(
        vello::peniko::Fill::NonZero,
        vello::peniko::BlendMode::default(),
        alpha,
        transform,
        &clip_bounds,
    );
    scene.append(
        content,
        Some(transform * vello::kurbo::Affine::translate((offset_x, 0.0))),
    );
    scene.pop_layer();
}
