//! The retirement hand-off (§1.5): the outgoing visuals a ghost owns once
//! semantics end — pure pixels, no live code.

#[cfg(feature = "gpu_surface")]
use alloc::rc::Rc;

use cocoa_ui::{PlatformView, Retained, focus, view};
use waterui_core::transition::TransitionGeometry;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The outgoing visuals: the retired host — which owns `root` as a
/// subview — and the geometry the ghost samples against. Everything
/// semantic already ended when this exists: no `NativeLeaf`, `Mounted`,
/// watcher, or layout facet survives the hand-off.
pub struct RetiredVisual {
    /// The retired host — reparenting and detaching are the only moves
    /// left; `Drop` performs the transferred native detachment.
    pub host: Retained<HostView>,
    /// The outgoing content root — the ghost's transform child.
    pub root: Retained<PlatformView>,
    /// The `FrozenCapturable` owners installed into each armed surface's
    /// capturable slot during retirement (§2.2) — kept alive for the
    /// visual's whole lifetime so an enclosing capture keeps resolving
    /// them.
    _frozen: Frozen,
    /// Host bounds size plus the claim's direction, frozen at removal.
    pub geometry: TransitionGeometry,
}

impl core::fmt::Debug for RetiredVisual {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetiredVisual").finish_non_exhaustive()
    }
}

impl RetiredVisual {
    /// The ghost's visuals for `host` and its outgoing `root`, with the
    /// `FrozenCapturable`s retirement installed over the armed surfaces.
    pub const fn new(
        host: Retained<HostView>,
        root: Retained<PlatformView>,
        frozen: Frozen,
        geometry: TransitionGeometry,
    ) -> Self {
        Self {
            host,
            root,
            _frozen: frozen,
            geometry,
        }
    }
}

impl Drop for RetiredVisual {
    /// The transferred native detachment: the site dropped the
    /// `Mounted`s without detaching, so the ghost removes the host here.
    fn drop(&mut self) {
        view::remove_from_superview(&self.host);
    }
}

/// Every view in `root`'s subtree, pre-order.
fn for_each_descendant(root: &PlatformView, f: &mut impl FnMut(&PlatformView)) {
    f(root);
    for subview in view::subviews(root) {
        for_each_descendant(&subview, f);
    }
}

/// Resigns any first responder inside `root`'s subtree (§1.5 step 4) —
/// the only per-view walk the retirement needs: the retired host strips
/// hit testing, accessibility and first-responder membership itself.
pub fn resign_focus_within(root: &PlatformView) {
    for_each_descendant(root, &mut |view| {
        let _ = focus::resign_first_responder(view);
    });
}

/// What a retired visual keeps alive for its armed surfaces: the
/// `FrozenCapturable`s installed into their capturable slots, so an
/// enclosing capture keeps resolving them.
#[cfg(feature = "gpu_surface")]
pub type Frozen = Vec<Rc<cocoa_ui::capture::FrozenCapturable>>;
/// Without GPU surfaces a retired visual keeps nothing extra alive.
#[cfg(not(feature = "gpu_surface"))]
pub type Frozen = ();

/// One armed surface's hand-off: its view, the last frame it presented,
/// and its presentation layer — collected while the surface is still
/// live.
#[cfg(feature = "gpu_surface")]
type FrozenHandoff = (
    Retained<PlatformView>,
    Option<cocoa_ui::capture::FrozenSurface>,
    Retained<objc2_quartz_core::CAMetalLayer>,
);

/// Step 3 of the retirement sequence (§2.2): every capturable surface
/// inside the outgoing subtree gives up its last presented frame while
/// the surface is still live — `take_frozen_frame` runs before any
/// semantic teardown.
#[cfg(feature = "gpu_surface")]
pub fn collect_frozen_surfaces(content: &PlatformView) -> Vec<FrozenHandoff> {
    let mut surfaces = Vec::new();
    cocoa_ui::capture::for_each_capturable(content, &mut |view, surface| {
        let (frame, layer) = surface.take_frozen_frame();
        surfaces.push((view::retain_base(view), frame, layer));
    });
    surfaces
}

/// Step 7 of the retirement sequence (§2.2): after the leaf drop cleared
/// every live capturable slot, each armed surface's `FrozenCapturable`
/// takes the slot, so an enclosing live filter keeps resolving the
/// ghost's surfaces — and the ghost keeps showing each layer's last
/// contents natively.
#[cfg(feature = "gpu_surface")]
pub fn install_frozen_surfaces(
    env: &waterui_backend_core::Environment,
    surfaces: Vec<FrozenHandoff>,
) -> Frozen {
    if surfaces.is_empty() {
        return Vec::new();
    }
    // The device every frozen stand-in blits on: the subtree's GPU
    // runtime, which armed surfaces always render under.
    let device = crate::gpu_runtime::raw_metal_device(&crate::gpu_runtime::runtime(env).context());
    surfaces
        .into_iter()
        .map(|(view, frame, layer)| {
            let frozen = cocoa_ui::capture::FrozenCapturable::new(layer, frame, &device);
            let surface: Rc<dyn cocoa_ui::capture::CapturableSurface> = frozen.clone();
            cocoa_ui::capture::capturable_slot(&view)
                .expect("a capturable surface's view always carries the slot")
                .install(&surface);
            frozen
        })
        .collect()
}
