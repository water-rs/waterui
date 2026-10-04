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
//! class hosts: [`subscribe_visibility_wakes`] registers one re-check
//! closure on `view` and every emitting ancestor and hands back a
//! [`VisibilityWakes`] token set — dropping it detaches every link, so a
//! refresh after a reparent cannot leave the old chain alive. There is
//! no registry and nothing static; each emitter belongs to the view
//! instance that fires it. [`VisibilityWatch`] owns one mounted leaf's
//! full observation — that token set plus the enclosing scroll-viewport
//! watch — and rebinds both together.
//!
//! Ancestors outside the `CocoaUi` classes cannot emit, on either
//! platform. `UIKit`'s geometry and hidden key paths carry no
//! documented KVO guarantee, and toggling `AppKit`'s documented
//! `postsFrameChangedNotifications`/`postsBoundsChangedNotifications`
//! opt-in on a foreign `NSView` is unsound once several mounted leaves
//! share that ancestor — one leaf's detach could silence another's
//! stream. A foreign non-scroll ancestor's hidden, alpha, frame,
//! transform or reparent inside the same window therefore publishes
//! nothing a descendant can observe: embedding hosts that mutate such a
//! container must refresh the mounted instance explicitly — the
//! `WaterUIHostController.updateVisibility` contract on both platforms
//! (`waterui_apple_update_visibility` natively). Scroll ancestors wake
//! through [`crate::scroll::observe_scroll_viewport`], which covers
//! `UIScrollView`/`NSScrollView` subclasses and third-party scroll
//! views alike. The window-level half — the owning `UIWindowScene`'s
//! activation transitions, or an `AppKit` window's miniaturize,
//! occlusion, screen and close notifications — the platform posts as
//! ordinary notifications, so [`VisibilityWatch`] subscribes them
//! against the owning object and rebinds the moment the window (or
//! scene) changes; nothing else can re-run [`window_presentable`]
//! otherwise and a backgrounded scene would leave surfaces parked
//! forever.
//!
//! # Safety
//!
//! The `unsafe`-free geometry paths call public `UIKit`/`AppKit` rectangle
//! conversion APIs on the main thread; `unsafe` is used only where a
//! platform call is marked unsafe by `objc2` or where a notification
//! constant is a static `NSString` the framework owns.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;

use crate::PlatformView;
use crate::notification::NotificationName;
use crate::scroll::{ScrollObservation, ScrollView};

/// Whether `view` could show pixels right now.
///
/// `UIKit`: the view sits in a window whose owning scene is
/// foreground-active, neither it nor any ancestor is hidden or fully
/// transparent, and its window-space bounds intersect the window's
/// bounds and every ancestor that clips — `clipsToBounds` or
/// `layer.masksToBounds`, which `UIScrollView` sets.
///
/// `AppKit`: the view sits in a visible, un-miniaturized window on a
/// screen, no ancestor hides it or zeroes its alpha, and the native
/// `visibleRect` — `AppKit`'s own account of clipping ancestors —
/// intersects the view's bounds.
///
/// The test is deliberately conservative: intersecting bounding
/// rectangles only proves a pixel *may* be on screen. Exact opaque
/// occlusion and mask-path coverage are never claimed — `false` means
/// definitely invisible, the only answer frame clocks act on.
#[must_use]
pub fn presentable(view: &PlatformView) -> bool {
    imp::presentable(view)
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

// MARK: - Emitters

/// One subscriber slot on an emitter: the closure kept weakly so a
/// surface that dies stops waking, and an id its [`Subscription`] detaches.
struct Slot {
    id: u64,
    handler: Weak<dyn Fn()>,
}

/// The emitter's shared state, one `Rc` per view instance.
struct Shared {
    slots: RefCell<Vec<Slot>>,
    /// `emit` is running — a nested `emit` coalesces into one more pass
    /// instead of recursing into the handler list.
    emitting: Cell<bool>,
    /// A nested `emit` asked for another pass.
    pending: Cell<bool>,
    next_id: Cell<u64>,
}

/// The typed, instance-owned visibility event an owned view class hosts.
///
/// Subscribing returns a [`Subscription`] token; dropping it detaches the
/// closure from this emitter deterministically. The closure is also held
/// weakly, so one that dies without its token stops firing on the next
/// `emit` and is pruned.
pub struct VisibilityEmitter {
    shared: Rc<Shared>,
}

impl Default for VisibilityEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl VisibilityEmitter {
    /// An emitter with no subscribers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Rc::new(Shared {
                slots: RefCell::new(Vec::new()),
                emitting: Cell::new(false),
                pending: Cell::new(false),
                next_id: Cell::new(0),
            }),
        }
    }

    /// Runs every live subscription, then prunes the dead ones.
    ///
    /// Handlers are collected before any run, so a handler that
    /// subscribes or detaches mid-emit cannot invalidate the iteration.
    /// A handler that re-entrantly `emit`s does not recurse: it marks a
    /// pending pass, which this call then runs against the refreshed
    /// chain — removals a handler performed are already reflected.
    pub fn emit(&self) {
        if self.shared.emitting.replace(true) {
            self.shared.pending.set(true);
            return;
        }
        loop {
            let live: Vec<Rc<dyn Fn()>> = {
                let mut slots = self.shared.slots.borrow_mut();
                slots.retain(|slot| slot.handler.strong_count() > 0);
                slots
                    .iter()
                    .filter_map(|slot| slot.handler.upgrade())
                    .collect()
            };
            for handler in live {
                handler();
            }
            if !self.shared.pending.replace(false) {
                self.shared.emitting.set(false);
                return;
            }
        }
    }

    /// Registers `handler` until the returned token drops.
    fn subscribe(&self, handler: &Weak<dyn Fn()>) -> Subscription {
        let id = self.shared.next_id.get();
        self.shared.next_id.set(id + 1);
        self.shared.slots.borrow_mut().push(Slot {
            id,
            handler: handler.clone(),
        });
        let shared = Rc::downgrade(&self.shared);
        Subscription::new(move || {
            if let Some(shared) = shared.upgrade() {
                shared.slots.borrow_mut().retain(|slot| slot.id != id);
            }
        })
    }
}

impl std::fmt::Debug for VisibilityEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisibilityEmitter").finish_non_exhaustive()
    }
}

/// One registered wake's detach-on-drop handle.
#[must_use = "dropping detaches the subscription immediately"]
pub struct Subscription(Option<Box<dyn FnOnce()>>);

impl Subscription {
    fn new(detach: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(detach)))
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(detach) = self.0.take() {
            detach();
        }
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription").finish_non_exhaustive()
    }
}

/// Every wake one observation registered — drop detaches them all.
#[derive(Debug)]
#[must_use = "dropping detaches every registered wake"]
pub struct VisibilityWakes {
    _subscriptions: Vec<Subscription>,
}

/// Registers `handler` on `view` and every ancestor that can publish
/// visibility changes — the same ancestor chain [`presentable`] walks.
///
/// Each `CocoaUi` node subscribes through its emitter. Foreign
/// (non-`CocoaUi`) ancestors publish nothing here: mutating a foreign
/// view's notification flags is unsound once several mounted leaves
/// share the ancestor — one token's restore can silence another live
/// observation — so geometry, `hidden`/`alpha`, and reparent changes a
/// host makes outside `CocoaUi` containers are reported through the
/// explicit host visibility contract (`waterui_apple_update_visibility`
/// / `-[WaterUIHostController updateVisibility]`) instead. Scroll
/// ancestors stay automatic: [`crate::scroll::observe_scroll_viewport`]
/// rides public `UIScrollView`/`NSScrollView` APIs on any class.
///
/// The returned [`VisibilityWakes`] owns the whole chain: refresh by
/// dropping the old set — the new walk then binds only the ancestors
/// that currently enclose `view`.
pub fn subscribe_visibility_wakes(view: &PlatformView, handler: &Rc<dyn Fn()>) -> VisibilityWakes {
    let mut subscriptions = Vec::new();
    let mut ancestor: Option<Retained<PlatformView>> = Some(Retained::from(view));
    while let Some(current) = ancestor {
        #[cfg(target_os = "ios")]
        let next = current.superview();
        #[cfg(target_os = "macos")]
        // SAFETY: `superview` is a read-only accessor queried on the main
        // thread, as every visibility decision is.
        let next = unsafe { current.superview() };
        if let Some(emitter) = emitter_of(&current) {
            subscriptions.push(emitter.subscribe(&Rc::downgrade(handler)));
        }
        ancestor = next;
    }
    VisibilityWakes {
        _subscriptions: subscriptions,
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

// MARK: - The owned watch

/// One mounted leaf's owned visibility observation, rebound together
/// so a reparent or reattachment never leaves a stale link behind.
///
/// The observation is the subscribed ancestor chain plus the enclosing
/// scroll-viewport and owning window/scene lifecycle watches. Both
/// components that schedule frames keep one of these. The shared
/// `handler` closure runs on every ancestor emission, every scroll
/// move and every window/scene lifecycle notification;
/// [`VisibilityWatch::refresh`] re-walks the hierarchy when the tree
/// changes. Dropping the watch detaches everything — the surfaces'
/// state drop is the full teardown.
pub struct VisibilityWatch {
    /// The re-evaluate closure every wake in this observation shares.
    handler: Rc<dyn Fn()>,
    /// The currently bound ancestor chain; replaced whole on refresh.
    wakes: RefCell<Option<VisibilityWakes>>,
    /// The nearest scroll ancestor's viewport watch and the view it
    /// binds, compared by object identity.
    scroll: RefCell<Option<ScrollBinding>>,
    /// The owning window/scene's lifecycle observers, compared by the
    /// observed object's identity.
    window: RefCell<Option<WindowBinding>>,
}

impl std::fmt::Debug for VisibilityWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisibilityWatch").finish_non_exhaustive()
    }
}

struct ScrollBinding {
    _observation: ScrollObservation,
    /// The scroll ancestor this binding watches, kept weakly: the watch
    /// is owned by a descendant, so a strong retain here would loop the
    /// view hierarchy (`scroll → subtree → view → state → watch`).
    view: objc2::rc::Weak<ScrollView>,
}

impl std::fmt::Debug for ScrollBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollBinding").finish_non_exhaustive()
    }
}

/// The owning window/scene's lifecycle subscription: one notification
/// observer per name the platform posts for the transitions
/// [`window_presentable`] reads — `UIKit` scene activation (`Foreground`
/// — `Active` — `Background`) and `AppKit` window occlusion,
/// miniaturize, screen and close. Bound on the owning object so
/// notifications from sibling windows never leak into this leaf's wake.
struct WindowBinding {
    _observers: Vec<crate::notification::NotificationObserver>,
    /// The observed object — the owning `UIWindowScene` on iOS, the
    /// `NSWindow` itself on macOS — kept weakly for the same
    /// retain-cycle reason as the scroll view (`window → subtree → view
    /// → state → watch → object`).
    object: objc2::rc::Weak<AnyObject>,
}

impl std::fmt::Debug for WindowBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowBinding").finish_non_exhaustive()
    }
}

impl VisibilityWatch {
    /// Subscribes `handler` to `view`'s visibility chain and arms its
    /// scroll-viewport watch.
    #[must_use = "dropping the watch detaches every wake"]
    pub fn new(view: &PlatformView, handler: Rc<dyn Fn()>) -> Self {
        let watch = Self {
            handler,
            wakes: RefCell::new(None),
            scroll: RefCell::new(None),
            window: RefCell::new(None),
        };
        watch.refresh(view);
        watch
    }

    /// Rebinds against `view`'s current hierarchy: the old chain's
    /// tokens drop (detaching ancestors the view left behind), the new
    /// walk subscribes fresh, and the scroll and window watches each
    /// re-arm only when the object they bind changed. Each binding is
    /// decided independently — an unchanged scroll binding must not skip
    /// the window rebind (a leaf with no scroll ancestor would never
    /// subscribe the window watch at all), and vice versa. The whole
    /// sequence is synchronous on the main thread, so no wake can be
    /// lost between the detach and the re-subscribe.
    ///
    /// # Panics
    ///
    /// Off the main thread — the scroll and lifecycle registrations,
    /// like every `UIKit`/`AppKit` call here, are main-thread only.
    pub fn refresh(&self, view: &PlatformView) {
        self.wakes.borrow_mut().take();
        *self.wakes.borrow_mut() = Some(subscribe_visibility_wakes(view, &self.handler));

        let scroll = crate::scroll::enclosing_scroll_view(view);
        let mut slot = self.scroll.borrow_mut();
        let scroll_unchanged = match (slot.as_ref(), scroll.as_ref()) {
            (Some(binding), Some(scroll)) => {
                binding.view.load().as_ref().map(Retained::as_ptr) == Some(Retained::as_ptr(scroll))
            }
            (None, None) => true,
            _ => false,
        };
        if !scroll_unchanged {
            *slot = scroll.map(|scroll| ScrollBinding {
                _observation: crate::scroll::observe_scroll_viewport(&scroll, {
                    let handler = self.handler.clone();
                    move || handler()
                }),
                view: objc2::rc::Weak::new(&*scroll),
            });
        }
        drop(slot);

        let owner = imp::window_watch_owner(view);
        let mut window_slot = self.window.borrow_mut();
        let window_unchanged = match (window_slot.as_ref(), owner.as_ref()) {
            (Some(binding), Some(owner)) => {
                binding.object.load().as_ref().map(Retained::as_ptr)
                    == Some(Retained::as_ptr(owner))
            }
            (None, None) => true,
            _ => false,
        };
        if !window_unchanged {
            *window_slot = owner.map(|owner| {
                let mtm = crate::MainThreadMarker::new()
                    .expect("visibility refresh runs on the main thread");
                WindowBinding {
                    _observers: imp::window_lifecycle_names()
                        .iter()
                        .map(|name| {
                            crate::notification::observe_object(mtm, name, &owner, {
                                let handler = self.handler.clone();
                                move || handler()
                            })
                        })
                        .collect(),
                    object: objc2::rc::Weak::new(&*owner),
                }
            });
        }
    }
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
    use super::{NotificationName, intersect};
    use objc2::Message;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
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

    /// The object `UIKit` window-level watches bind: the owning
    /// `UIWindowScene`, whose activation state is the whole
    /// [`window_presentable`] answer. `None` while the view has no
    /// window or the window is unattached — both states are already
    /// not-presentable and the binding re-arms on the next wake.
    pub fn window_watch_owner(view: &UIView) -> Option<Retained<AnyObject>> {
        let scene = view.window()?.windowScene()?;
        Some(AsRef::<AnyObject>::as_ref(&*scene).retain())
    }

    /// The notifications [`window_presentable`] can change on, posted by
    /// `UIKit` on the scene object: every activation-state edge.
    pub fn window_lifecycle_names() -> Vec<NotificationName> {
        use objc2_ui_kit::{
            UISceneDidActivateNotification, UISceneDidEnterBackgroundNotification,
            UISceneWillDeactivateNotification, UISceneWillEnterForegroundNotification,
        };
        // SAFETY: each static is an `NSString` the framework owns.
        unsafe {
            [
                UISceneWillEnterForegroundNotification,
                UISceneDidActivateNotification,
                UISceneWillDeactivateNotification,
                UISceneDidEnterBackgroundNotification,
            ]
            .into_iter()
            .map(NotificationName::framework)
            .collect()
        }
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
    use super::{NotificationName, intersect};
    use objc2::Message;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSView, NSWindow};

    /// The object `AppKit` window-level watches bind: the `NSWindow`
    /// itself — occlusion, miniaturize, screen and visibility state all
    /// live on it. `None` while the view has no window.
    pub fn window_watch_owner(view: &NSView) -> Option<objc2::rc::Retained<AnyObject>> {
        let window = view.window()?;
        Some(AsRef::<AnyObject>::as_ref(&*window).retain())
    }

    /// The notifications [`window_presentable`] can change on, posted by
    /// `AppKit` on the window object: occlusion state, miniaturize both
    /// ways, screen reattachment, expose and close.
    pub fn window_lifecycle_names() -> Vec<NotificationName> {
        use objc2_app_kit::{
            NSWindowDidChangeOcclusionStateNotification, NSWindowDidChangeScreenNotification,
            NSWindowDidDeminiaturizeNotification, NSWindowDidExposeNotification,
            NSWindowDidMiniaturizeNotification, NSWindowWillCloseNotification,
            NSWindowWillMiniaturizeNotification,
        };
        // SAFETY: each static is an `NSString` the framework owns.
        unsafe {
            [
                NSWindowWillMiniaturizeNotification,
                NSWindowDidMiniaturizeNotification,
                NSWindowDidDeminiaturizeNotification,
                NSWindowDidChangeOcclusionStateNotification,
                NSWindowDidChangeScreenNotification,
                NSWindowDidExposeNotification,
                NSWindowWillCloseNotification,
            ]
            .into_iter()
            .map(NotificationName::framework)
            .collect()
        }
    }

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
