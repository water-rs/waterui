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
use core::cell::RefCell;

use cocoa_ui::view::SubpixelOffset;
use cocoa_ui::{PlatformView, Rect};
use waterui_core::layout::ProposalSize;

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
}

impl Placement {
    /// A view no Rust parent has placed, nor any platform parent offered
    /// a proposal: the state every placement consumer starts in.
    pub const UNPLACED: Self = Self {
        proposal: None,
        offset: SubpixelOffset::ZERO,
    };

    /// A view offered `proposal` inside a parent the platform places on the
    /// device-pixel grid, such as a table cell, so its layout origin is its
    /// written origin.
    pub const fn platform_hosted(proposal: ProposalSize) -> Self {
        Self {
            proposal: Some(proposal),
            offset: SubpixelOffset::ZERO,
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
    let aligned = cocoa_ui::view::pixel_aligned_frame(view, frame, parent);
    deliver(
        view,
        Placement {
            proposal,
            offset: aligned.offset,
        },
    );
    cocoa_ui::view::set_frame(view, aligned.frame);
    aligned.frame
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
