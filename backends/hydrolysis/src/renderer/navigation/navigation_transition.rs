use super::*;

pub(crate) struct NavigationTransitionFrame<'a> {
    pub(crate) scene: &'a mut vello::Scene,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) style: NavigationTransition,
    pub(crate) direction: NavigationTransitionDirection,
    pub(crate) progress: f64,
    pub(crate) pushpop_parallax_factor: f64,
    pub(crate) from_scene: &'a vello::Scene,
    pub(crate) to_scene: &'a vello::Scene,
}

pub(crate) fn draw_navigation_transition(frame: NavigationTransitionFrame<'_>) {
    let width = frame.bounds.width();
    match frame.style {
        NavigationTransition::PushPop => {
            let (from_x, to_x) = match frame.direction {
                NavigationTransitionDirection::Push => (
                    -width * frame.pushpop_parallax_factor * frame.progress,
                    width * (1.0 - frame.progress),
                ),
                NavigationTransitionDirection::Pop => (
                    width * frame.progress,
                    -width * frame.pushpop_parallax_factor * (1.0 - frame.progress),
                ),
            };
            append_scene_with_alpha(
                frame.scene,
                frame.transform,
                frame.bounds,
                frame.from_scene,
                from_x,
                1.0,
            );
            append_scene_with_alpha(
                frame.scene,
                frame.transform,
                frame.bounds,
                frame.to_scene,
                to_x,
                1.0,
            );
        }
        NavigationTransition::Fade => {
            append_scene_with_alpha(
                frame.scene,
                frame.transform,
                frame.bounds,
                frame.from_scene,
                0.0,
                1.0f32 - frame.progress as f32,
            );
            append_scene_with_alpha(
                frame.scene,
                frame.transform,
                frame.bounds,
                frame.to_scene,
                0.0,
                frame.progress as f32,
            );
        }
        NavigationTransition::None => {
            frame.scene.append(frame.to_scene, Some(frame.transform));
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
