use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::prelude::theme_color::{MutedForeground, Surface};
use waterui::shape::RoundedRectangle;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_icon::SystemIcon;
use waterui_layout::stack::zstack;

use crate::renderer::{
    HydroNativeView, HydroState, WidgetRenderContext, measure_view_dimensions_with_proposal,
    measure_view_intrinsic, normalize_view_for_render,
};

/// Side length of the unsupported-`SystemIcon` placeholder marker, in points.
/// Matches the Material standard icon size so the marker occupies the slot a real
/// icon would.
const ICON_SIZE: f32 = 24.0;
/// Outline thickness of the placeholder marker.
const ICON_BORDER: f32 = 2.0;
/// Corner radius of the placeholder marker.
const ICON_RADIUS: f32 = 6.0;

/// `SystemIcon` resolves names against an OS-provided icon catalog (SF Symbols).
/// Hydrolysis draws its own pixels and has no such catalog, so it cannot render the
/// real symbol. Rather than crashing, it renders a neutral "missing glyph" marker
/// (an outlined rounded square in theme tokens) that surfaces the asymmetry without
/// faking the icon — portable code should depend on a packaged icon crate
/// (`waterui-icons-material-icon`, `waterui-icons-lucide`, `waterui-icons-fontawesome7`)
/// instead. The marker carries the requested symbol name as its accessibility label.
fn system_icon_placeholder(icon: &SystemIcon, env: &Environment) -> AnyView {
    let inner = ICON_SIZE - ICON_BORDER * 2.0;
    normalize_view_for_render(
        AnyView::new(
            zstack((
                ().size(ICON_SIZE, ICON_SIZE)
                    .background(MutedForeground)
                    .clip(RoundedRectangle::new(ICON_RADIUS)),
                ().size(inner, inner)
                    .background(Surface)
                    .clip(RoundedRectangle::new(ICON_RADIUS - ICON_BORDER)),
            ))
            .a11y_role(AccessibilityRole::Image)
            .a11y_label(icon.name.as_str().to_owned()),
        ),
        env,
    )
}

impl HydroNativeView for Native<SystemIcon> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let content = system_icon_placeholder(view.as_inner(), env);
        ctx.dispatch_in_rect(env, content, ctx.bounds);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_view_intrinsic(&system_icon_placeholder(view.as_inner(), env), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        measure_view_dimensions_with_proposal(
            &system_icon_placeholder(view.as_inner(), env),
            proposal,
            state,
            env,
        )
    }

    fn accessibility_is_render_driven() -> bool {
        true
    }
}
