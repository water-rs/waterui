//! Widget handlers: `WaterUI` control semantics rendered as dew draw
//! commands.
//!
//! Each submodule owns one native view type — its draw commands (kurbo
//! shapes painted with [`crate::theme`] colors) and its intrinsic
//! measurement — mirroring the semantics of the hydrolysis widget set at
//! dew's smaller scale. Handlers read reactive inputs through
//! [`DewRenderer::read_signal`], so a changed binding re-renders through
//! the runtime's display-list diff.
//!
//! Control labels are semantic [`Label`]s: dew renders them through the
//! styled-text pipeline from their semantic text, which is also exactly
//! what measurement uses. Icon payloads are not rendered — dew targets
//! panels without an OS icon catalog, so labels are text-only here.

use std::cell::RefCell;

use kurbo::{Affine, Rect};
use nami::Signal;
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_core::layout::{ProposalSize, Size};
use waterui_core::{AnyView, Environment, Native};
use waterui_text::styled::StyledStr;

use crate::dispatch::{DewDispatcher, DewRenderer, RenderContext, measure_view};
use crate::text::DewState;
use crate::theme;

pub mod divider;
#[cfg(feature = "progress")]
pub mod progress;
pub mod scroll;
pub mod slider;
pub mod stepper;
pub mod text_field;
pub mod toggle;

/// Registers every widget handler on the dispatcher.
pub fn register(dispatcher: &mut DewDispatcher) {
    dispatcher.register::<waterui_layout::Divider>(divider::render);
    dispatcher.register::<Native<waterui_layout::scroll::ScrollView>>(scroll::render);
    dispatcher.register::<Native<waterui_controls::toggle::ToggleConfig>>(toggle::render);
    dispatcher.register::<Native<waterui_controls::slider::SliderConfig>>(slider::render);
    dispatcher.register::<Native<waterui_controls::stepper::StepperConfig>>(stepper::render);
    dispatcher.register::<Native<waterui_controls::text_field::ResolvedTextFieldConfig>>(
        text_field::render,
    );
    #[cfg(feature = "progress")]
    dispatcher.register::<Native<waterui::component::progress::ProgressConfig>>(progress::render);
}

/// Narrows a logical-pixel `f64` to layout's `f32` vocabulary.
#[expect(
    clippy::cast_possible_truncation,
    reason = "logical-pixel geometry is far below f32 precision limits"
)]
pub const fn to_f32(value: f64) -> f32 {
    value as f32
}

/// Measures a control [`Label`] through its semantic text.
///
/// Visually hidden labels measure zero while their semantic text stays
/// available to future accessibility plumbing.
pub fn measure_label(state: &RefCell<DewState>, label: &Label, env: &Environment) -> Size {
    if matches!(label.display_mode_preference(), LabelDisplayMode::Hidden) {
        return Size::new(0.0, 0.0);
    }
    let styled = label.semantic_text().resolve(env).content.get();
    let (width, height) = state.borrow_mut().measure_styled(&styled, env, None);
    Size::new(width, height)
}

/// Renders a control [`Label`]'s semantic text inside `rect`, vertically
/// centered, subscribing to the text's reactive content.
pub fn render_label(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    rect: Rect,
    label: &Label,
    env: &Environment,
) {
    if matches!(label.display_mode_preference(), LabelDisplayMode::Hidden) {
        return;
    }
    let content = label.semantic_text().resolve(env).content;
    let styled = renderer.read_signal(&content);
    emit_styled_text(renderer, ctx, rect, &styled, env, theme::FOREGROUND);
}

/// Emits styled text laid out to `rect`'s width, vertically centered within
/// `rect`, painted with `brush` where spans carry no explicit color.
pub fn emit_styled_text(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    rect: Rect,
    styled: &StyledStr,
    env: &Environment,
    brush: peniko::Color,
) {
    let max_width = (rect.width() > 0.0).then(|| to_f32(rect.width()));
    let layout = renderer
        .state_cell()
        .borrow_mut()
        .build_styled_layout(styled, env, max_width, brush);
    let dy = ((rect.height() - f64::from(layout.height())) / 2.0).max(0.0);
    let transform = ctx.transform * Affine::translate((rect.x0, rect.y0 + dy));
    crate::text::emit_text_commands(renderer.list_mut(), &layout, transform);
}

/// Measures a normalized child view (slider end labels, progress labels) at
/// its intrinsic size.
pub fn measure_subview(state: &RefCell<DewState>, view: &AnyView, env: &Environment) -> Size {
    measure_view(state, view, env, ProposalSize::UNSPECIFIED).size
}

/// A child render context covering the window-coordinate-free local `rect`
/// within the current widget's bounds.
pub fn child_in_rect(ctx: RenderContext, rect: Rect) -> RenderContext {
    use waterui_core::layout::{Point, Rect as LayoutRect};
    ctx.child(LayoutRect::new(
        Point::new(to_f32(rect.x0), to_f32(rect.y0)),
        Size::new(to_f32(rect.width()), to_f32(rect.height())),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::DrawCommand;
    use kurbo::Point;
    use nami::binding;
    use peniko::Brush;
    use waterui_controls::slider::slider;
    use waterui_controls::toggle::Toggle;

    fn render_commands(view: impl waterui_core::View, width: f64, height: f64) -> Vec<DrawCommand> {
        let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
        let _ = executor_core::try_init_local_executor(native_executor::NativeExecutor::new());
        let mut renderer = DewRenderer::default();
        let list = renderer.render_tree(AnyView::new(view), &Environment::new(), width, height);
        list.commands().to_vec()
    }

    fn solid_brush(command: &DrawCommand) -> peniko::Color {
        match command {
            DrawCommand::FillPath {
                brush: Brush::Solid(color),
                ..
            }
            | DrawCommand::StrokePath {
                brush: Brush::Solid(color),
                ..
            } => *color,
            other => panic!("expected a solid fill or stroke, got {other:?}"),
        }
    }

    fn assert_point_near(actual: Point, expected: Point) {
        assert!(
            (actual - expected).hypot() < 0.05,
            "expected point near {expected:?}, got {actual:?}"
        );
    }

    fn assert_rect_near(actual: Rect, expected: Rect) {
        let close = |a: f64, b: f64| (a - b).abs() < 0.05;
        assert!(
            close(actual.x0, expected.x0)
                && close(actual.y0, expected.y0)
                && close(actual.x1, expected.x1)
                && close(actual.y1, expected.y1),
            "expected rect near {expected:?}, got {actual:?}"
        );
    }

    /// An unlabeled on-toggle in a 200×40 context: accent pill trailing at
    /// the right edge, white thumb at the on end, thumb outline.
    #[test]
    fn toggle_emits_track_thumb_and_outline() {
        let commands = render_commands(Toggle::new(&binding(true)), 200.0, 40.0);
        assert_eq!(commands.len(), 3, "track fill, thumb fill, thumb stroke");

        assert_eq!(solid_brush(&commands[0]), theme::ACCENT);
        assert_rect_near(commands[0].bounds(), Rect::new(149.0, 4.5, 200.0, 35.5));

        assert_eq!(solid_brush(&commands[1]), theme::THUMB);
        assert_point_near(commands[1].bounds().center(), Point::new(184.5, 20.0));

        assert_eq!(solid_brush(&commands[2]), theme::BORDER);
    }

    /// Turning the binding off swaps the track to the muted color and moves
    /// the thumb to the leading end.
    #[test]
    fn toggle_off_uses_muted_track_with_leading_thumb() {
        let commands = render_commands(Toggle::new(&binding(false)), 200.0, 40.0);
        assert_eq!(solid_brush(&commands[0]), theme::TRACK);
        assert_point_near(commands[1].bounds().center(), Point::new(164.5, 20.0));
    }

    /// A hidden-label slider at value 0.5 in a 220×20 context: full track,
    /// accent fill to the midpoint, thumb centered on the fill end.
    #[test]
    fn slider_emits_track_fill_and_thumb() {
        let commands = render_commands(slider("Volume", &binding(0.5)).hide_label(), 220.0, 20.0);
        assert_eq!(commands.len(), 4, "track, fill, thumb fill, thumb stroke");

        assert_eq!(solid_brush(&commands[0]), theme::TRACK);
        assert_rect_near(commands[0].bounds(), Rect::new(10.0, 8.0, 210.0, 12.0));

        assert_eq!(solid_brush(&commands[1]), theme::ACCENT);
        assert_rect_near(commands[1].bounds(), Rect::new(10.0, 8.0, 110.0, 12.0));

        assert_eq!(solid_brush(&commands[2]), theme::THUMB);
        assert_point_near(commands[2].bounds().center(), Point::new(110.0, 10.0));

        assert_eq!(solid_brush(&commands[3]), theme::BORDER);
    }
}
