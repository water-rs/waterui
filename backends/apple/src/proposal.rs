//! The placement channel: how a container's [`Placement`] of a child —
//! the [`ProposalSize`] it selected and the [`SubpixelOffset`] its pixel
//! alignment left — reaches the leaf that placed view hosts.
//!
//! `SubView` has no placement hook — a Rust parent holding a mounted
//! child only sees its `&dyn SubView`. A leaf that consumes placements (a
//! layout container re-proposing and aligning its children) registers a
//! sink under its view here; the container delivering a placement asks
//! [`deliver`], which answers through the channel the child's leaf
//! installed.
//!
//! Both directions live here so the registry stays the single place the
//! view-pointer ↔ placement-channel mapping exists.
//!
//! # Safety
//!
//! A registration lives exactly as long as its [`SinkGuard`], which the
//! leaf drops when its `SubView` drops — so the map never answers for a
//! leaf that is gone.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use cocoa_ui::geometry::{EdgeInsets, MeasureProposal};
use cocoa_ui::response::{self, ResponseGeometry};
use cocoa_ui::view::{self, SubpixelOffset};
use cocoa_ui::{PlatformView, Point as KitPoint, Rect};
use waterui_core::layout::{
    LayoutError, PlacedGeometry, Point, ProposalSize, Rect as LayoutRect, Size, SubView, UnitPoint,
};

/// A `MeasureProposal` (f64 axes) as a layout `ProposalSize` (f32 axes).
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
pub fn to_proposal(proposal: MeasureProposal) -> ProposalSize {
    ProposalSize::new(
        proposal.width.map(|width| width as f32),
        proposal.height.map(|height| height as f32),
    )
}

/// A layout `ProposalSize` (f32 axes) as a `MeasureProposal` (f64 axes).
fn to_measure(proposal: ProposalSize) -> MeasureProposal {
    MeasureProposal {
        width: proposal.width.map(f64::from),
        height: proposal.height.map(f64::from),
    }
}

/// A kit rectangle as a layout `Rect`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
pub const fn layout_rect(rect: Rect) -> LayoutRect {
    LayoutRect::new(
        Point::new(rect.origin.x as f32, rect.origin.y as f32),
        Size::new(rect.size.width as f32, rect.size.height as f32),
    )
}

/// The proposal `view` was placed under: the one its parent delivered,
/// else — a view no Rust parent negotiated with — the boundary offer for
/// the rect it fills, as [`Placement::proposal`] states.
pub fn placement_proposal(view: &PlatformView, slot: &ResponseGeometry) -> ProposalSize {
    slot.placement_proposal().map_or_else(
        || {
            let allocation = allocation(view);
            ProposalSize::new(Some(allocation.width()), Some(allocation.height()))
        },
        to_proposal,
    )
}

/// The allocation `view`'s geometry is committed against, in its own
/// layout space: the frame its parent's layout assigned — its bounds,
/// less what the parent painted it past on any edge (§7.1: a safe-area
/// paint extension never moves the content).
///
/// The response stays allocation-local; [`in_view`] places it back in
/// the view.
pub fn allocation(view: &PlatformView) -> LayoutRect {
    let bounds = layout_rect(view::bounds(view));
    let past = slot_of(view).painted_past();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; kit geometry is f64"
    )]
    LayoutRect::new(
        Point::new(bounds.x() + past.left as f32, bounds.y() + past.top as f32),
        Size::new(
            bounds.width() - (past.left + past.right) as f32,
            bounds.height() - (past.top + past.bottom) as f32,
        ),
    )
}

/// The geometry a committed node retains (`docs/layout-spec.md` §7.2): the
/// placement proposal, the raw answer, the allocation and the
/// allocation-local response, written once per placement by whatever
/// placed the node and read by everything that draws or reports against
/// it — the node's own content layout and its accessibility frame.
#[derive(Clone, Debug, Default)]
pub struct CommittedGeometry(Rc<Cell<Option<PlacedGeometry>>>);

impl CommittedGeometry {
    /// Retains `geometry` as the committed placement of the view carrying
    /// `slot`. When its parent positions its response
    /// ([`Placement::anchor`]), the response keeps its extent and moves to
    /// that anchor inside the allocation; the answer is how far it moved,
    /// which a driver translates the node's children by (translation
    /// covariance, `docs/layout-spec.md` §2.4), so its content draws where
    /// its response sits.
    pub fn commit(&self, slot: &ResponseGeometry, mut geometry: PlacedGeometry) -> Point {
        let resolved = geometry.response_rect;
        if let Some(anchor) = slot.response_anchor() {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the layout contract is f32; kit geometry is f64"
            )]
            let anchor = UnitPoint::new(anchor.x as f32, anchor.y as f32);
            geometry.response_rect =
                LayoutRect::from_size(*geometry.allocation.size()).align(*resolved.size(), anchor);
        }
        self.0.set(Some(geometry));
        Point::new(
            geometry.response_rect.x() - resolved.x(),
            geometry.response_rect.y() - resolved.y(),
        )
    }

    /// The committed placement, `None` before the node was first placed.
    #[must_use]
    pub fn get(&self) -> Option<PlacedGeometry> {
        self.0.get()
    }

    /// Resolves and retains a leaf's geometry under its own rule
    /// (`SubView::resolve_extent`), for the allocation and proposal `view`
    /// was placed with.
    ///
    /// # Errors
    ///
    /// Whatever the face's `resolve_extent` reports.
    pub fn commit_leaf(
        &self,
        view: &PlatformView,
        slot: &ResponseGeometry,
        layout: &dyn SubView,
    ) -> Result<(), LayoutError> {
        let geometry =
            PlacedGeometry::resolve(layout, allocation(view), placement_proposal(view, slot))?;
        // A leaf draws its own content in its response, wherever it sits.
        let _moved = self.commit(slot, geometry);
        Ok(())
    }
}

/// The response geometry of `view`, a leaf view every class of which
/// carries one.
///
/// # Panics
///
/// When `view`'s class carries no response geometry.
fn slot_of(view: &PlatformView) -> &ResponseGeometry {
    response::of(view).expect("a laid-out leaf's view carries its response geometry")
}

/// Wires the leaf whose platform view is `view` and whose layout face is
/// `layout` to a fresh committed geometry (`docs/layout-spec.md` §2.4,
/// §7.2): each layout pass of the view first commits its response under
/// the face's rule for the allocation and proposal it was placed with, and
/// the committed response is what its content draws in and its
/// accessibility frame reports. A view whose class carries no response
/// geometry reports its platform frame. The face is held weakly for the
/// reason `install_intrinsic_measure` states; the leaf's `Drop` clears the
/// handlers through [`release`] before the face dies.
pub fn install(view: &PlatformView, layout: &Rc<dyn SubView>) -> CommittedGeometry {
    let geometry = CommittedGeometry::default();
    let Some(slot) = response::of(view) else {
        return geometry;
    };
    slot.set_commit_handler({
        let layout = Rc::downgrade(layout);
        let geometry = geometry.clone();
        move |view| {
            let layout = layout
                .upgrade()
                .expect("a leaf's layout face outlives its view's commit handler");
            geometry
                .commit_leaf(view, slot_of(view), &*layout)
                .expect("a placed leaf's layout face resolves its response");
        }
    });
    slot.set_response_handler({
        let geometry = geometry.clone();
        move |view| {
            geometry
                .get()
                .map(|placed| in_view(view, &placed, placed.response_rect))
        }
    });
    geometry
}

/// Hands the geometry of the leaf whose view is `view` to the container
/// driver that places its children: the driver commits it from each
/// pass's `place_layout` answer, so the view stops resolving it under the
/// leaf's own rule.
///
/// # Panics
///
/// When `view`'s class carries no response geometry — a driver lays its
/// children out in a `HostView`.
pub fn drive(view: &PlatformView) {
    slot_of(view).clear_commit_handler();
}

/// Drops the response-geometry handlers of a leaf's `view`, releasing the
/// geometry and the layout face they capture: the leaf let go of the view.
pub fn release(view: &PlatformView) {
    if let Some(slot) = response::of(view) {
        slot.clear();
    }
}

/// `view`'s committed response rectangle in its own coordinates: where a
/// leaf draws its intrinsic content (`docs/layout-spec.md` §6).
///
/// # Panics
///
/// When `view` lays its content out before its geometry was committed —
/// the commit runs first in every layout pass.
pub fn response(view: &PlatformView) -> Rect {
    slot_of(view)
        .response_rect(view)
        .expect("a leaf commits its geometry before laying out its content")
}

/// Where `view` draws its intrinsic content: its committed response
/// rectangle, or its bounds before its first placement committed one.
#[cfg(feature = "picture")]
pub fn content_rect(view: &PlatformView) -> Rect {
    slot_of(view).content_rect(view)
}

/// An allocation-local `region` of `view`'s committed geometry `placed`,
/// in the view's own coordinates.
pub fn in_view(view: &PlatformView, placed: &PlacedGeometry, region: LayoutRect) -> Rect {
    let origin = placed.allocation.origin();
    view::from_layout_space(view, view_rect(region.translated(origin.x, origin.y)))
}

/// A layout `Rect` as a kit rectangle.
pub fn view_rect(rect: LayoutRect) -> Rect {
    Rect::new(
        f64::from(rect.x()),
        f64::from(rect.y()),
        f64::from(rect.width()),
        f64::from(rect.height()),
    )
}

/// How a parent placed a view: what its children lay out under.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The proposal the parent layout selected — `selectedProposal` in the
    /// Swift port. `None` when the parent negotiated none, and the view
    /// builds its own offer from the rect it fills.
    pub proposal: Option<ProposalSize>,
    /// What the parent's pixel alignment of the view discarded, which the
    /// view's own children align under.
    pub offset: SubpixelOffset,
    /// Where the parent's rule positions the view's response inside the
    /// frame it assigns, in physical unit coordinates — a List row's
    /// content sits at its leading edge, centred vertically. `None` when
    /// the view's own rule positions it. A layout-transparent wrapper
    /// forwards it with the rest of the placement.
    pub anchor: Option<UnitPoint>,
    /// How far, in layout space, the frame the parent writes extends past
    /// the allocation its layout assigned — a safe-area paint extension
    /// (§7.1) the view draws and scrolls into while its geometry stays the
    /// allocation's. Zero on every edge when the frame is the allocation.
    pub painted_past: EdgeInsets,
}

impl Placement {
    /// A view no Rust parent has placed, nor any platform parent offered
    /// a proposal: the state every placement consumer starts in.
    pub const UNPLACED: Self = Self {
        proposal: None,
        offset: SubpixelOffset::ZERO,
        anchor: None,
        painted_past: EdgeInsets::ZERO,
    };

    /// A view offered `proposal` inside a parent the platform places on the
    /// device-pixel grid, such as a table cell, so its layout origin is its
    /// written origin.
    pub const fn platform_hosted(proposal: ProposalSize) -> Self {
        Self {
            proposal: Some(proposal),
            offset: SubpixelOffset::ZERO,
            anchor: None,
            painted_past: EdgeInsets::ZERO,
        }
    }

    /// The same placement with the view's response positioned by the
    /// parent's rule at `anchor` inside the frame it assigns.
    #[must_use]
    pub const fn anchored(self, anchor: UnitPoint) -> Self {
        Self {
            anchor: Some(anchor),
            ..self
        }
    }
}

type Channel = Rc<dyn Fn(Placement)>;

thread_local! {
    /// The channel a leaf's platform view answers placements through,
    /// keyed by the view's address.
    ///
    /// Views are main-thread objects and every caller is a layout pass, so a
    /// thread-local map needs no synchronization.
    static CHANNELS: RefCell<std::collections::HashMap<usize, Channel>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The map key for `view`: its stable address.
fn key(view: &PlatformView) -> usize {
    core::ptr::from_ref::<PlatformView>(view) as usize
}

/// Delivers the placement a parent layout selected for `view`.
///
/// Does nothing when the leaf registered no channel — the same answer the
/// Swift baseline's default `setPlacementProposal` gives.
pub fn deliver(view: &PlatformView, placement: Placement) {
    if let Some(slot) = response::of(view) {
        slot.set_placement(
            view,
            placement.proposal.map(to_measure),
            placement
                .anchor
                .map(|anchor| KitPoint::new(f64::from(anchor.x), f64::from(anchor.y))),
            placement.painted_past,
        );
    }
    let channel = CHANNELS.with(|channels| channels.borrow().get(&key(view)).cloned());
    if let Some(sink) = channel {
        sink(placement);
    }
}

/// Places `view` at `frame`, a rect in the layout space of a parent whose
/// own alignment discarded `parent`: aligns the frame to the device-pixel
/// grid, delivers `proposal` with the offset the alignment left, then
/// writes the aligned frame — so a child that lays out on the frame change
/// already holds its placement, and a placement change alone still marks
/// it for relayout. Returns the frame written.
pub fn place(
    view: &PlatformView,
    frame: Rect,
    proposal: Option<ProposalSize>,
    parent: SubpixelOffset,
) -> Rect {
    place_painted(view, frame, EdgeInsets::ZERO, proposal, parent)
}

/// [`place`] for a `frame` the parent paints `painted_past` beyond the
/// allocation its layout assigned (§7.1): the view is written at `frame`
/// and commits its geometry against the allocation inside it.
pub fn place_painted(
    view: &PlatformView,
    frame: Rect,
    painted_past: EdgeInsets,
    proposal: Option<ProposalSize>,
    parent: SubpixelOffset,
) -> Rect {
    let aligned = view::pixel_aligned_frame(view, frame, parent);
    deliver(
        view,
        Placement {
            proposal,
            offset: aligned.offset,
            anchor: None,
            painted_past,
        },
    );
    view::set_frame(view, aligned.frame);
    aligned.frame
}

/// Places the leaf `view` in `frame` with its response at the leading edge
/// of the frame, offered `proposal`: the trailing edge when `rtl`, the
/// frame's top staying the response's top. The frame stays the allocation,
/// so a leaf that draws in its response (a label) lands at the leading
/// edge rather than centred in a frame wider than its content.
pub fn place_leading(view: &PlatformView, frame: Rect, proposal: ProposalSize, rtl: bool) {
    let anchor = if rtl {
        UnitPoint::TOP_TRAILING
    } else {
        UnitPoint::TOP_LEADING
    };
    deliver(view, Placement::platform_hosted(proposal).anchored(anchor));
    view::set_frame(view, frame);
}

/// Registers `sink` as the channel `view` answers placements through; the
/// returned guard unregisters on drop.
#[cfg(any(
    feature = "border",
    feature = "clip_shape",
    feature = "container",
    feature = "context_menu",
    feature = "cursor",
    feature = "draggable",
    feature = "drop_destination",
    feature = "dynamic",
    feature = "dynamic_range",
    feature = "fixed_container",
    feature = "gesture",
    feature = "glass_background",
    feature = "hittable",
    feature = "ignore_safe_area",
    feature = "layout_priority",
    feature = "lifecycle_hook",
    feature = "material_background",
    feature = "menu",
    feature = "offset",
    feature = "on_event",
    feature = "on_key_press",
    feature = "opacity",
    feature = "retain",
    feature = "rotation",
    feature = "scale",
    feature = "scroll",
    feature = "secure",
    feature = "shadow",
    feature = "env",
    feature = "transition_sites",
))]
pub fn register_sink(view: &PlatformView, sink: impl Fn(Placement) + 'static) -> SinkGuard {
    CHANNELS.with(|channels| {
        channels.borrow_mut().insert(key(view), Rc::new(sink));
    });
    SinkGuard { view: key(view) }
}

/// The guard [`register_sink`] returns; drops the registration.
#[cfg(any(
    feature = "border",
    feature = "clip_shape",
    feature = "container",
    feature = "context_menu",
    feature = "cursor",
    feature = "draggable",
    feature = "drop_destination",
    feature = "dynamic",
    feature = "dynamic_range",
    feature = "fixed_container",
    feature = "gesture",
    feature = "glass_background",
    feature = "hittable",
    feature = "ignore_safe_area",
    feature = "layout_priority",
    feature = "lifecycle_hook",
    feature = "material_background",
    feature = "menu",
    feature = "offset",
    feature = "on_event",
    feature = "on_key_press",
    feature = "opacity",
    feature = "retain",
    feature = "rotation",
    feature = "scale",
    feature = "scroll",
    feature = "secure",
    feature = "shadow",
    feature = "env",
    feature = "transition_sites",
))]
#[derive(Debug)]
pub struct SinkGuard {
    view: usize,
}

#[cfg(any(
    feature = "border",
    feature = "clip_shape",
    feature = "container",
    feature = "context_menu",
    feature = "cursor",
    feature = "draggable",
    feature = "drop_destination",
    feature = "dynamic",
    feature = "dynamic_range",
    feature = "fixed_container",
    feature = "gesture",
    feature = "glass_background",
    feature = "hittable",
    feature = "ignore_safe_area",
    feature = "layout_priority",
    feature = "lifecycle_hook",
    feature = "material_background",
    feature = "menu",
    feature = "offset",
    feature = "on_event",
    feature = "on_key_press",
    feature = "opacity",
    feature = "retain",
    feature = "rotation",
    feature = "scale",
    feature = "scroll",
    feature = "secure",
    feature = "shadow",
    feature = "env",
    feature = "transition_sites",
))]
impl Drop for SinkGuard {
    fn drop(&mut self) {
        CHANNELS.with(|channels| {
            channels.borrow_mut().remove(&self.view);
        });
    }
}
