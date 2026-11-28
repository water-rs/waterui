//! The `mask` metadata: `Metadata<Mask<Held>, Held>` masking wrapped content
//! with the alpha channel of another view laid out in the content's bounds.
//!
//! Layout is transparent: measure, stretch, priority, `is_empty`, and placement
//! proposals all answer for the wrapped content. On every layout pass, the
//! mask view is measured against the content's bounds and centered within them.
//!
//! Realization:
//! - On `UIKit` (iOS), a `mask_host` `HostView` is attached via `UIView.maskView`.
//! - On `AppKit` (macOS), `content_host`'s backing layer carries `mask_host`'s
//!   backing layer as `CALayer.mask`, kept in sync on layout, window attachment,
//!   and backing-scale changes.

use alloc::rc::Rc;

use cocoa_ui::shape;
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui_core::Metadata;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};
use waterui_shape::Mask;

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

#[cfg(target_os = "macos")]
struct MaskInvalidationGuard {
    mask_host: Retained<HostView>,
    _callback: Rc<dyn Fn()>,
}

#[cfg(target_os = "macos")]
impl Drop for MaskInvalidationGuard {
    fn drop(&mut self) {
        crate::invalidation::unregister_sink(&self.mask_host);
    }
}

struct MaskState {
    content: Mounted,
    mask: Mounted,
    #[cfg(target_os = "macos")]
    content_host: Retained<HostView>,
    mask_host: Retained<HostView>,
    #[cfg(target_os = "macos")]
    mask_layer: shape::ViewMaskLayer,
}

impl core::fmt::Debug for MaskState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaskState").finish_non_exhaustive()
    }
}

struct MaskSubView {
    state: Rc<MaskState>,
}

impl core::fmt::Debug for MaskSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaskSubView").finish_non_exhaustive()
    }
}

impl SubView for MaskSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.state.content.layout().resolve_extent(bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.content.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.content.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.content.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.content.layout().is_empty()
    }
}

fn layout_mask_host(host: &HostView, state: &MaskState, bounds: Rect) {
    let width = bounds.size.width.max(0.0);
    let height = bounds.size.height.max(0.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "WaterUI layout coordinates operate in f32 points"
    )]
    let mask_proposal = ProposalSize::new(Some(width as f32), Some(height as f32));
    proposal::deliver(
        state.mask.view(),
        proposal::Placement::platform_hosted(mask_proposal),
    );

    let measured = state.mask.layout().measure(mask_proposal).size;
    let mask_w = if measured.width.is_infinite() {
        width
    } else {
        f64::from(measured.width.max(0.0))
    };
    let mask_h = if measured.height.is_infinite() {
        height
    } else {
        f64::from(measured.height.max(0.0))
    };
    let mask_frame = Rect::new(
        (width - mask_w) * 0.5,
        (height - mask_h) * 0.5,
        mask_w,
        mask_h,
    );

    view::set_frame(&state.mask_host, bounds);
    view::set_frame(state.mask.view(), mask_frame);

    #[cfg(target_os = "macos")]
    {
        state
            .mask_layer
            .sync(host, &state.mask_host, bounds, host.display_scale());
    }
    #[cfg(target_os = "ios")]
    {
        let _ = host;
    }
}

#[cfg(target_os = "macos")]
fn install_macos_mask_hooks(
    host: &Retained<HostView>,
    state: &Rc<MaskState>,
    leaf: &mut NativeLeaf,
) {
    let window_state = Rc::clone(state);
    host.set_window_handler(move |host| {
        window_state.mask_layer.sync(
            host,
            &window_state.mask_host,
            view::bounds(host),
            host.display_scale(),
        );
    });
    let backing_state = Rc::clone(state);
    host.set_backing_changed_handler(move |host| {
        backing_state.mask_layer.sync(
            host,
            &backing_state.mask_host,
            view::bounds(host),
            host.display_scale(),
        );
    });
    let callback: Rc<dyn Fn()> = Rc::new({
        let host = host.clone();
        let state = Rc::clone(state);
        move || {
            let bounds = view::bounds(&host);
            layout_mask_host(&host, &state, bounds);
            crate::invalidation::invalidate_rendered_content(&host);
        }
    });
    crate::invalidation::register_sink(&state.mask_host, Rc::clone(&callback));
    leaf.keep(MaskInvalidationGuard {
        mask_host: state.mask_host.clone(),
        _callback: callback,
    });
}

/// Realizes `Metadata<Mask<Held>, Held>` as a transparent container whose
/// content is alpha-masked by the mask view.
pub fn realize_mask_metadata(
    metadata: Metadata<Mask<Held>, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    let mask_host = HostView::new(mtm, Rect::ZERO);
    let mask_mounted = ctx.realize(metadata.value.into_mask()).mount(&mask_host);
    view::set_translates_autoresizing(mask_mounted.view(), true);

    #[cfg(target_os = "macos")]
    let state = {
        view::ensure_layer_backed(&host);
        let content_host = HostView::new(mtm, Rect::ZERO);
        view::ensure_layer_backed(&content_host);
        view::ensure_layer_backed(&mask_host);
        view::add_subview(host_view, &content_host);
        view::set_translates_autoresizing(&content_host, true);

        let content = ctx.realize(metadata.content).mount(&content_host);
        crate::primary_content::forward(&host, content.view());
        view::set_translates_autoresizing(content.view(), true);

        let mask_layer = shape::ViewMaskLayer::new(mtm, view::retain_base(&*mask_host));
        shape::set_mask(&content_host, Some(&mask_layer.layer()));

        Rc::new(MaskState {
            content,
            mask: mask_mounted,
            content_host,
            mask_host,
            mask_layer,
        })
    };

    #[cfg(target_os = "ios")]
    let state = {
        let content = ctx.realize(metadata.content).mount(host_view);
        crate::primary_content::forward(&host, content.view());
        view::set_translates_autoresizing(content.view(), true);
        shape::set_mask_view(host_view, Some(&mask_host));

        Rc::new(MaskState {
            content,
            mask: mask_mounted,
            mask_host,
        })
    };

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host| {
            let bounds = view::bounds(host);
            #[cfg(target_os = "macos")]
            {
                view::set_frame(&state.content_host, bounds);
            }
            view::set_frame(state.content.view(), bounds);
            layout_mask_host(host, &state, bounds);
        }
    });

    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.content.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        MaskSubView {
            state: Rc::clone(&state),
        },
    );
    #[cfg(target_os = "macos")]
    install_macos_mask_hooks(&host, &state, &mut leaf);
    leaf.keep(sink_guard);
    leaf.keep(state);
    leaf
}
