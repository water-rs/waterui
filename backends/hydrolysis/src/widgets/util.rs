use crate::engine::WidgetTheme;
use waterui_backend_core::widget::WidgetInteractionState;
use waterui_core::Environment;

pub(crate) fn widget_theme(env: &Environment) -> &dyn WidgetTheme {
    env.get::<Box<dyn WidgetTheme>>()
        .map(Box::as_ref)
        .expect("hydrolysis widget theme is not installed in the environment")
}

pub(crate) fn inset_rect(rect: vello::kurbo::Rect, dx: f64, dy: f64) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        rect.x0 + dx,
        rect.y0 + dy,
        (rect.x1 - dx).max(rect.x0 + dx),
        (rect.y1 - dy).max(rect.y0 + dy),
    )
}

pub(crate) fn local_interaction_state(
    mut state: WidgetInteractionState,
    hit_transform: vello::kurbo::Affine,
) -> WidgetInteractionState {
    state.press_origin = state
        .press_origin
        .map(|origin| hit_transform.inverse() * origin);
    state
}
