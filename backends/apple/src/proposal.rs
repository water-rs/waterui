//! The placement-proposal channel: how a container's selected
//! [`ProposalSize`] reaches a leaf's `setPlacementProposal`.
//!
//! `SubView` has no placement hook — a Rust parent holding a mounted
//! child only sees its `&dyn SubView`. A leaf that consumes placement
//! proposals (a layout container re-proposing its children) registers a
//! sink under its view here; the container delivering a proposal asks
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

use cocoa_ui::PlatformView;
use waterui_core::layout::ProposalSize;

type Channel = Rc<dyn Fn(ProposalSize)>;

thread_local! {
    /// The channel a leaf's platform view answers placement proposals
    /// through, keyed by the view's address.
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

/// Delivers the proposal a parent layout selected for `view`.
///
/// Does nothing when the leaf registered no channel — the same answer the
/// Swift baseline's default `setPlacementProposal` gives.
pub fn deliver(view: &PlatformView, proposal: ProposalSize) {
    deliver_key(key(view), proposal);
}

/// [`deliver`] by map key, for callers holding only the address.
pub fn deliver_key(view_key: usize, proposal: ProposalSize) {
    let channel = CHANNELS.with(|channels| channels.borrow().get(&view_key).cloned());
    if let Some(sink) = channel {
        sink(proposal);
    }
}

/// Registers `sink` as the channel `view` answers placement proposals
/// through; the returned guard unregisters on drop.
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
    feature = "secure",
    feature = "shadow",
    feature = "with_env"
))]
pub fn register_sink(view: &PlatformView, sink: impl Fn(ProposalSize) + 'static) -> SinkGuard {
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
    feature = "secure",
    feature = "shadow",
    feature = "with_env"
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
    feature = "secure",
    feature = "shadow",
    feature = "with_env"
))]
impl Drop for SinkGuard {
    fn drop(&mut self) {
        CHANNELS.with(|channels| {
            channels.borrow_mut().remove(&self.view);
        });
    }
}
