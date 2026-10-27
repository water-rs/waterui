//! The transition host (§1.4): the wrapper every claimed instance lives
//! in. It is an ordinary `HostView` the site places; the content leaf
//! mounts inside it. The host's layout handler frames the content over
//! the host's whole bounds — as the `opacity` wrapper does — and then
//! re-applies the last sampled transform, since `AppKit` rewrites layer
//! geometry during layout. Retiring the host makes its subtree a ghost:
//! hit testing, accessibility, focus and the inspector walk all exclude
//! it while its views stay where the last layout put them.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use cocoa_ui::geometry::Rect;
use cocoa_ui::{PlatformView, Retained, view};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_core::transition::TransitionProperties;

use crate::contract::{Mounted, NativeLeaf};
use crate::proposal;
use crate::transition::property::PropertyTarget;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The wrapper every claimed instance lives in: the host view the site
/// places, the content leaf mounted inside it, and the property target a
/// transition samples onto. At removal the host retires and the content's
/// [`Mounted`] is taken out through `into_retired_leaf` — the retired
/// host then owns the outgoing native tree as pure visuals.
pub struct TransitionHost {
    /// The host view the site places; the ghost's `RetiredVisual` retains
    /// it after semantics end.
    pub host: Retained<HostView>,
    /// The content leaf, mounted into `host`. `None` after the retirement
    /// hand-off took it.
    pub content: RefCell<Option<Mounted>>,
    /// `= content.view()`, kept after semantics end — the ghost's
    /// transform child.
    pub content_view: Retained<PlatformView>,
    /// The property target every sample writes — host alpha plus the
    /// content child's transform.
    pub target: PropertyTarget,
    /// The last applied sample, re-applied after each layout pass.
    transform: Cell<TransitionProperties>,
}

impl core::fmt::Debug for TransitionHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransitionHost").finish_non_exhaustive()
    }
}

impl TransitionHost {
    /// Wraps `leaf` in a fresh host view and mounts it inside.
    pub fn new(mtm: cocoa_ui::MainThreadMarker, leaf: NativeLeaf) -> Self {
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = leaf.mount(&host);
        view::set_translates_autoresizing(mounted.view(), true);
        let content_view = view::retain_base(mounted.view());
        let target = PropertyTarget::new(host.clone(), content_view.clone());
        Self {
            host,
            content: RefCell::new(Some(mounted)),
            content_view,
            target,
            transform: Cell::new(TransitionProperties::IDENTITY),
        }
    }

    /// Writes `sample` and records it for the layout handler's re-apply.
    pub fn apply_sample(&self, sample: &TransitionProperties) {
        self.transform.set(*sample);
        self.target.apply(sample);
    }

    /// The wrapper leaf a site mounts: its view is `host`, its layout face
    /// forwards every answer to the content, and its keepalive holds the
    /// proposal sink that forwards the selected proposal to the content —
    /// the same forwarding every transparent wrapper makes.
    pub fn leaf(self: &Rc<Self>) -> NativeLeaf {
        self.host.set_layout_handler({
            let this = Rc::clone(self);
            move |view| {
                let bounds = view::bounds(view);
                view::set_frame(&this.content_view, bounds);
                this.target.apply(&this.transform.get());
            }
        });
        // §7.1: the primary-content chain descends through the wrapper.
        crate::primary_content::forward(&self.host, &self.content_view);
        let sink_guard = proposal::register_sink(&self.host, {
            let this = Rc::clone(self);
            move |selected| {
                proposal::deliver(&this.content_view, selected);
            }
        });
        let mut leaf = NativeLeaf::new(&*self.host, TransitionHostSubView(Rc::clone(self)));
        leaf.keep(sink_guard);
        leaf
    }
}

/// The wrapper's layout face: the content's answers everywhere. After the
/// retirement hand-off takes the content the face is dead — the wrapper's
/// handlers are cleared — but it answers the empty defaults for
/// completeness.
struct TransitionHostSubView(Rc<TransitionHost>);

impl core::fmt::Debug for TransitionHostSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransitionHostSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for TransitionHostSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.0.content.borrow().as_ref().map_or_else(
            || ViewDimensions::new(Size::new(0.0, 0.0)),
            |content| content.layout().measure(proposal),
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.0
            .content
            .borrow()
            .as_ref()
            .map_or(StretchAxis::None, |content| content.layout().stretch_axis())
    }

    fn priority(&self) -> i32 {
        self.0
            .content
            .borrow()
            .as_ref()
            .map_or(0, |content| content.layout().priority())
    }

    fn is_empty(&self) -> bool {
        self.0
            .content
            .borrow()
            .as_ref()
            .is_none_or(|content| content.layout().is_empty())
    }
}
