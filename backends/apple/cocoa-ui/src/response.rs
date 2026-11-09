//! The response geometry a leaf view carries for the layout that placed it.
//!
//! A view a layout placed answers its placement with a response rectangle
//! inside the frame it was assigned: the size it answered, positioned by
//! its layout's rule. A [`ResponseGeometry`] is where a leaf view keeps how
//! it was placed — the proposal and the anchor its parent placed it with —
//! and the owner's handlers that commit and answer the rectangle. The
//! classes that carry one report the rectangle as their accessibility
//! frame and draw their intrinsic content in it, while their own frame
//! stays the one their parent assigned.
//!
//! [`of`] finds the geometry of any view whose class carries one.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::DowncastTarget;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSAccessibilityFrameInView;
use objc2_core_foundation::CGRect;
#[cfg(target_os = "ios")]
use objc2_ui_kit::UIAccessibilityConvertFrameToScreenCoordinates;

use crate::PlatformView;
#[cfg(target_os = "macos")]
use crate::appkit::{
    ColorView, HostView, ImageView, Label, ListTableView, ScrollView, surface_view::SurfaceView,
    toggle::ToggleRow,
};
#[cfg(all(feature = "avkit", target_os = "macos"))]
use crate::avkit::PlayerContentView;
use crate::geometry::{EdgeInsets, MeasureProposal, Point, Rect};
#[cfg(feature = "map")]
use crate::map::MapView;
#[cfg(target_os = "ios")]
use crate::uikit::{
    ColorView, HostView, ImageView, Label, ScrollView, TableView as ListTableView,
    surface_view::SurfaceView, toggle::ToggleRow,
};
use crate::view;
use crate::web_kit::LeafWebView;

/// A handler run against the view that carries the geometry.
type ViewHandler = Rc<dyn Fn(&PlatformView)>;
/// A handler answering a rectangle in the view's own coordinates, or
/// `None` while the view holds no committed geometry.
type RectHandler = Rc<dyn Fn(&PlatformView) -> Option<Rect>>;

/// How a leaf view was placed, and the handlers through which its owner
/// commits and answers its response rectangle.
#[derive(Default)]
pub struct ResponseGeometry {
    /// The proposal the parent layout placed the view with.
    placement_proposal: Cell<Option<MeasureProposal>>,
    /// The unit point the parent's rule anchors the view's response at.
    response_anchor: Cell<Option<Point>>,
    /// How far, in layout space, the frame the parent wrote extends past
    /// the allocation its layout assigned — a safe-area paint extension.
    painted_past: Cell<EdgeInsets>,
    /// Commits the view's geometry at the start of every layout pass,
    /// before anything is placed or drawn against it.
    commit: RefCell<Option<ViewHandler>>,
    /// Answers the view's committed response rectangle.
    response: RefCell<Option<RectHandler>>,
    /// Names the rectangle the accessibility frame reports instead of the
    /// response.
    accessibility_frame: RefCell<Option<RectHandler>>,
}

impl fmt::Debug for ResponseGeometry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseGeometry")
            .field("placement_proposal", &self.placement_proposal.get())
            .field("response_anchor", &self.response_anchor.get())
            .field("painted_past", &self.painted_past.get())
            .field("commit", &self.commit.borrow().is_some())
            .field("response", &self.response.borrow().is_some())
            .field(
                "accessibility_frame",
                &self.accessibility_frame.borrow().is_some(),
            )
            .finish()
    }
}

impl ResponseGeometry {
    /// Records how the parent layout placed `view`: the proposal it
    /// selected — `None` for a view no Rust parent has placed — the unit
    /// point the parent's rule anchors the view's response at inside its
    /// frame, `None` when the view's own rule positions it, and how far
    /// the frame the parent writes extends past the allocation its layout
    /// assigned. A changed placement marks `view` for layout, so the
    /// geometry committed under it is never stale while the frame stays
    /// the same.
    pub fn set_placement(
        &self,
        view: &PlatformView,
        proposal: Option<MeasureProposal>,
        anchor: Option<Point>,
        painted_past: EdgeInsets,
    ) {
        let proposal_changed = self.placement_proposal.replace(proposal) != proposal;
        let anchor_changed = self.response_anchor.replace(anchor) != anchor;
        let painted_changed = self.painted_past.replace(painted_past) != painted_past;
        if proposal_changed || anchor_changed || painted_changed {
            view::set_needs_layout(view);
        }
    }

    /// How far, in layout space, the frame the parent wrote extends past
    /// the allocation its layout assigned: zero on every edge unless the
    /// parent paints the view past it.
    #[must_use]
    pub const fn painted_past(&self) -> EdgeInsets {
        self.painted_past.get()
    }

    /// The proposal the parent layout placed the view with, if any.
    #[must_use]
    pub const fn placement_proposal(&self) -> Option<MeasureProposal> {
        self.placement_proposal.get()
    }

    /// The unit point the parent's rule anchors the view's response at
    /// inside its frame, if the parent positions it.
    #[must_use]
    pub const fn response_anchor(&self) -> Option<Point> {
        self.response_anchor.get()
    }

    /// Calls `handler` at the start of every layout pass of the view: where
    /// the view's owner commits the geometry the view was placed with.
    pub fn set_commit_handler(&self, handler: impl Fn(&PlatformView) + 'static) {
        self.commit.replace(Some(Rc::new(handler)));
    }

    /// Drops the commit handler: the view's geometry is committed by the
    /// driver that places its children instead.
    pub fn clear_commit_handler(&self) {
        self.commit.replace(None);
    }

    /// Runs the commit handler against `view`, the view carrying this
    /// geometry — the first step of its layout pass.
    pub fn commit(&self, view: &PlatformView) {
        let handler = self.commit.borrow().clone();
        if let Some(handler) = handler {
            handler(view);
        }
    }

    /// Lets `handler` answer the view's committed response rectangle, in
    /// the view's own coordinates.
    pub fn set_response_handler(&self, handler: impl Fn(&PlatformView) -> Option<Rect> + 'static) {
        self.response.replace(Some(Rc::new(handler)));
    }

    /// `view`'s committed response rectangle in its own coordinates, or
    /// `None` while it holds no committed geometry.
    #[must_use]
    pub fn response_rect(&self, view: &PlatformView) -> Option<Rect> {
        let handler = self.response.borrow().clone();
        handler.and_then(|handler| handler(view))
    }

    /// Where `view` draws its intrinsic content: its committed response
    /// rectangle, or its bounds while no owner commits one — a kit view
    /// used outside a laid-out leaf draws in the frame it was given.
    #[must_use]
    pub fn content_rect(&self, view: &PlatformView) -> Rect {
        self.response_rect(view)
            .unwrap_or_else(|| view::bounds(view))
    }

    /// Lets `handler` name the rectangle the accessibility frame reports,
    /// in the view's own coordinates, instead of the response rectangle.
    pub fn set_accessibility_frame_handler(
        &self,
        handler: impl Fn(&PlatformView) -> Option<Rect> + 'static,
    ) {
        self.accessibility_frame.replace(Some(Rc::new(handler)));
    }

    /// The rectangle `view`'s accessibility frame reports, in screen
    /// coordinates: the named one, else the response; `None` while `view`
    /// holds no committed geometry, when the class reports what its
    /// superclass does.
    #[must_use]
    pub fn accessibility_frame(&self, view: &PlatformView) -> Option<CGRect> {
        let handler = self.accessibility_frame.borrow().clone();
        let rect = handler.map_or_else(|| self.response_rect(view), |handler| handler(view))?;
        #[cfg(target_os = "macos")]
        let frame = NSAccessibilityFrameInView(view, rect.into());
        #[cfg(target_os = "ios")]
        let frame = UIAccessibilityConvertFrameToScreenCoordinates(rect.into(), view);
        Some(frame)
    }

    /// Drops every handler, releasing what they capture: the leaf that
    /// owned the view let go of it.
    pub fn clear(&self) {
        self.commit.replace(None);
        self.response.replace(None);
        self.accessibility_frame.replace(None);
    }
}

/// A view class that carries a [`ResponseGeometry`].
pub trait ReportsResponse {
    /// The view's response geometry.
    fn response_geometry(&self) -> &ResponseGeometry;
}

/// The response geometry `view` carries as a `T`, when it is one.
fn class<T: ReportsResponse + DowncastTarget>(view: &PlatformView) -> Option<&ResponseGeometry> {
    view.downcast_ref::<T>()
        .map(ReportsResponse::response_geometry)
}

/// The response geometry `view` carries, when its class carries one.
#[must_use]
pub fn of(view: &PlatformView) -> Option<&ResponseGeometry> {
    let found = class::<HostView>(view)
        .or_else(|| class::<ImageView>(view))
        .or_else(|| class::<Label>(view))
        .or_else(|| class::<ToggleRow>(view))
        .or_else(|| class::<ColorView>(view))
        .or_else(|| class::<SurfaceView>(view))
        .or_else(|| class::<ScrollView>(view))
        .or_else(|| class::<ListTableView>(view))
        .or_else(|| class::<LeafWebView>(view));
    #[cfg(feature = "map")]
    let found = found.or_else(|| class::<MapView>(view));
    #[cfg(all(feature = "avkit", target_os = "macos"))]
    let found = found.or_else(|| class::<PlayerContentView>(view));
    found
}
