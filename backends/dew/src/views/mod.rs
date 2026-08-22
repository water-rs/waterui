//! Widget handlers: `WaterUI` control semantics rendered as dew draw
//! commands.
//!
//! Each submodule owns one persistent native-view node. Nodes retain their
//! semantic configuration and read reactive inputs on refresh, so bindings
//! never require a view-body rebuild.
//!
//! Control labels are semantic [`Label`]s: dew renders them through the
//! styled-text pipeline from their semantic text, which is also exactly
//! what measurement uses. Icon payloads are not rendered — dew targets
//! panels without an OS icon catalog, so labels are text-only here.

use std::cell::RefCell;

use kurbo::{Affine, Rect};
use nami::Computed;
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_core::Environment;
use waterui_core::layout::Size;
use waterui_text::styled::StyledStr;

use crate::dispatch::{DewRenderer, RenderContext, WatchedSignal};
use crate::text::{DewState, TextLayoutCache, TextLayoutKey};
use crate::theme;

pub mod button;
pub mod divider;
#[cfg(feature = "progress")]
pub mod progress;
pub mod scroll;
pub mod slider;
pub mod stepper;
pub mod text_field;
pub mod toggle;

/// The disabled state in force at this point in the view tree.
///
/// Disabled is a scoped environment attribute installed by `.disabled(...)`,
/// not a field on any control's configuration: a control reads the state at its
/// own position. Reads `false` when no enclosing scope disables the subtree.
pub fn view_disabled(env: &Environment) -> nami::Computed<bool> {
    env.get::<waterui_core::interaction::Disabled>()
        .map_or_else(
            || nami::Computed::constant(false),
            |scope| scope.signal().clone(),
        )
}

/// Narrows a logical-pixel `f64` to layout's `f32` vocabulary.
#[expect(
    clippy::cast_possible_truncation,
    reason = "logical-pixel geometry is far below f32 precision limits"
)]
pub const fn to_f32(value: f64) -> f32 {
    value as f32
}

/// A control label together with its retained text layout.
///
/// Control labels resolve their text from a signal, so they need exactly what
/// `TextNode` already has: a watched revision, a [`TextLayoutCache`] keyed on
/// it, and retained glyph runs. Without this every control re-shaped its label
/// through `parley` on every measure *and* every render — and layout probes
/// each control several times per frame, so a static button label was being
/// re-shaped a handful of times per frame forever.
///
/// Build one per label at node-construction time and keep it in the node.
pub struct LabelText {
    display_mode: LabelDisplayMode,
    content: WatchedSignal<Computed<StyledStr>>,
    cache: RefCell<TextLayoutCache>,
}

impl LabelText {
    /// Retains `label`'s semantic text, subscribing to its reactive content.
    #[must_use]
    pub fn new(label: &Label, env: &Environment, signals: FrameSignals) -> Self {
        Self {
            display_mode: label.display_mode_preference(),
            content: WatchedSignal::new(label.semantic_text().resolve(env).content, signals),
            cache: RefCell::new(TextLayoutCache::default()),
        }
    }

    /// Whether this label draws nothing.
    ///
    /// Visually hidden labels measure zero and emit no glyphs while their
    /// semantic text stays available to accessibility plumbing.
    const fn is_hidden(&self) -> bool {
        matches!(self.display_mode, LabelDisplayMode::Hidden)
    }

    /// Measures the label at its intrinsic width.
    pub fn measure(&self, state: &RefCell<DewState>, env: &Environment) -> Size {
        if self.is_hidden() {
            return Size::new(0.0, 0.0);
        }
        let key = TextLayoutKey::new(None, theme::foreground(env));
        let mut cache = self.cache.borrow_mut();
        let ((width, height), outcome) = cache.measure(self.content.revision(), key, None, || {
            state
                .borrow_mut()
                .build_styled_layout(&self.content.get(), env, None, key.brush)
        });
        let size = Size::new(width, height);
        state.borrow_mut().record_layout(outcome);
        size
    }

    /// Renders the label inside `rect`, vertically centred, in the theme
    /// foreground.
    pub fn render(
        &self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        rect: Rect,
        env: &Environment,
    ) {
        let foreground = renderer.theme().foreground();
        self.render_with_brush(renderer, ctx, rect, env, foreground);
    }

    /// Renders the label with a caller-selected default brush, for surfaces
    /// that change the foreground contrast.
    pub fn render_with_brush(
        &self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        rect: Rect,
        env: &Environment,
        brush: peniko::Color,
    ) {
        if self.is_hidden() {
            return;
        }
        let max_width = (rect.width() > 0.0).then(|| to_f32(rect.width()));
        let key = TextLayoutKey { max_width, brush };
        let revision = self.content.revision();
        let mut cache = self.cache.borrow_mut();
        let (list, state) = renderer.list_and_state();
        // Vertical centring needs the laid-out height, which is only known
        // once a layout for this key has existed.
        let ((_, height), outcome) = cache.measure(revision, key, None, || {
            state
                .borrow_mut()
                .build_styled_layout(&self.content.get(), env, max_width, brush)
        });
        state.borrow_mut().record_layout(outcome);
        let dy = ((rect.height() - f64::from(height)) / 2.0).max(0.0);
        let transform = ctx.transform * Affine::translate((rect.x0, rect.y0 + dy));
        let outcome = cache.emit(revision, key, None, transform, list, || {
            state
                .borrow_mut()
                .build_styled_layout(&self.content.get(), env, max_width, brush)
        });
        state.borrow_mut().record_layout(outcome);
    }
}

/// Emits styled text laid out to `rect`'s width, vertically centered within
/// `rect`, painted with `brush` where spans carry no explicit color.
///
/// The uncached path, for text a node does not retain (a value that is
/// re-derived every frame anyway). Prefer [`LabelText`] for anything stable.
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
    use waterui_core::AnyView;
    use waterui_core::plugin::Plugin;
    use waterui_graphics::color::{ResolvedColor, Srgb};

    fn render_commands(
        view: impl waterui_core::View,
        width: f64,
        height: f64,
    ) -> Vec<crate::display_list::PlacedCommand> {
        let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
        waterui_testing::install_test_executor();
        let mut renderer = DewRenderer::default();
        let list = renderer.render_tree(AnyView::new(view), &Environment::new(), width, height);
        list.commands().to_vec()
    }

    fn solid_brush(placed: &crate::display_list::PlacedCommand) -> peniko::Color {
        match placed.command() {
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

    fn assert_color_near(actual: peniko::Color, expected: peniko::Color) {
        assert!(
            actual
                .components
                .iter()
                .zip(expected.components)
                .all(|(actual, expected)| (actual - expected).abs() < 1.0e-6),
            "expected color near {expected:?}, got {actual:?}"
        );
    }

    /// An unlabeled on-toggle in a 200×40 context: accent pill trailing at
    /// the right edge, white thumb at the on end, thumb outline.
    #[test]
    fn toggle_emits_track_thumb_and_outline() {
        let commands = render_commands(
            Toggle::new("Ready", &binding(true)).hide_label(),
            200.0,
            40.0,
        );
        assert_eq!(commands.len(), 3, "track fill, thumb fill, thumb stroke");

        assert_color_near(solid_brush(&commands[0]), theme::ACCENT);
        assert_rect_near(commands[0].bounds(), Rect::new(149.0, 4.5, 200.0, 35.5));

        assert_color_near(solid_brush(&commands[1]), theme::THUMB);
        assert_point_near(commands[1].bounds().center(), Point::new(184.5, 20.0));

        assert_color_near(solid_brush(&commands[2]), theme::BORDER);
    }

    /// Turning the binding off swaps the track to the muted color and moves
    /// the thumb to the leading end.
    #[test]
    fn toggle_off_uses_muted_track_with_leading_thumb() {
        let commands = render_commands(
            Toggle::new("Ready", &binding(false)).hide_label(),
            200.0,
            40.0,
        );
        assert_color_near(solid_brush(&commands[0]), theme::TRACK);
        assert_point_near(commands[1].bounds().center(), Point::new(164.5, 20.0));
    }

    /// A hidden-label slider at value 0.5 in a 220×20 context: full track,
    /// accent fill to the midpoint, thumb centered on the fill end.
    #[test]
    fn slider_emits_track_fill_and_thumb() {
        let commands = render_commands(slider("Volume", &binding(0.5)).hide_label(), 220.0, 20.0);
        assert_eq!(commands.len(), 4, "track, fill, thumb fill, thumb stroke");

        assert_color_near(solid_brush(&commands[0]), theme::TRACK);
        assert_rect_near(commands[0].bounds(), Rect::new(10.0, 8.0, 210.0, 12.0));

        assert_color_near(solid_brush(&commands[1]), theme::ACCENT);
        assert_rect_near(commands[1].bounds(), Rect::new(10.0, 8.0, 110.0, 12.0));

        assert_color_near(solid_brush(&commands[2]), theme::THUMB);
        assert_point_near(commands[2].bounds().center(), Point::new(110.0, 10.0));

        assert_color_near(solid_brush(&commands[3]), theme::BORDER);
    }

    #[test]
    fn controls_read_installed_theme_colors() {
        let accent: nami::Binding<ResolvedColor> =
            binding(ResolvedColor::from_srgb(Srgb::new(1.0, 0.0, 0.0)));
        let mut env = Environment::new();
        waterui::theme::Theme::new()
            .colors(waterui::theme::ColorSettings::new().accent(accent.clone()))
            .install(&mut env);

        let mut renderer = DewRenderer::default();
        let commands = renderer.render_tree(
            AnyView::new(Toggle::new("Ready", &binding(true)).hide_label()),
            &env,
            200.0,
            40.0,
        );
        assert_color_near(
            solid_brush(&commands.commands()[0]),
            peniko::Color::from_rgb8(255, 0, 0),
        );

        accent.set(ResolvedColor::from_srgb(Srgb::new(0.0, 0.0, 1.0)));
        let commands = renderer.refresh_tree(200.0, 40.0);
        assert_color_near(
            solid_brush(&commands.commands()[0]),
            peniko::Color::from_rgb8(0, 0, 255),
        );
    }
}
