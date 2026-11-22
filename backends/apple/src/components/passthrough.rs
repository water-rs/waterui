//! A transparent container: a metadata wrapper whose platform side is only
//! a tag, a scope or nothing at all forwards its child's layout face and
//! lays the child out over its own bounds.

use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, StretchAxis, SubView, ViewDimensions,
};

use cocoa_ui::PlatformView;

use crate::contract::{Mounted, NativeLeaf};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// A transparent container's layout face: every answer the mounted
/// child's — `WuiNavigationTransitionTaggedView` forwards `stretchAxis`,
/// `layoutPriority` and `measure` unchanged.
struct PassthroughSubView {
    child: Mounted,
}

impl core::fmt::Debug for PassthroughSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PassthroughSubView").finish_non_exhaustive()
    }
}

impl SubView for PassthroughSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.child.layout().resolve_extent(bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.child.layout().priority()
    }
}

/// Renders `content` into a transparent host and returns the leaf with the
/// child mounted over the host's bounds.
pub fn passthrough_leaf(
    content: crate::contract::Held,
    ctx: &crate::contract::RenderContext<'_>,
) -> NativeLeaf {
    let host = HostView::new(ctx.mtm(), cocoa_ui::Rect::ZERO);
    let host_view: &PlatformView = &host;
    let mounted = ctx.realize(content).mount(host_view);
    crate::primary_content::forward(&host, mounted.view());
    host.set_layout_handler({
        let child = cocoa_ui::view::retain_base(mounted.view());
        move |view| cocoa_ui::view::set_frame(&child, cocoa_ui::view::bounds(view))
    });
    NativeLeaf::new(host_view, PassthroughSubView { child: mounted })
}
