//! The retirement hand-off (§1.5): the outgoing visuals a ghost owns once
//! semantics end — pure pixels, no live code.

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
    /// Host bounds size plus the claim's direction, frozen at removal.
    pub geometry: TransitionGeometry,
}

impl core::fmt::Debug for RetiredVisual {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetiredVisual").finish_non_exhaustive()
    }
}

impl RetiredVisual {
    /// The ghost's visuals for `host` and its outgoing `root`.
    pub const fn new(
        host: Retained<HostView>,
        root: Retained<PlatformView>,
        geometry: TransitionGeometry,
    ) -> Self {
        Self {
            host,
            root,
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
