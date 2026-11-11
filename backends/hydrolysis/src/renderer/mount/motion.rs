//! Engine-run wrapper motion (water-rs/waterui#2203).
//!
//! A wrapper's opacity, offset, rotation and scale are layer properties the
//! engine animates on its own clock: each change reaches the layer as a
//! `Prop` carrying `WaterUI`'s curve and the instant it starts at on
//! Hydrolysis's frame clock. The host keeps the same track here, in the
//! same lanes, and evaluates it through the shared evaluator wherever it
//! needs the property's value at an instant — hit testing and
//! accessibility frames at event time, the settle queries — so nothing on
//! the UI thread samples these properties per frame.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use cherenkov_record::{
    Animatable, Animation, AnimationOrigin, AnimationStart, Lanes, Live, TrackOrigin,
    TransformComponents,
};
use nami::Signal;
use nami::watcher::{BoxWatcherGuard, Context, Metadata, WatcherManager, WatcherManagerGuard};
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_core::animation::ChangeAnimation;
use waterui_graphics::animation::engine_animation;

use super::placement::PlacementClock;
use crate::renderer::signals::SubscribedSnapshot;
use crate::time::Instant;

/// When a motion's running track is done, and what its owner does then —
/// the untyped part of a [`Motion`] the animation controller keeps to
/// answer settle queries and fire the one settle re-record, without ever
/// evaluating the track.
pub struct MotionStatus {
    /// The instant the running track is done, `None` at rest.
    until: Cell<Option<Instant>>,
    /// Whether the owner re-records once the track is done: armed by a
    /// change whose track can show content its rest value hides, or by a
    /// record that waits on the track (an exiting overlay).
    settle_record: Cell<bool>,
    /// Marks the owner for that re-record.
    record: Box<dyn Fn()>,
}

impl MotionStatus {
    /// Whether the track still runs at `now`.
    pub fn is_running(&self, now: Instant) -> bool {
        self.until.get().is_some_and(|until| now < until)
    }

    /// Re-records the owner once the running track is done: a record that
    /// decided on the track running — an exiting overlay kept drawn while
    /// its content animates — decides again when it ends.
    pub fn arm_settle(&self, now: Instant) {
        if self.is_running(now) {
            self.settle_record.set(true);
        }
    }

    /// Fires the settle re-record once the track is done at `now`; `true`
    /// when it fired.
    pub fn settle(&self, now: Instant) -> bool {
        if !self.settle_record.get() || self.is_running(now) {
            return false;
        }
        self.settle_record.set(false);
        (self.record)();
        true
    }
}

impl std::fmt::Debug for MotionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MotionStatus")
            .field("until", &self.until.get())
            .field("settle_record", &self.settle_record.get())
            .finish_non_exhaustive()
    }
}

/// A running track in the property's lanes: exactly the state the
/// engine's track for the same change holds — the same `from` and
/// `velocity` lanes, animation and start — so both evaluate alike.
#[derive(Clone, Copy)]
struct Track<T: Animatable> {
    from: T::Lanes,
    velocity: T::Lanes,
    animation: Animation,
    start: Instant,
}

impl<T: Animatable> Track<T> {
    /// `(position, velocity, done)` at `at`, through the shared evaluator,
    /// as the engine's sampler evaluates the same track: a done track is at
    /// `target` exactly, at rest.
    fn eval(&self, target: T, at: Instant) -> (T::Lanes, T::Lanes, bool) {
        // `duration_since` saturates at zero, as the engine's sampler reads
        // an instant before the start.
        let dt = at.duration_since(self.start).as_secs_f64();
        let (mut position, mut velocity) = (T::Lanes::zero(), T::Lanes::zero());
        let done = self
            .animation
            .timing()
            .expect("a wrapper property's animation is a curve or a spring")
            .sample_lanes(
                self.from.as_ref(),
                self.velocity.as_ref(),
                target.into_lanes().as_ref(),
                dt,
                position.as_mut(),
                velocity.as_mut(),
            );
        (position, velocity, done)
    }

    /// When the track is done: a curve once its duration passed, a spring
    /// from its slowest lane's settling time on, rounded up to the
    /// nanosecond so the instant itself evaluates done.
    fn until(&self, target: T) -> Instant {
        let after = match self.animation {
            Animation::Curve(curve) => curve.duration,
            Animation::Spring(spring) => {
                let target = target.into_lanes();
                let seconds = (0..T::Lanes::N)
                    .map(|lane| {
                        spring.settling_duration(
                            self.from.get(lane) - target.get(lane),
                            self.velocity.get(lane),
                        )
                    })
                    .fold(0.0_f64, f64::max);
                let settled = Duration::from_secs_f64(seconds);
                if settled.as_secs_f64() < seconds {
                    settled + Duration::from_nanos(1)
                } else {
                    settled
                }
            }
            Animation::Decay(_) => unreachable!("a wrapper property never decays"),
        };
        self.start + after
    }
}

struct MotionState<T: Animatable> {
    /// The value the property rests at: its signal's latest value.
    target: T,
    /// The track moving the property toward `target`, kept until the next
    /// change: a done track evaluates to `target`.
    track: Option<Track<T>>,
}

impl<T: Animatable> MotionState<T> {
    /// The property's `(position, velocity)` lanes at `at`.
    fn lanes_at(&self, at: Instant) -> (T::Lanes, T::Lanes) {
        self.track.map_or_else(
            || (self.target.into_lanes(), T::Lanes::zero()),
            |track| {
                let (position, velocity, _) = track.eval(self.target, at);
                (position, velocity)
            },
        )
    }

    /// The property's value at `at`, as the engine samples it: the done
    /// track's target exactly, otherwise the position's lanes.
    fn value_at(&self, at: Instant) -> T {
        self.track.map_or(self.target, |track| {
            let (position, _, done) = track.eval(self.target, at);
            if done {
                self.target
            } else {
                T::from_lanes(position)
            }
        })
    }
}

struct MotionCell<T: Animatable> {
    state: RefCell<MotionState<T>>,
    status: Rc<MotionStatus>,
    /// The engine layer properties bound to the motion.
    watchers: WatcherManager<T>,
    frames: FrameSignals,
    clock: Rc<PlacementClock>,
    /// Whether the property decides what the owner's record shows: an
    /// opacity's owner re-records when a change moves it to or from
    /// fully transparent.
    gates_record: bool,
    /// The subscription to the property's signal.
    source: RefCell<Option<BoxWatcherGuard>>,
}

/// One wrapper property's engine-run motion: the track the engine samples
/// on the render thread, held here so the host evaluates the same track at
/// the instants it asks about (water-rs/waterui#2203).
///
/// It follows `signal` from construction on. Every change starts a track at
/// the frame clock's instant, from the previous track's value and velocity
/// at that instant, and hands the engine the same change through
/// [`Motion::live`]: the value with the change's animation, an
/// [`AnimationStart`] at that instant and the track's [`AnimationOrigin`],
/// so the engine starts the very track this side holds. A change that
/// names no animation snaps.
pub struct Motion<T: Animatable>(Rc<MotionCell<T>>);

impl<T: Animatable> Clone for Motion<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Animatable + PartialEq> Motion<T> {
    /// A motion following `signal`, whose values map into the property
    /// through `map`. The engine reports the refresh rate its track needs
    /// while it runs, as it does for every track it samples. `record`
    /// marks the owner for a re-record. With `gates_record`, the owner
    /// re-records whenever a change or a track's end moves the property to
    /// or from zero: the opacity's own gate on what its record shows.
    pub(crate) fn new<S>(
        signal: &S,
        map: fn(f32) -> T,
        frames: FrameSignals,
        clock: Rc<PlacementClock>,
        record: Box<dyn Fn()>,
        gates_record: bool,
    ) -> Self
    where
        S: Signal<Output = f32> + 'static,
    {
        // Subscribed before the read, so no change between the two is lost;
        // the subscription's registration echo lands on the read value.
        let (subscription, observed) = SubscribedSnapshot::new(signal);
        let cell = Rc::new(MotionCell {
            state: RefCell::new(MotionState {
                target: map(observed),
                track: None,
            }),
            status: Rc::new(MotionStatus {
                until: Cell::new(None),
                settle_record: Cell::new(false),
                record,
            }),
            watchers: WatcherManager::new(),
            frames,
            clock,
            gates_record,
            source: RefCell::new(None),
        });
        let weak: Weak<MotionCell<T>> = Rc::downgrade(&cell);
        let guard = subscription.activate(move |update: Context<f32>| {
            if let Some(cell) = weak.upgrade() {
                let animation = ChangeAnimation::of(update.metadata())
                    .animation()
                    .map(|animation| engine_animation(&animation));
                Self(cell).change(map(update.into_value()), animation);
            }
        });
        *cell.source.borrow_mut() = Some(Box::new(guard));
        Self(cell)
    }

    /// The untyped status the animation controller keeps.
    pub fn status(&self) -> &Rc<MotionStatus> {
        &self.0.status
    }

    /// Whether `other` is this motion, whatever value it holds.
    pub fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// The value the property rests at.
    pub fn target(&self) -> T {
        self.0.state.borrow().target
    }

    /// The property's value at `at`, through the shared evaluator: what
    /// the engine samples for a frame presented at `at`.
    pub fn value_at(&self, at: Instant) -> T {
        self.0.state.borrow().value_at(at)
    }

    /// The largest value of the property's lane anywhere along the running
    /// track — its start and its target, between which a curve or a
    /// settling spring keeps the property up to an overshoot — or its rest
    /// value: whether content under an opacity can show at all before the
    /// track ends.
    pub fn reach(&self, now: Instant) -> f64 {
        self.reach_of(&self.0.state.borrow(), now)
    }

    fn reach_of(&self, state: &MotionState<T>, now: Instant) -> f64 {
        let rest = state.target.into_lanes().max_abs();
        match state.track {
            Some(track) if self.0.status.is_running(now) => rest.max(track.from.max_abs()),
            _ => rest,
        }
    }

    /// The property's live value for an engine layer binding. Bind it with
    /// [`Self::rebind`] right after, so a track running at bind time
    /// reaches the new binding.
    pub fn live(&self) -> Live<T> {
        Live::from(self.clone())
    }

    /// Hands a newly bound layer property the running track, if one runs.
    ///
    /// A binding starts at the value [`Self::live`] read and receives later
    /// changes only, so the running track reaches it as one: its origin,
    /// velocity, animation and start, whole — a layer bound mid-spring (a
    /// first mount, a device remount) runs the very track the host
    /// evaluates, velocity included.
    pub fn rebind(&self) {
        let now = self.0.frames.frame_clock();
        let state = self.0.state.borrow();
        let Some(track) = state.track.filter(|_| self.0.status.is_running(now)) else {
            return;
        };
        let target = state.target;
        drop(state);
        self.notify(target, Some(track));
    }

    /// Applies a change of the property to `target` under `animation`,
    /// at the frame clock's instant.
    fn change(&self, target: T, animation: Option<Animation>) {
        let now = self.0.frames.frame_clock();
        let mut state = self.0.state.borrow_mut();
        if state.target == target {
            return;
        }
        let reach_before = self.reach_of(&state, now);
        let track = animation.map(|animation| {
            let (from, velocity) = state.lanes_at(now);
            Track {
                from,
                velocity,
                animation,
                start: now,
            }
        });
        state.target = target;
        state.track = track;
        drop(state);
        let until = track.map(|track| track.until(target));
        self.0.status.until.set(until);
        self.0.clock.note_motion(until);
        if self.0.gates_record {
            let reach_after = self.reach(now);
            // Visibility is a function of the reach: zero hides.
            if (reach_before > 0.0) != (reach_after > 0.0) {
                (self.0.status.record)();
            }
            let rest = target.into_lanes().max_abs();
            self.0
                .status
                .settle_record
                .set((rest > 0.0) != (reach_after > 0.0));
        }
        self.notify(target, track);
    }

    /// Tells the bound layer properties: `target`, reached by `track` —
    /// its animation, its start and the state it starts from, so every
    /// binding runs the host's track exactly — or snapped.
    fn notify(&self, target: T, track: Option<Track<T>>) {
        let metadata = track.map_or_else(Metadata::new, |track| {
            Metadata::new()
                .with(track.animation)
                .with(AnimationStart(track.start))
                .with(AnimationOrigin(TrackOrigin::<T> {
                    from: track.from,
                    velocity: track.velocity,
                }))
        });
        self.0.watchers.notify_owned(Context::new(target, metadata));
    }
}

impl<T: Animatable + PartialEq> Signal for Motion<T> {
    type Output = T;
    type Guard = WatcherManagerGuard<T>;

    /// The value a binding starts from: the property's value at the frame
    /// clock's instant.
    fn snapshot(&self) -> T {
        self.value_at(self.0.frames.frame_clock())
    }

    fn watch(&self, watcher: impl Fn(Context<T>) + 'static) -> Self::Guard {
        self.0.watchers.register_as_guard(watcher)
    }
}

/// A layer's transform motion: the translation, rotation and scale
/// components its wrapper animates about a pivot, each axis a motion of
/// its own. The layer's transform is the engine's component composition
/// over the wrapper's place, so placement composes the very matrix the
/// engine draws with.
pub struct LayerMotion {
    pivot: Cell<kurbo::Vec2>,
    translation: [Option<Motion<f64>>; 2],
    rotation: Option<Motion<f64>>,
    scale: [Option<Motion<f64>>; 2],
    clock: Rc<PlacementClock>,
}

impl LayerMotion {
    /// An offset's motion: a translation axis per signal.
    pub(crate) const fn offset(x: Motion<f64>, y: Motion<f64>, clock: Rc<PlacementClock>) -> Self {
        Self::new([Some(x), Some(y)], None, [None, None], clock)
    }

    /// A rotation's motion, in radians, about the pivot.
    pub(crate) const fn rotation(angle: Motion<f64>, clock: Rc<PlacementClock>) -> Self {
        Self::new([None, None], Some(angle), [None, None], clock)
    }

    /// A scale's motion about the pivot, a track per axis.
    pub(crate) const fn scale(x: Motion<f64>, y: Motion<f64>, clock: Rc<PlacementClock>) -> Self {
        Self::new([None, None], None, [Some(x), Some(y)], clock)
    }

    const fn new(
        translation: [Option<Motion<f64>>; 2],
        rotation: Option<Motion<f64>>,
        scale: [Option<Motion<f64>>; 2],
        clock: Rc<PlacementClock>,
    ) -> Self {
        Self {
            pivot: Cell::new(kurbo::Vec2::ZERO),
            translation,
            rotation,
            scale,
            clock,
        }
    }

    /// The pivot rotation and scale turn about, in the layer's own space.
    pub const fn pivot(&self) -> kurbo::Vec2 {
        self.pivot.get()
    }

    /// Moves the pivot: the wrapper's anchor in its laid-out bounds.
    pub fn set_pivot(&self, pivot: kurbo::Vec2) {
        if self.pivot.replace(pivot) != pivot {
            self.clock.bump();
        }
    }

    /// The translation axes' motions, x first.
    pub const fn translation_axes(&self) -> &[Option<Motion<f64>>; 2] {
        &self.translation
    }

    /// The rotation's motion.
    pub const fn rotation_angle(&self) -> Option<&Motion<f64>> {
        self.rotation.as_ref()
    }

    /// The scale axes' motions, x first.
    pub const fn scale_axes(&self) -> &[Option<Motion<f64>>; 2] {
        &self.scale
    }

    /// Every motion the layer runs.
    pub fn motions(&self) -> impl Iterator<Item = &Motion<f64>> {
        self.translation
            .iter()
            .chain(std::iter::once(&self.rotation))
            .chain(&self.scale)
            .flatten()
    }

    /// The components at `at` — or at rest, with `None`.
    pub fn components(&self, at: Option<Instant>) -> TransformComponents {
        let value = |motion: &Option<Motion<f64>>, rest: f64| {
            motion.as_ref().map_or(rest, |motion| {
                at.map_or_else(|| motion.target(), |at| motion.value_at(at))
            })
        };
        TransformComponents {
            translation: kurbo::Vec2::new(
                value(&self.translation[0], 0.),
                value(&self.translation[1], 0.),
            ),
            rotation: value(&self.rotation, 0.),
            scale: kurbo::Vec2::new(value(&self.scale[0], 1.), value(&self.scale[1], 1.)),
            pivot: self.pivot.get(),
            ..TransformComponents::IDENTITY
        }
    }

    /// The layer's local transform over `base`, its place in the parent, at
    /// `at` — or at rest with `None`: the matrix the engine samples for a
    /// frame presented at `at`.
    pub fn matrix(&self, base: kurbo::Affine, at: Option<Instant>) -> kurbo::Affine {
        self.components(at).matrix(base)
    }
}

impl std::fmt::Debug for LayerMotion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerMotion")
            .field("pivot", &self.pivot.get())
            .field("at_rest", &self.components(None))
            .finish_non_exhaustive()
    }
}
