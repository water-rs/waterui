//! Presentation visibility and the wake chain behind it.
//!
//! [`presentable`] is the single query every autonomous frame source —
//! GPU presentation and filtered output alike — consults: could `view`
//! put pixels on the screen right now? A surface that answers `false`
//! parks its frame clock until a wake says the answer may have changed,
//! so scroll-clipped tiles stop submitting frames instead of rendering
//! into a clip region nobody sees.
//!
//! [`VisibilityEmitter`] is the instance-owned event a `CocoaUi` view
//! class hosts: a subtree `subscribe`s one re-check closure and the view
//! `emit`s it when its hidden, alpha, frame, bounds or hierarchy state
//! changes. [`subscribe_visibility_wakes`] registers that closure on
//! `view` and every emitting ancestor — there is no registry and nothing
//! static; each emitter belongs to the view instance that fires it.
//!
//! Ancestors outside the `CocoaUi` classes cannot emit. A scroll view —
//! ours or third-party — still wakes subscribers through
//! [`crate::scroll::observe_scroll_viewport`], which owns its observer
//! token per instance; a clip change inside a foreign container that is
//! not a scroll view (a resized map or web view ancestor) is a documented
//! gap, not claimed coverage.
//!
//! # Safety
//!
//! The `unsafe`-free geometry paths call public `UIKit`/`AppKit` rectangle
//! conversion APIs on the main thread; the impl blocks use `unsafe` only
//! where a platform call is marked unsafe by `objc2`.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;

use crate::PlatformView;

/// Whether `view` could show pixels right now.
///
/// `UIKit`: the view sits in a window of an active application, neither it
/// nor any ancestor is hidden or fully transparent, and its window-space
/// bounds intersect the window's bounds and every ancestor that clips —
/// `clipsToBounds` or `layer.masksToBounds`, which `UIScrollView` sets.
///
/// `AppKit`: the view sits in a visible, un-miniaturized window on a
/// screen, no ancestor hides it or zeroes its alpha, and the native
/// `visibleRect` — `AppKit`'s own account of clipping ancestors — is
/// nonempty.
///
/// The test is deliberately conservative: intersecting bounding
/// rectangles only proves a pixel *may* be on screen. Exact opaque
/// occlusion and mask-path coverage are never claimed — `false` means
/// definitely invisible, the only answer frame clocks act on.
#[must_use]
pub fn presentable(view: &PlatformView) -> bool {
    imp::presentable(view)
}

/// The typed, instance-owned visibility event an owned view class hosts.
///
/// `subscribe` keeps a weak handle on the closure, so a surface that is
/// reparented or torn down prunes itself on the next `emit` and can never
/// be woken through a stale ancestor.
#[derive(Default)]
pub struct VisibilityEmitter {
    handlers: RefCell<Vec<Weak<dyn Fn()>>>,
}

impl std::fmt::Debug for VisibilityEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisibilityEmitter").finish_non_exhaustive()
    }
}

impl VisibilityEmitter {
    /// Registers `handler`; subscribing the same closure twice is a no-op
    /// so refreshing a subscription set after a reparent is cheap.
    pub fn subscribe(&self, handler: &Rc<dyn Fn()>) {
        let weak = Rc::downgrade(handler);
        let mut handlers = self.handlers.borrow_mut();
        if !handlers.iter().any(|existing| existing.ptr_eq(&weak)) {
            handlers.push(weak);
        }
    }

    /// Runs every live subscription and prunes the dead ones.
    ///
    /// Handlers are collected before any run: a handler that re-entrantly
    /// subscribes or unsubscribes cannot invalidate the iteration, and a
    /// handler that emits again simply queues another pass.
    pub fn emit(&self) {
        let live: Vec<Rc<dyn Fn()>> = {
            let mut handlers = self.handlers.borrow_mut();
            let live: Vec<Rc<dyn Fn()>> = handlers.iter().filter_map(Weak::upgrade).collect();
            *handlers = live.iter().map(Rc::downgrade).collect();
            live
        };
        for handler in live {
            handler();
        }
    }
}

/// Registers `handler` on `view` and every ancestor that can emit
/// visibility changes — the same ancestor chain [`presentable`] walks.
///
/// Subscriptions deduplicate per closure, so re-running after a reparent
/// only attaches to newly enclosing emitters; stale ancestors keep a weak
/// handle that never fires again once `handler` is dropped.
pub fn subscribe_visibility_wakes(view: &PlatformView, handler: &Rc<dyn Fn()>) {
    let mut ancestor: Option<Retained<PlatformView>> = Some(Retained::from(view));
    while let Some(current) = ancestor {
        if let Some(emitter) = emitter_of(&current) {
            emitter.subscribe(handler);
        }
        #[cfg(target_os = "macos")]
        {
            // SAFETY: `superview` is a read-only accessor queried on the
            // main thread, as every visibility decision is.
            ancestor = unsafe { current.superview() };
        }
        #[cfg(target_os = "ios")]
        {
            ancestor = current.superview();
        }
    }
}

/// The emitter `view` hosts, if it is one of the `CocoaUi` classes that
/// publish visibility events.
fn emitter_of(view: &PlatformView) -> Option<&VisibilityEmitter> {
    #[cfg(target_os = "ios")]
    {
        if let Some(host) = view.downcast_ref::<crate::uikit::HostView>() {
            return Some(host.visibility_emitter());
        }
        if let Some(scroll) = view.downcast_ref::<crate::uikit::ScrollView>() {
            return Some(scroll.visibility_emitter());
        }
        if let Some(surface) = view.downcast_ref::<crate::uikit::surface_view::SurfaceView>() {
            return Some(surface.visibility_emitter());
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(host) = view.downcast_ref::<crate::appkit::HostView>() {
            return Some(host.visibility_emitter());
        }
        if let Some(scroll) = view.downcast_ref::<crate::appkit::ScrollView>() {
            return Some(scroll.visibility_emitter());
        }
        if let Some(surface) = view.downcast_ref::<crate::appkit::surface_view::SurfaceView>() {
            return Some(surface.visibility_emitter());
        }
    }
    None
}

/// Whether `window` can put a frame in front of someone — the
/// window-level half of [`presentable`], shared by the weaker attach and
/// buffer-allocation gates that must not wait for full visibility.
///
/// `UIKit`: the window's *owning* `UIWindowScene` is foreground-active —
/// not merely the process reporting `UIApplication.isActive`. On a
/// multi-scene session (iPad windows side by side) a background or
/// unattached scene must not schedule its surfaces at all (#1327).
///
/// `AppKit`: the window is visible, un-miniaturized and on a screen.
#[must_use]
pub fn window_presentable(
    #[cfg(target_os = "ios")] window: &objc2_ui_kit::UIWindow,
    #[cfg(target_os = "macos")] window: &objc2_app_kit::NSWindow,
) -> bool {
    imp::window_presentable(window)
}

/// The intersection of `a` and `b`; `None` when they share no area.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn intersect(
    a: objc2_core_foundation::CGRect,
    b: objc2_core_foundation::CGRect,
) -> Option<objc2_core_foundation::CGRect> {
    use objc2_core_foundation::{CGPoint, CGSize};
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    let width = (a.origin.x + a.size.width).min(b.origin.x + b.size.width) - x;
    let height = (a.origin.y + a.size.height).min(b.origin.y + b.size.height) - y;
    (width > 0.0 && height > 0.0)
        .then(|| objc2_core_foundation::CGRect::new(CGPoint::new(x, y), CGSize::new(width, height)))
}

#[cfg(target_os = "ios")]
mod imp {
    use super::intersect;
    use objc2_ui_kit::{UISceneActivationState, UIView, UIWindow};

    /// `clipsToBounds` or `layer.masksToBounds` — the ancestor states that
    /// clip descendants, which `UIScrollView` sets by default.
    fn clips_descendants(view: &UIView) -> bool {
        view.clipsToBounds() || view.layer().masksToBounds()
    }

    /// `UIKit`: the owning scene answers for its window — a window with no
    /// scene (`windowScene` nil, e.g. not yet attached) cannot present.
    /// `ForegroundActive` is the only activation state that displays
    /// frames; `ForegroundInactive`, `Background` and `Unattached` all
    /// defer, and the scene lifecycle notifications re-run the check.
    pub fn window_presentable(window: &UIWindow) -> bool {
        window.windowScene().is_some_and(|scene| {
            scene.activationState() == UISceneActivationState::ForegroundActive
        })
    }

    /// `UIKit`: windowed on a foreground-active scene, the ancestor chain
    /// clear of hidden or transparent links, and the window-space bounds
    /// inside the window and every clipping ancestor's window-space bounds.
    pub fn presentable(view: &UIView) -> bool {
        let Some(window) = view.window() else {
            return false;
        };
        if !window_presentable(&window) {
            return false;
        }
        if view.isHidden() || view.alpha() <= 0.0 {
            return false;
        }
        // `convertRect:toView:nil` — every rectangle lands in the window's
        // base coordinate space, whatever transforms sit between.
        let rect = view.convertRect_toView(view.bounds(), None);
        let mut zone = window.bounds();
        let mut ancestor = view.superview();
        while let Some(candidate) = ancestor {
            if candidate.isHidden() || candidate.alpha() <= 0.0 {
                return false;
            }
            if clips_descendants(&candidate) {
                let clip = candidate.convertRect_toView(candidate.bounds(), None);
                match intersect(zone, clip) {
                    Some(next) => zone = next,
                    None => return false,
                }
            }
            ancestor = candidate.superview();
        }
        intersect(zone, rect).is_some()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::intersect;
    use objc2_app_kit::{NSView, NSWindow};

    /// `AppKit`: the window can present — visible, un-miniaturized and on
    /// a screen (`screen` is nil for a window no display can show).
    pub fn window_presentable(window: &NSWindow) -> bool {
        crate::appkit::is_visible(window) && !window.isMiniaturized() && window.screen().is_some()
    }

    /// `AppKit`: windowed on a presentable window, the ancestor chain
    /// clear of hidden or transparent links, and some of the view's own
    /// bounds inside its native `visibleRect` — `AppKit` maps the clip's
    /// rect into the view's space without intersecting the view's bounds,
    /// so the intersection is the honest emptiness test.
    pub fn presentable(view: &NSView) -> bool {
        let Some(window) = view.window() else {
            return false;
        };
        if !window_presentable(&window) {
            return false;
        }
        let mut ancestor: Option<objc2::rc::Retained<NSView>> =
            Some(objc2::rc::Retained::from(view));
        while let Some(candidate) = ancestor {
            if candidate.isHidden() || candidate.alphaValue() <= 0.0 {
                return false;
            }
            // SAFETY: `superview` is a read-only accessor queried on the
            // main thread, as every visibility decision is.
            ancestor = unsafe { candidate.superview() };
        }
        // `visibleRect` answers the clip's bounds mapped into the view's
        // coordinate space without intersecting the view's own bounds — a
        // fully clipped view can report a nonzero rect disjoint from
        // `bounds`, so the honest test is the intersection itself.
        intersect(view.bounds(), view.visibleRect()).is_some()
    }
}
