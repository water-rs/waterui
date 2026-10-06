//! The scroll animation state a kit scroll surface owns.
//!
//! At most one scroll animation runs per surface. An explicit `Animation`
//! (`Bezier`/`Spring`) drives the offset frame by frame on a
//! [`FrameClock`](crate::display_link::FrameClock): each tick evaluates
//! the curve and writes the **model** offset — `setContentOffset` on
//! `UIKit`, `scrollToPoint` on `AppKit` — so `didScroll` and `bounds`
//! observers follow the flight and lazily built content fills in as it
//! moves. The flight's landing is the surface's own jump to the target,
//! so landing equals the jump by construction. `AppKit`'s
//! `Animation::Default` instead rides an `NSAnimationContext` group on
//! the clip's `boundsOrigin`, which writes the model per frame itself.
//!
//! Either animation is superseded by a jump, a new request, or the
//! user's scroll — `AppKit` cancels through `scrollWheel:` and the pair
//! of live-scroll notifications, `UIKit` through its drag delegate — and
//! a view leaving its window lands the flight through
//! [`land`](ScrollFlight::land). [`ScrollFlight::cancel`] stops the
//! clock at the offset the last write left — the model is the
//! presentation, nothing needs committing — and retires a context group
//! by retargeting it to the current origin in a zero-duration group.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use objc2::MainThreadMarker;
use objc2_quartz_core::CACurrentMediaTime;

use crate::PlatformView;
use crate::display_link::FrameClock;
use crate::geometry::Point;

/// A flight in progress: `from` eased toward `to` on `progress`, tick by
/// tick, with the write the surface performs each frame.
struct Flight {
    /// The offset the flight started from.
    from: Point,
    /// `CACurrentMediaTime()` when the flight began.
    start: f64,
    /// What the flight runs — target, landing, curve and write.
    plan: FlightPlan,
    /// The generation `begin` stamped this flight with — a completion may
    /// only retire the state it was started under.
    generation: u64,
}

/// What [`ScrollFlight::begin`] parks: the target resolver, the landing
/// jump, the curve, and the per-tick write — the pieces a surface
/// assembles per animation request.
pub struct FlightPlan {
    /// The offset the flight lands at, resolved fresh each tick — a row
    /// target's position corrects as lazily built rows materialize
    /// mid-flight, and the flight must chase the true target, not the
    /// estimate the request resolved.
    pub to: Rc<dyn Fn() -> Point>,
    /// The flight's landing: the surface's own jump to the target, run on
    /// the last tick and whenever the flight is landed early — landing
    /// equals the jump by construction.
    pub land: Rc<dyn Fn()>,
    /// Seconds the whole curve spans; `elapsed >= duration` completes.
    pub duration: f64,
    /// Elapsed seconds → eased progress in `[0, 1]` (a spring may
    /// overshoot).
    pub progress: Rc<dyn Fn(f64) -> f64>,
    /// Writes the model offset for one tick — the surface's own
    /// unanimated write, so observers track the flight.
    pub write: Rc<dyn Fn(Point)>,
}

/// One scroll surface's animation state: a frame clock driving at most
/// one clocked flight, plus the context group `Animation::Default` rides
/// on `AppKit`.
///
/// Every mutating entry point first [`cancel`](Self::cancel)s whatever is
/// in flight — a jump, a new request, or the user's live scroll supersedes
/// at the same place, and the generation guard keeps a completion
/// callback from retiring a successor that parked mid-write.
pub struct ScrollFlight {
    /// The clock driving clocked flights.
    clock: FrameClock,
    /// The flight being eased tick by tick, or `None`.
    flight: RefCell<Option<Flight>>,
    /// The monotonic token every start takes.
    generation: Cell<u64>,
    /// This instance's weak self, captured by the context group's
    /// completion handler.
    #[cfg(target_os = "macos")]
    weak_self: Weak<Self>,
    /// The clip whose `boundsOrigin` the context animation plays, while
    /// one runs.
    #[cfg(target_os = "macos")]
    context_clip: RefCell<Option<objc2::rc::Weak<PlatformView>>>,
    /// The context group's generation, or 0 while none runs.
    #[cfg(target_os = "macos")]
    context: Cell<u64>,
}

impl std::fmt::Debug for ScrollFlight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollFlight")
            .field("flight", &self.flight.borrow().is_some())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl ScrollFlight {
    /// Builds the shared state `Rc` — the frame callback ticks through the
    /// weak self, so surfaces drop their `Rc` without a cycle.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Rc<Self> {
        Rc::new_cyclic(|weak: &Weak<Self>| Self {
            clock: FrameClock::new(mtm, {
                let weak = weak.clone();
                move || {
                    if let Some(this) = weak.upgrade() {
                        this.tick();
                    }
                }
            }),
            flight: RefCell::new(None),
            generation: Cell::new(0),
            #[cfg(target_os = "macos")]
            weak_self: weak.clone(),
            #[cfg(target_os = "macos")]
            context_clip: RefCell::new(None),
            #[cfg(target_os = "macos")]
            context: Cell::new(0),
        })
    }

    /// Starts a clocked flight from `from` to `to`: each tick resolves
    /// `to` fresh, evaluates `progress(elapsed_seconds)`, and `write`s
    /// the eased model offset; the last tick runs `land` — the surface's
    /// own jump — so landing equals the jump by construction. Whatever
    /// animation was in flight is superseded.
    ///
    /// `to` resolves per tick rather than once at the request because a
    /// list row's offset can be only an estimate until the rows the
    /// flight passes have materialized — the flight chases the corrected
    /// geometry instead of landing on the stale one. A surface whose row
    /// geometry is exact passes a constant `to`.
    ///
    /// A view with no window at all leaves the clock stopped — the
    /// `!is_running` arm lands the flight at once. An occluded or
    /// minimized window still has a screen: the link stays armed and the
    /// flight simply waits, resuming when the window shows again. A view
    /// that leaves its window entirely is landed by the surface's
    /// `viewDidMoveToWindow`/`didMoveToWindow` calling
    /// [`land`](Self::land).
    pub fn begin(&self, view: &PlatformView, from: Point, plan: FlightPlan) {
        self.cancel();
        if plan.duration <= 0.0 || from == (plan.to)() {
            (plan.land)();
            return;
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.flight.borrow_mut().replace(Flight {
            from,
            plan,
            start: CACurrentMediaTime(),
            generation,
        });
        self.clock.start(view);
        if !self.clock.is_running() {
            let flight = self.flight.borrow_mut().take();
            if let Some(flight) = flight {
                (flight.plan.land)();
            }
        }
    }

    /// Ends whatever is in flight. A clocked flight stops at the offset
    /// the last tick wrote — the model is the presentation, so the eye's
    /// position needs no commit; the context animation retargets to the
    /// current origin.
    pub fn cancel(&self) {
        self.flight.borrow_mut().take();
        self.clock.stop();
        #[cfg(target_os = "macos")]
        self.cancel_context();
    }

    /// Lands a parked flight exactly where a jump would have put it — the
    /// flight's `land` is the surface's own jump, which retires the
    /// flight through its cancel — and cancels a context group. A view
    /// that moves to another window or leaves it lands this way.
    pub fn land(&self) {
        #[cfg(target_os = "macos")]
        self.cancel_context();
        let land = self
            .flight
            .borrow()
            .as_ref()
            .map(|flight| Rc::clone(&flight.plan.land));
        if let Some(land) = land {
            land();
        }
    }

    /// Advances the parked flight one tick: the eased offset lands in the
    /// model first — the write is the surface's own unanimated one, so
    /// observers and lazily built content track every frame — and the
    /// completing tick lands the flight through the surface's own jump.
    fn tick(&self) {
        enum Action {
            /// Write the eased offset.
            Tick(Rc<dyn Fn(Point)>, Point),
            /// The schedule ended: the surface's jump to the target.
            Land(Rc<dyn Fn()>),
        }
        let now = self
            .clock
            .current_target_timestamp()
            .unwrap_or_else(|| CACurrentMediaTime());
        let (action, generation) = {
            let slot = self.flight.borrow();
            let Some(flight) = slot.as_ref() else {
                self.clock.stop();
                return;
            };
            let elapsed = (now - flight.start).max(0.0);
            let action = if elapsed >= flight.plan.duration {
                Action::Land(Rc::clone(&flight.plan.land))
            } else {
                let progress = (flight.plan.progress)(elapsed);
                let to = (flight.plan.to)();
                Action::Tick(
                    Rc::clone(&flight.plan.write),
                    Point::new(
                        (to.x - flight.from.x).mul_add(progress, flight.from.x),
                        (to.y - flight.from.y).mul_add(progress, flight.from.y),
                    ),
                )
            };
            (action, flight.generation)
        };
        // The borrow is released before the write or the landing runs:
        // both can synchronously re-enter `cancel`/`begin` — completing
        // under a held `RefCell` would panic or stop the successor's
        // clock.
        match action {
            Action::Tick(write, point) => write(point),
            Action::Land(land) => {
                land();
                // The landing's jump retired the flight itself; if its
                // surface was already gone and no jump ran, the same
                // generation is still parked — retire it here so the
                // clock stops.
                let mut slot = self.flight.borrow_mut();
                if slot
                    .as_ref()
                    .is_some_and(|flight| flight.generation == generation)
                {
                    slot.take();
                    drop(slot);
                    self.clock.stop();
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl ScrollFlight {
    /// The user-input cancellation every `AppKit` scroll surface
    /// installs: `NSScrollViewWillStartLiveScrollNotification` announces
    /// a gesture scroll, `NSScrollViewDidLiveScrollNotification` reports
    /// the bounds writes a user-initiated scroll makes — the pair covers
    /// wheel, trackpad, momentum and knob input on both modern and
    /// legacy devices, and neither posts for `scrollToPoint`, so a
    /// flight's own writes never self-cancel. The returned observers
    /// keep the subscription alive while they are retained.
    ///
    /// Keyboard paging and arrows are not covered: they scroll the clip
    /// through `NSClipView`'s own responder actions
    /// (`scrollPageUp:`/`scrollLineDown:`/…), an animated bounds write
    /// that posts no live-scroll notification and never travels to the
    /// scroll view, so nothing on an `NSScrollView` subclass sees that
    /// path. `NSViewBoundsDidChangeNotification` cannot substitute — it
    /// fires for a flight's own writes, and user and animator writes are
    /// indistinguishable.
    pub fn watch_user_scroll(
        self: &Rc<Self>,
        scroll_view: &objc2_app_kit::NSScrollView,
        mtm: MainThreadMarker,
    ) -> Vec<crate::notification::NotificationObserver> {
        use crate::notification::{NotificationName, observe_object};
        use objc2_app_kit::{
            NSScrollViewDidLiveScrollNotification, NSScrollViewWillStartLiveScrollNotification,
        };

        // SAFETY: both names are system notification constants.
        let names = unsafe {
            [
                NSScrollViewWillStartLiveScrollNotification,
                NSScrollViewDidLiveScrollNotification,
            ]
        };
        names
            .into_iter()
            .map(|name| {
                let weak = Rc::downgrade(self);
                observe_object(
                    mtm,
                    &NotificationName::framework(name),
                    scroll_view,
                    move || {
                        if let Some(flight) = weak.upgrade() {
                            flight.cancel();
                        }
                    },
                )
            })
            .collect()
    }

    /// `point` clamped to the clip's scrollable bounds — the only target
    /// an `AppKit` flight or context group may aim at, so neither
    /// overshoots the document.
    #[must_use]
    pub fn constrained(clip: &objc2_app_kit::NSClipView, point: Point) -> Point {
        clip.constrainBoundsRect(objc2_foundation::NSRect::new(
            point.into(),
            clip.bounds().size,
        ))
        .origin
        .into()
    }

    /// The jump every `AppKit` scroll surface shares: supersede whatever
    /// is in flight, write the clip, reflect it.
    pub fn jump_to(&self, scroll_view: &objc2_app_kit::NSScrollView, point: Point) {
        self.cancel();
        let clip = scroll_view.contentView();
        clip.scrollToPoint(point.into());
        scroll_view.reflectScrolledClipView(&clip);
    }

    /// The write a clocked `AppKit` flight performs each tick —
    /// `scrollToPoint` plus the `reflectScrolledClipView` that keeps the
    /// scrollers honest — weak to the scroll view so a parked flight
    /// never keeps its surface alive.
    pub fn clip_write(scroll_view: &objc2_app_kit::NSScrollView) -> Rc<dyn Fn(Point)> {
        let weak = objc2::rc::Weak::new(scroll_view);
        Rc::new(move |point| {
            if let Some(this) = weak.load() {
                let clip = this.contentView();
                clip.scrollToPoint(point.into());
                this.reflectScrolledClipView(&clip);
            }
        })
    }

    /// The `reflectScrolledClipView` the context group fires each frame —
    /// weak to the clip so the group never keeps its surface alive.
    pub fn context_reflect(clip: &objc2_app_kit::NSClipView) -> Rc<dyn Fn()> {
        let weak = objc2::rc::Weak::new(clip);
        Rc::new(move || {
            if let Some(clip) = weak.load()
                && let Some(scroll) = clip.enclosingScrollView()
            {
                scroll.reflectScrolledClipView(&clip);
            }
        })
    }

    /// `Animation::Default` on `AppKit`: `boundsOrigin` through the
    /// `animator()` proxy inside an `NSAnimationContext` group at the
    /// context's own duration — the platform's counterpart of
    /// `setContentOffset(_:animated: true)`. The group writes the model
    /// per frame, so `bounds` observers track the flight and the proxy's
    /// constraint lands exactly where the jump would.
    pub fn scroll_context(&self, clip: &PlatformView, to: Point, reflect: Rc<dyn Fn()>) {
        use block2::RcBlock;
        use core::ptr::NonNull;
        use objc2::rc::Retained;
        use objc2::{Message, msg_send};
        use objc2_app_kit::NSAnimationContext;
        use objc2_foundation::NSPoint;

        self.cancel();
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.context.set(generation);
        self.context_clip
            .borrow_mut()
            .replace(objc2::rc::Weak::from_retained(&clip.retain()));
        let weak_self = self.weak_self.clone();
        NSAnimationContext::runAnimationGroup_completionHandler(
            &RcBlock::new(move |ctx: NonNull<NSAnimationContext>| {
                // SAFETY: the context pointer is `AppKit`'s grouping
                // object, live for the block's duration.
                let ctx = unsafe { ctx.as_ref() };
                ctx.setAllowsImplicitAnimation(true);
                // SAFETY: `animator` returns this view's
                // `NSAnimatablePropertyContainer` proxy.
                let proxy: Retained<PlatformView> = unsafe { msg_send![clip, animator] };
                proxy.setBoundsOrigin(NSPoint::new(to.x, to.y));
                reflect();
            }),
            Some(&RcBlock::new(move || {
                if let Some(this) = weak_self.upgrade() {
                    this.finish_context(generation);
                }
            })),
        );
    }

    /// The group finished: the clip is at its destination; retire the
    /// state only if no newer animation superseded this one.
    fn finish_context(&self, generation: u64) {
        if self.context.get() == generation {
            self.context.set(0);
            self.context_clip.borrow_mut().take();
        }
    }

    /// Retires a running context group without committing the target —
    /// a zero-duration group retargets `boundsOrigin` to the origin the
    /// animation already moved the model to, so the eye's position stands
    /// as the user's scroll takes over.
    fn cancel_context(&self) {
        use block2::RcBlock;
        use core::ptr::NonNull;
        use objc2::msg_send;
        use objc2::rc::Retained;
        use objc2_app_kit::NSAnimationContext;

        if self.context.get() == 0 {
            return;
        }
        self.context.set(0);
        let Some(clip) = self
            .context_clip
            .borrow_mut()
            .take()
            .and_then(|weak| weak.load())
        else {
            return;
        };
        let current = clip.bounds().origin;
        NSAnimationContext::runAnimationGroup(&RcBlock::new(
            move |ctx: NonNull<NSAnimationContext>| {
                // SAFETY: the context pointer is `AppKit`'s grouping object,
                // live for the block's duration.
                let ctx = unsafe { ctx.as_ref() };
                ctx.setDuration(0.0);
                // SAFETY: `animator` returns this view's
                // `NSAnimatablePropertyContainer` proxy.
                let proxy: Retained<PlatformView> = unsafe { msg_send![&*clip, animator] };
                proxy.setBoundsOrigin(current);
            },
        ));
    }
}

#[cfg(target_os = "ios")]
impl ScrollFlight {
    /// Freezes `UIKit`'s own animation and momentum at the current offset
    /// — before a flight reads `from`, so a running `Default` scroll or a
    /// flick's deceleration does not fight the clocked writes.
    pub fn freeze_scroll(scroll_view: &objc2_ui_kit::UIScrollView) {
        scroll_view.setContentOffset_animated(scroll_view.contentOffset(), false);
    }

    /// The write a clocked `UIKit` flight performs each tick — the raw
    /// `setContentOffset`, never the cancelling wrapper — weak to the
    /// scroll view so a parked flight never keeps its surface alive.
    pub fn uikit_write(scroll_view: &objc2_ui_kit::UIScrollView) -> Rc<dyn Fn(Point)> {
        let weak = objc2::rc::Weak::new(scroll_view);
        Rc::new(move |point| {
            if let Some(this) = weak.load() {
                this.setContentOffset(point.into());
            }
        })
    }
}

#[cfg(feature = "native-test")]
impl ScrollFlight {
    /// Whether any scroll animation is in flight — the clocked flight or
    /// the context group — exposed to the native test suite only.
    #[must_use]
    pub fn in_flight(&self) -> bool {
        let animating = self.flight.borrow().is_some();
        #[cfg(target_os = "macos")]
        let animating = animating || self.context.get() != 0;
        animating
    }
}
