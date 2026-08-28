use crate::engine::WidgetTheme;
use nami::Computed;
use waterui_core::Environment;
use waterui_core::interaction::Disabled;

/// The disabled state in force at this point in the view tree.
///
/// Disabled is a scoped environment attribute installed by `.disabled(...)`,
/// not a field on any control's configuration: a control reads the state at its
/// own position, exactly the way it reads the widget theme. Reads `false` when
/// no enclosing scope disables the subtree.
pub(crate) fn widget_disabled(env: &Environment) -> Computed<bool> {
    env.get::<Disabled>()
        .map_or_else(|| Computed::constant(false), |scope| scope.signal().clone())
}

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
