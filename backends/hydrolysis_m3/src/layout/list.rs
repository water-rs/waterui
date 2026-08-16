use crate::dimensions::{
    LIST_DELETE_CONTROL_WIDTH, LIST_DIVIDER_LEADING_INSET, LIST_DIVIDER_TRAILING_INSET,
    LIST_HORIZONTAL_INSET, LIST_MOVE_CONTROL_WIDTH, LIST_ONE_LINE_ROW_HEIGHT,
    LIST_SECTION_FOOTER_HEIGHT, LIST_SECTION_HEADER_HEIGHT,
    LIST_TRAILING_CONTROL_SPACING, LIST_TRAILING_CONTROL_VERTICAL_INSET, LIST_VERTICAL_INSET,
};
use crate::icon_paths::{self, IconGrid};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{
    Brush, DrawContext, ListDividerMetrics, ListMetrics, ListRowMetrics,
    ListSectionMetrics, ListTrailingControlMetrics, WidgetInteractionState,
};
use vello::kurbo::{Point, Rect};

/// Size the trailing row affordances draw their icons at (Material's 24dp
/// icon), centered in the larger hit box the row gives them.
const LIST_CONTROL_ICON_SIZE: f64 = 24.0;

/// Diameter of a trailing control's state layer, matching
/// `IconButtonTokens.StateLayerSize`.
const LIST_CONTROL_STATE_LAYER_SIZE: f64 = 40.0;

/// Size of the delete glyph revealed by a swipe, on Material's 24dp icon grid.
const SWIPE_ICON_SIZE: f64 = 24.0;
/// Gap between the glyph and the edge of the row it is revealed against.
const SWIPE_ICON_EDGE_INSET: f64 = 16.0;
/// Scale the glyph starts at, growing to full size as the swipe reaches its
/// dismiss threshold.
const SWIPE_ICON_MIN_SCALE: f64 = 0.7;
/// Opacity of the key shadow cast by a lifted row.
const LIFT_SHADOW_ALPHA: f32 = 0.19;

pub const fn metrics() -> ListMetrics {
    ListMetrics::new(
        ListRowMetrics::new(
            LIST_ONE_LINE_ROW_HEIGHT,
            LIST_HORIZONTAL_INSET,
            LIST_VERTICAL_INSET,
        ),
        ListDividerMetrics::new(LIST_DIVIDER_LEADING_INSET, LIST_DIVIDER_TRAILING_INSET),
        ListTrailingControlMetrics::new(
            LIST_MOVE_CONTROL_WIDTH,
            LIST_DELETE_CONTROL_WIDTH,
            LIST_TRAILING_CONTROL_SPACING,
            LIST_TRAILING_CONTROL_VERTICAL_INSET,
        ),
        ListSectionMetrics::new(LIST_SECTION_HEADER_HEIGHT, LIST_SECTION_FOOTER_HEIGHT),
    )
}

pub fn draw_row_background(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    _alternate: bool,
) {
    draw.fill_rect(bounds, &Brush::from(colors.surface.peniko()));
}

/// The reorder grip: Material's `drag_handle` icon on the bare row.
///
/// Material gives a list's trailing affordance an icon and a state layer, not a
/// bordered container of its own — an outlined box here reads as a stepper
/// control rather than something to drag.
pub fn draw_move_control(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    let grid = IconGrid::centered(icon_center(bounds), LIST_CONTROL_ICON_SIZE);
    draw.fill_path(
        &icon_paths::drag_handle(grid),
        &Brush::from(colors.on_surface_variant.peniko()),
    );
}

pub fn draw_move_control_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    draw_control_state_layer(draw, bounds, colors.on_surface.peniko(), state);
}

/// The delete affordance: Material's `delete` icon tinted `error`.
///
/// A solid error-coloured slab was never the Material treatment — the row keeps
/// its own surface and the destructive action is carried by the icon's colour.
pub fn draw_delete_control(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    let grid = IconGrid::centered(icon_center(bounds), LIST_CONTROL_ICON_SIZE);
    draw.fill_path(
        &icon_paths::delete(grid),
        &Brush::from(colors.error.peniko()),
    );
}

/// Centre of a trailing control's hit box, where its icon is drawn.
fn icon_center(bounds: Rect) -> Point {
    Point::new(
        bounds.x0 + bounds.width() / 2.0,
        bounds.y0 + bounds.height() / 2.0,
    )
}

pub fn draw_delete_control_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    draw_control_state_layer(draw, bounds, colors.on_error.peniko(), state);
}

/// Background revealed behind a row being swiped away: the error container,
/// with a delete glyph pinned to the edge the row is uncovering.
///
/// The glyph fades and grows in with `progress`, so crossing the dismiss
/// threshold is legible before the finger lifts.
pub fn draw_swipe_dismiss_background(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    progress: f64,
    toward_start: bool,
) {
    draw.fill_rect(bounds, &Brush::from(colors.error_container.peniko()));
    let inset = SWIPE_ICON_EDGE_INSET + SWIPE_ICON_SIZE / 2.0;
    let center_x = if toward_start {
        bounds.x1 - inset
    } else {
        bounds.x0 + inset
    };
    let center = Point::new(center_x, bounds.y0 + bounds.height() / 2.0);
    // The glyph grows from a reduced size to full as the swipe approaches its
    // threshold, so crossing it is legible before the finger lifts.
    let scale =
        (1.0 - SWIPE_ICON_MIN_SCALE).mul_add(progress.clamp(0.0, 1.0), SWIPE_ICON_MIN_SCALE);
    let grid = IconGrid::centered(center, SWIPE_ICON_SIZE * scale);
    draw.fill_path(
        &icon_paths::delete(grid),
        &Brush::from(colors.on_error_container.peniko()),
    );
}

/// Lifted treatment for a row being dragged to a new position: Material raises
/// the item to a surface-container tone and casts the level-3 key shadow.
pub fn draw_row_lifted(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    elevation: f64,
) {
    let shadow = Rect::new(
        bounds.x0,
        elevation.mul_add(0.5, bounds.y0),
        bounds.x1,
        elevation.mul_add(0.5, bounds.y1),
    );
    draw.fill_rect(
        shadow,
        &Brush::from(colors.shadow.peniko().multiply_alpha(LIFT_SHADOW_ALPHA)),
    );
    draw.fill_rect(bounds, &Brush::from(colors.surface_container_high.peniko()));
}

pub fn draw_separator(_colors: &MaterialColorScheme, _draw: &mut dyn DrawContext, _bounds: Rect) {}

/// Material draws an icon button's state layer as a circle, so the layer is
/// inscribed in `bounds` rather than filling it — stretching it across a wide,
/// short hit box would produce an oval no Material surface has.
fn draw_control_state_layer(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    color: vello::peniko::Color,
    state: WidgetInteractionState,
) {
    let diameter = bounds
        .width()
        .min(bounds.height())
        .min(LIST_CONTROL_STATE_LAYER_SIZE);
    let center = icon_center(bounds);
    let layer = Rect::new(
        center.x - diameter / 2.0,
        center.y - diameter / 2.0,
        center.x + diameter / 2.0,
        center.y + diameter / 2.0,
    );
    state_layer::draw_bounded(draw, layer, (diameter / 2.0).into(), color, state);
}

#[cfg(test)]
mod tests {
    use super::metrics;
    use crate::dimensions::{
        LIST_DELETE_CONTROL_WIDTH, LIST_DIVIDER_LEADING_INSET, LIST_DIVIDER_TRAILING_INSET,
        LIST_HORIZONTAL_INSET, LIST_MOVE_CONTROL_WIDTH, LIST_ONE_LINE_ROW_HEIGHT,
        LIST_TRAILING_CONTROL_SPACING, LIST_TRAILING_CONTROL_VERTICAL_INSET, LIST_VERTICAL_INSET,
    };

    #[test]
    fn list_metrics_match_compose_list_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.one_line_row_height, LIST_ONE_LINE_ROW_HEIGHT);
        assert_eq!(metrics.horizontal_inset, LIST_HORIZONTAL_INSET);
        assert_eq!(metrics.vertical_inset, LIST_VERTICAL_INSET);
        assert_eq!(metrics.divider_leading_inset, LIST_DIVIDER_LEADING_INSET);
        assert_eq!(metrics.divider_trailing_inset, LIST_DIVIDER_TRAILING_INSET);
        assert_eq!(metrics.move_control_width, LIST_MOVE_CONTROL_WIDTH);
        assert_eq!(metrics.delete_control_width, LIST_DELETE_CONTROL_WIDTH);
        assert_eq!(
            metrics.trailing_control_spacing,
            LIST_TRAILING_CONTROL_SPACING
        );
        assert_eq!(
            metrics.trailing_control_vertical_inset,
            LIST_TRAILING_CONTROL_VERTICAL_INSET
        );
        assert_eq!(LIST_ONE_LINE_ROW_HEIGHT, 56.0);
        assert_eq!(LIST_HORIZONTAL_INSET, 16.0);
        assert_eq!(LIST_VERTICAL_INSET, 10.0);
        assert_eq!(LIST_DIVIDER_LEADING_INSET, 16.0);
        assert_eq!(LIST_DIVIDER_TRAILING_INSET, 16.0);
    }
}
