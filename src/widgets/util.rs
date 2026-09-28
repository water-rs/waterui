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

pub(crate) fn inset_rect(rect: kurbo::Rect, dx: f64, dy: f64) -> kurbo::Rect {
    kurbo::Rect::new(
        rect.x0 + dx,
        rect.y0 + dy,
        (rect.x1 - dx).max(rect.x0 + dx),
        (rect.y1 - dy).max(rect.y0 + dy),
    )
}

/// The rect a label is flushed into inside button chrome: its measured size,
/// capped to `rect` and centred, so a label smaller than the chrome's content
/// rect sits in the middle — the same centred-content placement Compose
/// applies inside a button's minimum bounds.
pub(crate) fn centered_label_rect(
    rect: kurbo::Rect,
    size: waterui_core::layout::Size,
) -> kurbo::Rect {
    let width = f64::from(size.width).min(rect.width());
    let height = f64::from(size.height).min(rect.height());
    let x0 = rect.x0 + (rect.width() - width) * 0.5;
    let y0 = rect.y0 + (rect.height() - height) * 0.5;
    kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

/// The rect for a label that sits beside its control on a row — a toggle's
/// label next to its switch or checkbox, a stepper's label next to its
/// buttons. The label keeps the horizontal extent the widget picked for it,
/// but vertically it takes its own height (capped to `row`) centred on the
/// control's centre line: a single-line label shares the control's centre
/// instead of riding the row's top edge.
pub(crate) fn label_beside_control_bounds(
    x0: f64,
    x1: f64,
    row: kurbo::Rect,
    control: kurbo::Rect,
    label_height: f64,
) -> kurbo::Rect {
    let height = label_height.min(row.height());
    let y0 = (control.y0 + control.y1 - height) * 0.5;
    kurbo::Rect::new(x0, y0, x1, y0 + height)
}
