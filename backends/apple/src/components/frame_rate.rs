//! The `frame_rate` metadata: `Metadata<FrameRateVote>` wrapped around a child
//! contributes its active vote to the owning window while mounted and visible.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::fmt::{Debug, Formatter, Result as FmtResult};

use cocoa_ui::visibility::{VisibilityWatch, presentable};
use cocoa_ui::{PlatformView, Rect, Retained, view};
use waterui::animation::{FrameRate, FrameRateVote};
use waterui::reactive::Signal;
use waterui_core::Metadata;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::frame_rate::{VoteRegistration, WindowFrameRateVotes};
use crate::primary_content;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

struct FrameRateState {
    child: Mounted,
    view: Retained<PlatformView>,
    vote: Cell<Option<FrameRate>>,
    registration: Option<VoteRegistration>,
    visibility_watch: RefCell<Option<VisibilityWatch>>,
}

impl Debug for FrameRateState {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("FrameRateState").finish_non_exhaustive()
    }
}

impl FrameRateState {
    fn sync(&self) {
        let Some(registration) = &self.registration else {
            return;
        };
        let active = if presentable(&self.view) {
            self.vote.get()
        } else {
            None
        };
        registration.set(active);
    }
}

struct FrameRateSubView {
    state: Rc<FrameRateState>,
}

impl Debug for FrameRateSubView {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("FrameRateSubView").finish_non_exhaustive()
    }
}

impl SubView for FrameRateSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.state.child.layout().resolve_extent(bounds, proposal)
    }

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

/// Realizes `Metadata<FrameRateVote>` as a transparent wrapper that contributes
/// its vote to the enclosing window while mounted and visible.
pub fn realize_frame_rate_metadata(
    metadata: Metadata<FrameRateVote, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    let mounted = ctx.realize(metadata.content).mount(host_view);
    primary_content::forward(&host, mounted.view());
    view::set_translates_autoresizing(mounted.view(), true);

    let registration = ctx
        .env()
        .get::<WindowFrameRateVotes>()
        .map(WindowFrameRateVotes::register);
    let signal = metadata.value.signal().clone();
    let state = Rc::new(FrameRateState {
        child: mounted,
        view: view::retain_base(host_view),
        vote: Cell::new(signal.snapshot()),
        registration,
        visibility_watch: RefCell::new(None),
    });

    let watch = VisibilityWatch::new(
        host_view,
        Rc::new({
            let state = Rc::downgrade(&state);
            move || {
                if let Some(state) = state.upgrade() {
                    state.sync();
                }
            }
        }),
    );
    *state.visibility_watch.borrow_mut() = Some(watch);
    state.sync();

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host| {
            let bounds = view::bounds(host);
            view::set_frame(state.child.view(), bounds);
            if let Some(watch) = state.visibility_watch.borrow().as_ref() {
                watch.refresh(host);
            }
            state.sync();
        }
    });
    host.set_window_handler({
        let state = Rc::downgrade(&state);
        move |host| {
            if let Some(state) = state.upgrade() {
                if let Some(watch) = state.visibility_watch.borrow().as_ref() {
                    watch.refresh(host);
                }
                state.sync();
            }
        }
    });
    host.set_hidden_handler({
        let state = Rc::downgrade(&state);
        move |_, _| {
            if let Some(state) = state.upgrade() {
                state.sync();
            }
        }
    });

    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        FrameRateSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.watch(&signal, {
        let state = Rc::downgrade(&state);
        move |wctx| {
            if let Some(state) = state.upgrade() {
                state.vote.set(*wctx.value());
                state.sync();
            }
        }
    });
    leaf.keep(state);
    leaf
}
