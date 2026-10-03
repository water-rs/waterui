//! The `dynamic_range` metadata: `Metadata<StandardDynamicRange>` and
//! `Metadata<HighDynamicRange>` wrapped around content whose backing layer
//! tree gets a `preferredDynamicRange` choice.
//!
//! Mirrors `WuiStandardDynamicRange`/`WuiHighDynamicRange`: a transparent
//! `HostView` container — measure, stretch, priority and the placement
//! proposal all answer for the mounted child — that tags itself and its
//! layer tree with the mode at construction and re-applies it on every
//! layout pass, so later-attached sublayers inherit it while a layer
//! already carrying its own tag keeps its local mode.

use alloc::rc::Rc;

use cocoa_ui::dynamic_range::{self, DynamicRange};
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::metadata::secure::{HighDynamicRange, StandardDynamicRange};
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct DynamicRangeState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for DynamicRangeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicRangeState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct DynamicRangeSubView {
    /// The leaf's state.
    state: Rc<DynamicRangeState>,
}

impl core::fmt::Debug for DynamicRangeSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicRangeSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for DynamicRangeSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

/// The transparent container leaf both mode handlers share: the mode tags
/// the wrapper and its layer tree at construction, and the layout handler
/// re-applies it so sublayers added later inherit while already-tagged
/// layers keep their own mode.
fn dynamic_range_leaf(
    mode: DynamicRange,
    content: waterui_backend_core::AnyView,
    ctx: &crate::contract::RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    let mounted = ctx.render(content).mount(host_view);
    crate::primary_content::forward(&host, mounted.view());
    view::set_translates_autoresizing(mounted.view(), true);

    dynamic_range::apply_to_view(mode, host_view);

    let state = Rc::new(DynamicRangeState { child: mounted });

    // Each layout pass stretches the content over the bounds and re-applies
    // the mode so newly attached sublayers inherit the override.
    host.set_layout_handler({
        let state = Rc::clone(&state);
        let host_view = Retained::from(host_view);
        move |host| {
            let bounds = view::bounds(host);
            view::set_frame(state.child.view(), bounds);
            dynamic_range::apply_to_view(mode, &host_view);
        }
    });

    // `setPlacementProposal`: the proposal selected for this wrapper is the
    // proposal its content was negotiated with.
    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        DynamicRangeSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.keep(state);
    leaf
}

/// Installs the two dynamic-range handlers on the dispatcher:
/// `Metadata<StandardDynamicRange>` pins the subtree to SDR,
/// `Metadata<HighDynamicRange>` pins it to HDR.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<StandardDynamicRange>>(|metadata, ctx| {
        dynamic_range_leaf(DynamicRange::Standard, metadata.content, ctx)
    });
    dispatcher.register_view::<Metadata<HighDynamicRange>>(|metadata, ctx| {
        dynamic_range_leaf(DynamicRange::High, metadata.content, ctx)
    });
}
