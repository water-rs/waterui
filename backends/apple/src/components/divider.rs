//! The `divider` leaf: `Native<Divider>` realized as a 1-point separator line
//! oriented across the enclosing stack and tinted by the `Border` theme slot.

use waterui_controls::Divider;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_graphics::color::Color;
use waterui_layout::stack::Axis;

use crate::components::resolved_color::platform_color;
use crate::contract::{NativeLeaf, RenderContext};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::ColorView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::ColorView;

/// Divider stroke thickness: 1 pt, per `docs/layout-spec.md`.
const DIVIDER_THICKNESS: f32 = 1.0;

#[derive(Debug, Clone, Copy)]
struct DividerSubView {
    vertical: bool,
}

impl SubView for DividerSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let size = if self.vertical {
            Size::new(DIVIDER_THICKNESS, proposal.height.unwrap_or(0.0))
        } else {
            Size::new(proposal.width.unwrap_or(0.0), DIVIDER_THICKNESS)
        };
        ViewDimensions::new(size)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::CrossAxis
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Realizes `Divider` as a platform `ColorView` bound to `Color::border()` with
/// axis-aware 1-point thickness and `StretchAxis::CrossAxis`.
pub fn realize_divider(_divider: Divider, ctx: &RenderContext<'_>) -> NativeLeaf {
    let vertical = matches!(ctx.env().get::<Axis>(), Some(Axis::Horizontal));
    let view = ColorView::new(ctx.mtm());
    let mut leaf = NativeLeaf::new(&*view, DividerSubView { vertical });
    let resolved = Color::border().resolve(ctx.env());
    leaf.bind(&resolved, move |color| {
        view.set_color(Some(&platform_color(&color)));
    });
    leaf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_divider_is_one_point_tall_and_fills_proposed_width() {
        let divider = DividerSubView { vertical: false };
        assert_eq!(
            divider
                .measure(ProposalSize::new(Some(240.0), Some(40.0)))
                .size,
            Size::new(240.0, 1.0)
        );
        assert_eq!(
            divider.measure(ProposalSize::UNSPECIFIED).size,
            Size::new(0.0, 1.0)
        );
        assert_eq!(divider.stretch_axis(), StretchAxis::CrossAxis);
    }

    #[test]
    fn vertical_divider_is_one_point_wide_and_fills_proposed_height() {
        let divider = DividerSubView { vertical: true };
        assert_eq!(
            divider
                .measure(ProposalSize::new(Some(240.0), Some(40.0)))
                .size,
            Size::new(1.0, 40.0)
        );
        assert_eq!(
            divider.measure(ProposalSize::UNSPECIFIED).size,
            Size::new(1.0, 0.0)
        );
        assert_eq!(divider.stretch_axis(), StretchAxis::CrossAxis);
    }
}
