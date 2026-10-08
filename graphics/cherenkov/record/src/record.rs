//! Recording: the drawing verbs, the two recorders, and the change set a
//! commit sends to the render thread.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::{needs_drop, size_of};
use std::rc::{Rc, Weak};

use kurbo::{Affine, Rect, Stroke};
use nami_core::Signal;

use crate::Instant;
use crate::animation::{AnimLanes, Animation, OperandTrack};
use crate::display_list::{
    Command, DisplayList, DisplayListView, Operand, Picture, Slot, SlotUpdate,
};
use crate::glyph::GlyphRun;
use crate::material::{
    BackdropMaterial, CaptureClass, LayeredContent, MaterialEffect, MaterialRun, MaterialScope,
    MaterialShader,
};
use crate::paint::{ImageId, Paint, Sampling};
use crate::resource::ResourceId;
use crate::shape::Shape;
use crate::size::LayoutSize;
use crate::style::{Group, Shadow};
use crate::text::TextLayoutId;

/// The drawing verbs, shared by [`StaticRecorder`] and [`Recorder`].
///
/// [`Draw::Value`] decides what a parameter accepts: a plain value for
/// [`StaticRecorder`], any nami [`Signal`] for [`Recorder`]. Constants are
/// signals, so a plain value works with both. Wrap an owned constant in
/// [`Fixed`] to move it without an extra signal snapshot.
///
/// State changes are closure scopes only: [`clip`](Draw::clip),
/// [`transform`](Draw::transform) and [`group`](Draw::group) take the body
/// that runs inside them, so a scope cannot be left open.
pub trait Draw {
    /// What a parameter of type `T` accepts.
    type Value<T: 'static>;

    /// Fills a shape.
    fn fill<S: Shape, P: Into<Paint> + 'static>(
        &mut self,
        shape: impl Into<Self::Value<S>>,
        paint: impl Into<Self::Value<P>>,
    );

    /// Strokes a shape.
    fn stroke<S: Shape, P: Into<Paint> + 'static>(
        &mut self,
        shape: impl Into<Self::Value<S>>,
        stroke: impl Into<Self::Value<Stroke>>,
        paint: impl Into<Self::Value<P>>,
    );

    /// Casts a shadow from a shape.
    fn shadow<S: Shape>(
        &mut self,
        shape: impl Into<Self::Value<S>>,
        shadow: impl Into<Self::Value<Shadow>>,
    );

    /// Draws a glyph run.
    fn glyphs<P: Into<Paint> + 'static>(
        &mut self,
        run: impl Into<Self::Value<GlyphRun>>,
        paint: impl Into<Self::Value<P>>,
    );

    /// Draws an image into a rectangle.
    fn image(&mut self, image: ImageId, dst: impl Into<Self::Value<Rect>>, sampling: Sampling);

    /// Draws a shared picture.
    fn picture(&mut self, picture: &Picture, transform: impl Into<Self::Value<Affine>>);

    /// Draws a text layout the render target registered, placed by
    /// `transform`. The layout carries its own colours and spans.
    fn text(&mut self, layout: TextLayoutId, transform: impl Into<Self::Value<Affine>>);

    /// Runs `body` clipped to a shape.
    fn clip<S: Shape>(&mut self, shape: impl Into<Self::Value<S>>, body: impl FnOnce(&mut Self));

    /// Runs `body` under a transform.
    fn transform(
        &mut self,
        transform: impl Into<Self::Value<Affine>>,
        body: impl FnOnce(&mut Self),
    );

    /// Runs `body` isolated as a group.
    fn group(&mut self, group: impl Into<Self::Value<Group>>, body: impl FnOnce(&mut Self));
}

/// A fixed value accepted by either recorder.
///
/// [`Recorder`] consumes the value without taking a signal snapshot or
/// creating a subscription. This avoids cloning owned data such as glyph runs.
#[derive(Clone, Copy, Debug)]
pub struct Fixed<T>(pub T);

impl<T> From<T> for Fixed<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

/// Records constants on any thread into a [`Picture`].
#[derive(Debug, Default)]
pub struct StaticRecorder {
    list: DisplayList,
}

impl Picture {
    /// Records a picture. Pictures hold no signals, so they can be recorded on
    /// any thread and shared by any number of layers.
    #[must_use]
    pub fn record(body: impl FnOnce(&mut StaticRecorder)) -> Self {
        let mut recorder = StaticRecorder::default();
        body(&mut recorder);
        Self::from_list(recorder.list)
    }
}

impl Draw for StaticRecorder {
    type Value<T: 'static> = Fixed<T>;

    fn fill<S: Shape, P: Into<Paint> + 'static>(
        &mut self,
        shape: impl Into<Fixed<S>>,
        paint: impl Into<Fixed<P>>,
    ) {
        self.list.push(Command::Fill {
            shape: shape.into().0.into_data(),
            paint: paint.into().0.into(),
        });
    }

    fn stroke<S: Shape, P: Into<Paint> + 'static>(
        &mut self,
        shape: impl Into<Fixed<S>>,
        stroke: impl Into<Fixed<Stroke>>,
        paint: impl Into<Fixed<P>>,
    ) {
        self.list.push(Command::Stroke {
            shape: shape.into().0.into_data(),
            stroke: stroke.into().0,
            paint: paint.into().0.into(),
        });
    }

    fn shadow<S: Shape>(&mut self, shape: impl Into<Fixed<S>>, shadow: impl Into<Fixed<Shadow>>) {
        self.list.push(Command::Shadow {
            shape: shape.into().0.into_data(),
            shadow: shadow.into().0,
        });
    }

    fn glyphs<P: Into<Paint> + 'static>(
        &mut self,
        run: impl Into<Fixed<GlyphRun>>,
        paint: impl Into<Fixed<P>>,
    ) {
        self.list.push(Command::Glyphs {
            run: run.into().0,
            paint: paint.into().0.into(),
        });
    }

    fn image(&mut self, image: ImageId, dst: impl Into<Fixed<Rect>>, sampling: Sampling) {
        self.list.push(Command::Image {
            image,
            dst: dst.into().0,
            sampling,
        });
    }

    fn picture(&mut self, picture: &Picture, transform: impl Into<Fixed<Affine>>) {
        self.list.push(Command::Picture {
            picture: picture.clone(),
            transform: transform.into().0,
        });
    }

    fn text(&mut self, layout: TextLayoutId, transform: impl Into<Fixed<Affine>>) {
        self.list.push(Command::Text {
            layout,
            transform: transform.into().0,
        });
    }

    fn clip<S: Shape>(&mut self, shape: impl Into<Fixed<S>>, body: impl FnOnce(&mut Self)) {
        let begin = self.list.push(Command::BeginClip {
            shape: shape.into().0.into_data(),
            end: 0,
        });
        body(self);
        self.list.end(begin);
    }

    fn transform(&mut self, transform: impl Into<Fixed<Affine>>, body: impl FnOnce(&mut Self)) {
        let begin = self.list.push(Command::BeginTransform {
            transform: transform.into().0,
            end: 0,
        });
        body(self);
        self.list.end(begin);
    }

    fn group(&mut self, group: impl Into<Fixed<Group>>, body: impl FnOnce(&mut Self)) {
        let begin = self.list.push(Command::BeginGroup {
            group: group.into().0,
            end: 0,
        });
        body(self);
        self.list.end(begin);
    }
}

/// A signal consumer. Recorded slots keep their concrete callback state inline;
/// property bindings additionally receive animation metadata.
struct Watch<T> {
    destination: Destination<T>,
}

impl<T> std::fmt::Debug for Watch<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch").finish_non_exhaustive()
    }
}

enum Destination<T> {
    Slot {
        state: Weak<LiveState>,
        command: u32,
        convert: fn(T) -> Operand,
    },
    Binding(Box<dyn Fn(nami_core::watcher::Context<T>)>),
}

impl<T> Watch<T> {
    // The destination `Live::watch` subscriptions notify through.
    fn binding(callback: impl Fn(nami_core::watcher::Context<T>) + 'static) -> Self {
        Self {
            destination: Destination::Binding(Box::new(callback)),
        }
    }

    fn notify(&self, context: nami_core::watcher::Context<T>) {
        match &self.destination {
            Destination::Slot {
                state,
                command,
                convert,
            } => {
                if let Some(state) = state.upgrade() {
                    // The `Context` metadata carries the animation a bound
                    // signal was set under, exactly like a layer property's
                    // binding op does.
                    let animation = context.metadata().try_get::<Animation>();
                    let value = convert(context.into_value());
                    match animation {
                        Some(animation) => state.animate(*command, value, animation),
                        None => state.snap(SlotUpdate {
                            command: *command,
                            value,
                        }),
                    }
                }
            }
            Destination::Binding(callback) => callback(context),
        }
    }
}

/// A signal's subscription factory and the guard keeping it alive.
/// `Rc` so a `Live` is `Clone`: a host may bind one `Live` to more than
/// one layer property — or rebind it — and each binding gets its own
/// watch of the shared subscription.
#[derive(Clone)]
struct Subscribe<T>(Option<Rc<dyn Subscription<T>>>);

impl<T> std::fmt::Debug for Subscribe<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Subscribe").finish_non_exhaustive()
    }
}

trait Subscription<T> {
    /// The source's value now.
    fn current(&self) -> T;

    /// Whether the subscription can notify: a signal whose guard is
    /// zero-sized with no drop glue never notifies, per nami's guard
    /// contract ("when dropped, will unregister the watcher" — nothing
    /// to unregister, so nothing to register). [`Live::watch`] skips the
    /// watch of a subscription that cannot fire.
    fn fires(&self) -> bool;

    fn start(&self, watch: Watch<T>) -> Option<Box<dyn Any>>;
}

struct SignalSubscription<S>(S);

impl<S: Signal> Subscription<S::Output> for SignalSubscription<S> {
    fn current(&self) -> S::Output {
        self.0.snapshot()
    }

    fn fires(&self) -> bool {
        // The same rule `start` applies to the guard it returns.
        size_of::<S::Guard>() != 0 || needs_drop::<S::Guard>()
    }

    #[expect(
        clippy::inline_always,
        reason = "erase constant watches after devirtualizing the subscription"
    )]
    #[inline(always)]
    fn start(&self, watch: Watch<S::Output>) -> Option<Box<dyn Any>> {
        let guard = self.0.watch(move |context| watch.notify(context));
        // The watch always runs. Only guards with neither size nor drop glue
        // can be discarded instead of retained for unsubscription.
        if size_of::<S::Guard>() == 0 && !needs_drop::<S::Guard>() {
            None
        } else {
            Some(Box::new(guard) as Box<dyn Any>)
        }
    }
}

impl<T> Subscribe<T> {
    #[expect(
        clippy::inline_always,
        reason = "expose the concrete subscription to the recorder's call site"
    )]
    #[inline(always)]
    // Starts a recorder's slot subscription.
    #[must_use]
    fn start(self, watch: Watch<T>) -> Option<Box<dyn Any>> {
        self.0
            .as_ref()
            .and_then(|subscription| subscription.start(watch))
    }
}

/// A value accepted by [`Recorder`]: a [`Fixed`] value, or the current value
/// of a nami signal with the subscription that reports its later changes.
///
/// Outside a recording, a target binds a property to a `Live` through
/// [`Live::watch`]: layer properties take `impl Into<Live<T>>`, so a bound
/// signal keeps updating the property with no further transaction.
///
/// `Clone` so a host may bind one `Live` to more than one property — or
/// rebind it — where the recording hands it a single handle.
#[derive(Clone)]
pub struct Live<T> {
    value: T,
    subscription: Subscribe<T>,
}

impl<T: std::fmt::Debug> std::fmt::Debug for Live<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

impl<T> From<Fixed<T>> for Live<T> {
    #[inline]
    fn from(value: Fixed<T>) -> Self {
        Self {
            value: value.0,
            subscription: Subscribe(None),
        }
    }
}

impl<T: 'static, S: Signal<Output = T>> From<S> for Live<T> {
    #[expect(
        clippy::inline_always,
        reason = "expose constant signal subscriptions to call-site dead code elimination"
    )]
    #[inline(always)]
    fn from(signal: S) -> Self {
        let value = signal.snapshot();
        Self {
            value,
            subscription: Subscribe(Some(Rc::new(SignalSubscription(signal)))),
        }
    }
}

/// The guard keeping a [`Live::watch`] subscription alive.
///
/// A binding stores it for the binding's life; dropping it unsubscribes the
/// watch, so the signal's later changes no longer reach the property.
pub struct Binding {
    _guard: Box<dyn Any>,
}

impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Binding(..)")
    }
}

impl<T> Live<T> {
    /// The value when the `Live` was made: the constant, or the signal's
    /// value at that point. A binding ([`watch`](Self::watch)) of a signal
    /// starts from the signal's value when it binds instead — unless the
    /// signal cannot fire, which keeps this stored value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Whether binding this `Live` watches a signal's changes: `false`
    /// for a constant and for a signal whose subscription cannot fire,
    /// per [`Subscription::fires`].
    pub(crate) fn is_signal(&self) -> bool {
        self.subscription
            .0
            .as_ref()
            .is_some_and(|subscription| subscription.fires())
    }

    /// Binds `watcher` to the source signal's later changes — the one
    /// subscription API a target uses to bind a layer property to a
    /// signal. The watcher runs on the recording thread with the change's
    /// [`Context`](nami_core::watcher::Context): its value is the property's
    /// new target, and an [`Animation`] in its metadata is the animation
    /// the change was made under.
    ///
    /// Returns the value the binding starts from — the constant, or the
    /// signal's value read immediately before the watch starts, so a
    /// change made since the `Live` was made is not lost — and the guard
    /// keeping the subscription alive: `None` for a constant and for a
    /// signal whose subscription cannot fire. The non-firing case starts
    /// from the value the `Live` was made with, which differs from
    /// [`Subscription::current`] only when an impure map ran over a
    /// constant. Either way the binding replaces the property's previous
    /// one.
    #[expect(
        clippy::inline_always,
        reason = "expose the concrete subscription to the binding's call site"
    )]
    #[inline(always)]
    pub fn watch(
        self,
        watcher: impl Fn(nami_core::watcher::Context<T>) + 'static,
    ) -> (T, Option<Binding>) {
        let Self {
            value,
            subscription,
        } = self;
        let Some(subscription) = subscription.0 else {
            return (value, None);
        };
        if !subscription.fires() {
            return (value, None);
        }
        // A signal binding snapshots twice per edit: once when the `Live`
        // was made, once here before the watch starts. A registration-time
        // emit is not applied.
        let value = subscription.current();
        let guard = subscription
            .start(Watch::binding(watcher))
            .map(|guard| Binding { _guard: guard });
        (value, guard)
    }

    /// A `Live` of `f` applied to this one's value: `f` maps the stored
    /// value now, and the value the binding starts from and every later
    /// change when the result is bound. A change keeps its `Context`
    /// metadata, so an [`Animation`] it was made under still reaches the
    /// binding.
    pub fn map<U: 'static>(self, f: impl Fn(T) -> U + 'static) -> Live<U>
    where
        T: 'static,
    {
        let Self {
            value,
            subscription,
        } = self;
        let value = f(value);
        Live {
            value,
            subscription: Subscribe(subscription.0.map(|inner| {
                Rc::new(MappedSubscription {
                    inner,
                    f: Rc::new(f),
                }) as Rc<dyn Subscription<U>>
            })),
        }
    }
}

/// A subscription whose changes pass through a map before they reach the
/// watch: what [`Live::map`] leaves.
struct MappedSubscription<T, F> {
    inner: Rc<dyn Subscription<T>>,
    f: Rc<F>,
}

impl<T: 'static, U: 'static, F: Fn(T) -> U + 'static> Subscription<U> for MappedSubscription<T, F> {
    fn current(&self) -> U {
        (self.f)(self.inner.current())
    }

    fn fires(&self) -> bool {
        self.inner.fires()
    }

    fn start(&self, watch: Watch<U>) -> Option<Box<dyn Any>> {
        let f = Rc::clone(&self.f);
        self.inner.start(Watch::binding(
            move |context: nami_core::watcher::Context<T>| {
                watch.notify(context.map(&*f));
            },
        ))
    }
}

/// State shared between a [`Content`] and the watchers of its signals.
#[derive(Default)]
pub(crate) struct LiveState {
    pending: RefCell<Vec<SlotUpdate>>,
    /// Queued animated changes and running tracks. `None` until the
    /// first animated change, so a static `LiveState` costs one
    /// `Option` to build and drop, not a `HashMap`.
    anim: Cell<Option<Box<AnimState>>>,
    /// Set while queued animates or running tracks make
    /// [`LiveState::sample`] worth its borrows.
    // The surface drain probes it as a plain cell read — `sample` is a
    // real call on every static content otherwise.
    needs_sample: Cell<bool>,
    /// The installing surface's "a content may be sampling" flag,
    /// poked by [`LiveState::animate`] so a fully static surface can
    /// skip per-content probes. Detached when the content retires.
    surface_animated: RefCell<Option<SampleFlag>>,
    guards: RefCell<Vec<Box<dyn Any>>>,
    /// The surface the content is installed on. Detached when the content
    /// retires.
    owner: RefCell<Option<Weak<dyn LiveOwner>>>,
}

impl std::fmt::Debug for LiveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveState")
            .field("needs_sample", &self.needs_sample.get())
            .finish_non_exhaustive()
    }
}

/// The surface a content is installed on, as its live operands reach it.
pub trait LiveOwner {
    /// A live operand changed: a visible surface wakes the host for the
    /// frame that samples it, and a hidden one applies it at once.
    fn changed(&self);
}

/// A change carrying an `Animation`, queued until the next
/// [`LiveState::sample`] resolves the operand it animates from.
type Animate = (u32, Operand, Animation);

/// The animated-operand machinery of a [`LiveState`], present only
/// while a change is queued or a track runs.
#[derive(Default)]
struct AnimState {
    /// Animated changes not yet sampled into tracks.
    animates: Vec<Animate>,
    /// A running animation per operand slot.
    tracks: HashMap<Slot, OperandTrack>,
}

impl LiveState {
    fn push(&self, update: SlotUpdate) {
        let mut pending = self.pending.borrow_mut();
        let slot = update.slot();
        match pending.iter_mut().find(|queued| queued.slot() == slot) {
            Some(queued) => *queued = update,
            None => pending.push(update),
        }
        drop(pending);
        self.wake();
    }

    fn wake(&self) {
        let owner = self.owner.borrow().as_ref().and_then(Weak::upgrade);
        if let Some(owner) = owner {
            owner.changed();
        }
    }

    /// A change whose `Context` carried an `Animation`: queued for the
    /// next [`Content::sample`], which starts or retargets the slot's
    /// track from the operand's then-current value.
    ///
    /// # Panics
    /// Panics on a `Decay` animation, the same invariant `scroll_offset`'s
    /// siblings hold.
    fn animate(&self, command: u32, target: Operand, animation: Animation) {
        assert!(
            !matches!(animation, Animation::Decay(_)),
            "Decay is only legal on scroll_offset"
        );
        let mut anim = self.anim.take().unwrap_or_default();
        anim.animates.push((command, target, animation));
        self.anim.set(Some(anim));
        self.needs_sample.set(true);
        if let Some(flag) = self.surface_animated.borrow().as_ref() {
            flag.set(true);
        }
        self.wake();
    }

    /// A change without an `Animation`: the value snaps and any track or
    /// queued animate on the slot drops.
    fn snap(&self, update: SlotUpdate) {
        let slot = update.slot();
        if let Some(mut anim) = self.anim.take() {
            anim.tracks.remove(&slot);
            anim.animates.retain(|(command, target, _)| {
                *command != slot.command || target.kind() != slot.operand
            });
            let needed = !anim.tracks.is_empty() || !anim.animates.is_empty();
            if needed {
                self.anim.set(Some(anim));
            }
            self.needs_sample.set(needed);
        }
        self.push(update);
    }

    /// Starts or retargets tracks for the queued animated changes, then
    /// samples every running operand track at `time`, queuing the operand
    /// updates the next [`Content::take_change`] drains. An animate's
    /// start operand is the slot's latest queued update, or `list`'s
    /// recorded operand when none arrived. Returns `true` while tracks
    /// still run.
    fn sample(&self, time: Instant, list: &DisplayList) -> bool {
        if !self.needs_sample.get() {
            return false;
        }
        let Some(mut anim) = self.anim.take() else {
            self.needs_sample.set(false);
            return false;
        };
        let animates = std::mem::take(&mut anim.animates);
        if !animates.is_empty() {
            let mut pending = self.pending.borrow_mut();
            for (command, target, animation) in animates {
                let slot = Slot {
                    command,
                    operand: target.kind(),
                };
                // A running track retargets, keeping the last sampled
                // position and velocity when the new target keeps the lane
                // layout.
                if anim
                    .tracks
                    .get_mut(&slot)
                    .is_some_and(|track| track.retarget(target.clone(), animation))
                {
                    continue;
                }
                let from = pending
                    .iter()
                    .find(|queued| queued.slot() == slot)
                    .map_or_else(
                        || list.operand(command, slot.operand),
                        |queued| Some(queued.value.clone()),
                    );
                match from.and_then(|from| from.anim_lanes(&target)) {
                    Some(from) => {
                        anim.tracks
                            .insert(slot, OperandTrack::new(from, target, animation));
                    }
                    // Endpoints sharing no lane decomposition snap the
                    // change like an un-animated one.
                    None => match pending.iter_mut().find(|queued| queued.slot() == slot) {
                        Some(queued) => queued.value = target,
                        None => pending.push(SlotUpdate {
                            command,
                            value: target,
                        }),
                    },
                }
            }
            drop(pending);
        }
        if anim.tracks.is_empty() {
            self.needs_sample.set(false);
            return false;
        }
        let mut pending = self.pending.borrow_mut();
        let mut animating = false;
        anim.tracks.retain(|slot, track| {
            let (value, running) = track.sample(time);
            animating |= running;
            match pending.iter_mut().find(|queued| queued.slot() == *slot) {
                Some(queued) => queued.value = value,
                None => pending.push(SlotUpdate {
                    command: slot.command,
                    value,
                }),
            }
            running
        });
        let running = !anim.tracks.is_empty();
        if running {
            self.anim.set(Some(anim));
        }
        self.needs_sample.set(running);
        animating
    }

    /// Attaches the owning surface's sampling flag: `animate` pokes it
    /// once set, and a state that already needs sampling sets it
    /// immediately so nothing queues behind an install.
    fn attach_animated(&self, flag: &SampleFlag) {
        if self.needs_sample.get() {
            flag.set(true);
        }
        *self.surface_animated.borrow_mut() = Some(flag.clone());
    }

    /// Detaches the surface's sampling flag when the content retires
    /// into a spare, whose next install may live on another surface.
    fn detach_animated(&self) {
        *self.surface_animated.borrow_mut() = None;
    }
}

/// Records on the UI thread, accepting nami signals anywhere a value is
/// accepted.
pub struct Recorder {
    list: DisplayList,
    live: Rc<LiveState>,
    picture: Option<Picture>,
    size: LayoutSize,
    /// The clip, transform and group scopes open around the current call.
    depth: u32,
    /// The state of a recording opened with [`Content::record_layered`];
    /// `None` for a plain recording, which takes no material.
    layered: Option<Layered>,
}

/// A layered recording's state.
#[derive(Debug)]
struct Layered {
    /// The material scope the recording was opened under.
    scope: MaterialScope,
    /// Each material so far, in recording order, with the content recorded
    /// before it.
    parts: Vec<(Content, BackdropMaterial)>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("list", &self.list)
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// A plain recording into `list`, recording `live` operands.
    const fn new(
        list: DisplayList,
        live: Rc<LiveState>,
        picture: Option<Picture>,
        size: LayoutSize,
    ) -> Self {
        Self {
            list,
            live,
            picture,
            size,
            depth: 0,
            layered: None,
        }
    }

    /// Declares a backdrop material at this point of the recording: a
    /// chrome surface that samples what lies behind it.
    ///
    /// What was recorded before the call draws below the material and is
    /// part of its backdrop; what is recorded after it draws above it.
    /// `shape` is the member's clip and `effect` its per-member
    /// parameters for the registered `shader`; the member's group takes
    /// the parameters of the registered `capture` class. Both `shape` and
    /// `effect` are [`Live`]: their signals are not bound to any slot of
    /// the recording but reach the realized member, so a change updates
    /// it with no re-recording. See [`crate::material`].
    ///
    /// # Panics
    /// - in a recording not opened with [`Content::record_layered`];
    /// - inside a [`clip`](Draw::clip), [`transform`](Draw::transform) or
    ///   [`group`](Draw::group) scope: a material is allowed only at the
    ///   top level of a layered recording.
    pub fn backdrop_material<S: Shape>(
        &mut self,
        shape: impl Into<Live<S>>,
        shader: MaterialShader,
        capture: CaptureClass,
        effect: impl Into<Live<MaterialEffect>>,
    ) {
        let Self {
            layered: Some(layered),
            list,
            live,
            depth,
            ..
        } = self
        else {
            panic!(
                "a backdrop material can only be recorded into a recording opened with \
                 `Content::record_layered`"
            );
        };
        assert!(
            *depth == 0,
            "a backdrop material can only be recorded at the top level of a layered recording, \
             outside every clip, transform and group scope"
        );
        // Built before the part is taken: a panic in the caller's
        // conversions leaves the recording unchanged.
        let shape = shape.into().map(Shape::into_data);
        let material = BackdropMaterial::new(shape, shader, capture, effect.into(), layered.scope);
        layered.parts.push((Self::take_part(list, live), material));
    }

    /// Runs a clip, transform or group scope's `body` one scope deeper.
    fn scoped(&mut self, body: impl FnOnce(&mut Self)) {
        self.depth += 1;
        body(self);
        self.depth -= 1;
    }

    /// Finishes what has been recorded since the last material into a
    /// content of its own, leaving the recorder empty for the next part.
    fn take_part(list: &mut DisplayList, live: &mut Rc<LiveState>) -> Content {
        let mut list = std::mem::take(list);
        list.trim_spare();
        Content {
            picture: Picture::from_list(list),
            live: std::mem::take(live),
            sent: false,
        }
    }

    /// The size the host lays the recorded layer out at, as a signal:
    /// geometry derived from it (`c.layout_size().map(…)`) updates when the
    /// host resizes the layer, without re-recording. See [`LayoutSize`].
    #[must_use]
    pub fn layout_size(&self) -> LayoutSize {
        self.size.clone()
    }

    /// Finishes a recording without freezing its live operands. Subsequent
    /// signal changes remain incremental updates when the content is installed.
    #[must_use]
    #[inline]
    pub(crate) fn finish(mut self) -> Content {
        self.list.trim_spare();
        let picture = match self.picture.take() {
            Some(mut picture) => {
                picture.put_unique_list(self.list);
                picture
            }
            None => Picture::from_list(self.list),
        };
        Content {
            picture,
            live: self.live,
            sent: false,
        }
    }

    /// Subscribes to a value's later changes, which update operand `convert`
    /// produces on command `command`.
    #[expect(
        clippy::inline_always,
        reason = "expose constant signal subscriptions to call-site dead code elimination"
    )]
    #[inline(always)]
    fn subscribe<T: 'static>(
        &self,
        subscribe: Subscribe<T>,
        command: u32,
        convert: fn(T) -> Operand,
    ) {
        let guard = subscribe.start(Watch {
            destination: Destination::Slot {
                state: Rc::downgrade(&self.live),
                command,
                convert,
            },
        });
        if let Some(guard) = guard {
            self.live.guards.borrow_mut().push(guard);
        }
    }
}

fn paint_operand<P: Into<Paint>>(paint: P) -> Operand {
    Operand::Paint(paint.into())
}

impl Draw for Recorder {
    type Value<T: 'static> = Live<T>;

    fn fill<S: Shape, P: Into<Paint> + 'static>(
        &mut self,
        shape: impl Into<Live<S>>,
        paint: impl Into<Live<P>>,
    ) {
        let (shape, paint) = (shape.into(), paint.into());
        let shape_data = shape.value.into_data();
        let paint_data: Paint = paint.value.into();
        let command = self.list.push(Command::Fill {
            shape: shape_data,
            paint: paint_data,
        });
        self.subscribe(shape.subscription, command, |shape: S| {
            Operand::Shape(shape.into_data())
        });
        self.subscribe(paint.subscription, command, paint_operand::<P>);
    }

    fn stroke<S: Shape, P: Into<Paint> + 'static>(
        &mut self,
        shape: impl Into<Live<S>>,
        stroke: impl Into<Live<Stroke>>,
        paint: impl Into<Live<P>>,
    ) {
        let (shape, stroke, paint) = (shape.into(), stroke.into(), paint.into());
        let shape_data = shape.value.into_data();
        let paint_data: Paint = paint.value.into();
        let command = self.list.push(Command::Stroke {
            shape: shape_data,
            stroke: stroke.value,
            paint: paint_data,
        });
        self.subscribe(shape.subscription, command, |shape: S| {
            Operand::Shape(shape.into_data())
        });
        self.subscribe(stroke.subscription, command, Operand::Stroke);
        self.subscribe(paint.subscription, command, paint_operand::<P>);
    }

    fn shadow<S: Shape>(&mut self, shape: impl Into<Live<S>>, shadow: impl Into<Live<Shadow>>) {
        let (shape, shadow) = (shape.into(), shadow.into());
        let shape_data = shape.value.into_data();
        let command = self.list.push(Command::Shadow {
            shape: shape_data,
            shadow: shadow.value,
        });
        self.subscribe(shape.subscription, command, |shape: S| {
            Operand::Shape(shape.into_data())
        });
        self.subscribe(shadow.subscription, command, Operand::Shadow);
    }

    #[expect(
        clippy::inline_always,
        reason = "expose constant signal subscriptions to call-site dead code elimination"
    )]
    #[inline(always)]
    fn glyphs<P: Into<Paint> + 'static>(
        &mut self,
        run: impl Into<Live<GlyphRun>>,
        paint: impl Into<Live<P>>,
    ) {
        let run = run.into();
        let paint = paint.into();
        let paint_data: Paint = paint.value.into();
        let command = self.list.push(Command::Glyphs {
            run: run.value,
            paint: paint_data,
        });
        self.subscribe(run.subscription, command, Operand::Run);
        self.subscribe(paint.subscription, command, paint_operand::<P>);
    }

    fn image(&mut self, image: ImageId, dst: impl Into<Live<Rect>>, sampling: Sampling) {
        let dst = dst.into();
        let command = self.list.push(Command::Image {
            image,
            dst: dst.value,
            sampling,
        });
        self.subscribe(dst.subscription, command, Operand::Rect);
    }

    fn picture(&mut self, picture: &Picture, transform: impl Into<Live<Affine>>) {
        let transform = transform.into();
        let command = self.list.push(Command::Picture {
            picture: picture.clone(),
            transform: transform.value,
        });
        self.subscribe(transform.subscription, command, Operand::Transform);
    }

    fn text(&mut self, layout: TextLayoutId, transform: impl Into<Live<Affine>>) {
        let transform = transform.into();
        let command = self.list.push(Command::Text {
            layout,
            transform: transform.value,
        });
        self.subscribe(transform.subscription, command, Operand::Transform);
    }

    fn clip<S: Shape>(&mut self, shape: impl Into<Live<S>>, body: impl FnOnce(&mut Self)) {
        let shape = shape.into();
        let shape_data = shape.value.into_data();
        let begin = self.list.push(Command::BeginClip {
            shape: shape_data,
            end: 0,
        });
        self.subscribe(shape.subscription, begin, |shape: S| {
            Operand::Shape(shape.into_data())
        });
        self.scoped(body);
        self.list.end(begin);
    }

    fn transform(&mut self, transform: impl Into<Live<Affine>>, body: impl FnOnce(&mut Self)) {
        let transform = transform.into();
        let begin = self.list.push(Command::BeginTransform {
            transform: transform.value,
            end: 0,
        });
        self.subscribe(transform.subscription, begin, Operand::Transform);
        self.scoped(body);
        self.list.end(begin);
    }

    fn group(&mut self, group: impl Into<Live<Group>>, body: impl FnOnce(&mut Self)) {
        let group = group.into();
        let begin = self.list.push(Command::BeginGroup {
            group: group.value,
            end: 0,
        });
        self.subscribe(group.subscription, begin, Operand::Group);
        self.scoped(body);
        self.list.end(begin);
    }
}

/// The "something needs sampling" bit a target keeps per surface.
///
/// [`Content::attach`] connects an installed content to it: an animated
/// operand arriving sets it, so a surface with any live content knows a
/// frame must sample without probing every content's
/// [`needs_sample`](Content::needs_sample). The target clears it after a
/// sampling pass in which nothing still animates.
#[derive(Clone, Debug, Default)]
pub struct SampleFlag(Rc<Cell<bool>>);

impl SampleFlag {
    /// A clear flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an attached content needs sampling; a plain cell read.
    #[must_use]
    pub fn get(&self) -> bool {
        self.0.get()
    }

    /// Sets or clears the flag.
    pub fn set(&self, sampling: bool) {
        self.0.set(sampling);
    }
}

/// Whether a [`Content::sample`] left operand animations running: another
/// frame is required to keep them moving.
///
/// A bool newtype so a call site cannot confuse the answer with the
/// sampled instant or an operand count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Animating(bool);

impl Animating {
    /// The idle answer: nothing still animates.
    pub const IDLE: Self = Self(false);
    /// The running answer: another frame is required.
    pub const RUNNING: Self = Self(true);

    /// Whether another frame is required.
    #[must_use]
    pub const fn is_animating(&self) -> bool {
        self.0
    }
}

impl From<bool> for Animating {
    fn from(animating: bool) -> Self {
        Self(animating)
    }
}

/// Content recorded on the UI thread. It owns the subscriptions of the signals
/// it was recorded with, and turns their changes into [`ContentChange`]s.
///
/// Replaced picture storage can return after the render thread releases it.
///
/// `Content` is not `Send`: its signals live on the UI thread. What crosses to
/// the render thread is the owned [`ContentChange`].
pub struct Content {
    picture: Picture,
    live: Rc<LiveState>,
    sent: bool,
}

/// Spare storage a retired [`Content`] leaves for the next recording.
///
/// Its contents are opaque: a target stores it between recordings and hands
/// it back whole to [`Content::record_into`].
#[derive(Debug, Default)]
pub struct ContentSpare {
    /// The recycled picture storage.
    picture: Option<Picture>,
    /// The recycled live state.
    live: Option<Rc<LiveState>>,
}

impl ContentSpare {
    /// Hands the picture storage a render thread returned for reuse to the
    /// next [`Content::record_into`].
    pub fn put_picture(&mut self, picture: Picture) {
        self.picture = Some(picture);
    }

    /// Folds another spare's live state into this one, keeping this spare's
    /// picture storage: what a replaced content's [`Content::retire`] hands
    /// back.
    pub fn merge(&mut self, other: Self) {
        self.live = other.live;
    }
}

impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Content")
            .field("list", self.picture.display_list())
            .field("sent", &self.sent)
            .finish_non_exhaustive()
    }
}

impl Content {
    /// Records content for a layer laid out at `size` — the [`LayoutSize`]
    /// the target owns for the layer, which the recording reads through
    /// [`Recorder::layout_size`]. This is how any host produces content.
    #[must_use]
    pub fn record(size: &LayoutSize, body: impl FnOnce(&mut Recorder)) -> Self {
        Self::record_with_capacity(0, size, body)
    }

    /// Records content that may declare backdrop materials
    /// ([`Recorder::backdrop_material`]), split at each material.
    ///
    /// `size` is the layer's [`LayoutSize`], as for
    /// [`record`](Self::record). `scope` is the material scope the host
    /// records under: the nearest enclosing view subtree that groups its
    /// materials, or [`MaterialScope::SOLO`]. Every material recorded
    /// carries it.
    ///
    /// The recording comes back as [`LayeredContent`]: the content below
    /// the first material, then each material with the content recorded
    /// after it. A signal used by commands of one part becomes a slot of
    /// that part only. A recording with no material is all `below`.
    pub fn record_layered(
        size: &LayoutSize,
        scope: MaterialScope,
        body: impl FnOnce(&mut Recorder),
    ) -> LayeredContent {
        let mut recorder = Recorder::new(DisplayList::default(), Rc::default(), None, size.clone());
        recorder.layered = Some(Layered {
            scope,
            parts: Vec::new(),
        });
        body(&mut recorder);
        // Each material's part is the content recorded before it; the
        // content after the last one is the last run's.
        let mut above = Recorder::take_part(&mut recorder.list, &mut recorder.live);
        let Some(Layered { parts, .. }) = recorder.layered else {
            unreachable!("`record_layered` opened the recording with its layered state");
        };
        let mut runs = Vec::with_capacity(parts.len());
        for (before, material) in parts.into_iter().rev() {
            runs.push(MaterialRun { material, above });
            above = before;
        }
        runs.reverse();
        LayeredContent { below: above, runs }
    }

    /// Records into the `spare` a retired content handed back, reusing its
    /// picture storage and live state; a fresh recording when `spare` is
    /// empty.
    pub fn record_into(
        mut spare: ContentSpare,
        size: &LayoutSize,
        body: impl FnOnce(&mut Recorder),
    ) -> Self {
        let (picture, list) = spare.picture.take().map_or_else(
            || (None, DisplayList::default()),
            |mut picture| {
                picture.take_unique_list().map_or_else(
                    || (None, DisplayList::default()),
                    |mut list| {
                        list.clear();
                        (Some(picture), list)
                    },
                )
            },
        );
        let mut live = spare.live.take().unwrap_or_default();
        let live = if Rc::get_mut(&mut live).is_some() {
            live
        } else {
            Rc::new(LiveState::default())
        };
        let mut recorder = Recorder::new(list, live, picture, size.clone());
        body(&mut recorder);
        recorder.finish()
    }

    /// Retires the content, detaching its owner and sampling flag and
    /// releasing its subscriptions: after this call the owner receives no
    /// more change notifications. The spare it hands back keeps the
    /// recording's live state for the next [`record_into`](Self::record_into).
    #[must_use]
    pub fn retire(self) -> ContentSpare {
        let Self { picture, live, .. } = self;
        // Watchers only exist while the content carried signals; a
        // signal-free recording never attached the sampling flag, and
        // its sampling state needs no reset.
        if !live.guards.borrow().is_empty() {
            live.detach_animated();
            live.anim.set(None);
            live.needs_sample.set(false);
        }
        live.guards.borrow_mut().clear();
        live.pending.borrow_mut().clear();
        *live.owner.borrow_mut() = None;
        drop(picture);
        ContentSpare {
            picture: None,
            live: Some(live),
        }
    }

    /// Like [`record`](Self::record), reserving room for `capacity` commands —
    /// the previous recording's [`len`](Self::len) when re-recording the
    /// same content.
    pub(crate) fn record_with_capacity(
        capacity: usize,
        size: &LayoutSize,
        body: impl FnOnce(&mut Recorder),
    ) -> Self {
        let mut recorder = Recorder::new(
            DisplayList::with_capacity(capacity),
            Rc::default(),
            None,
            size.clone(),
        );
        body(&mut recorder);
        recorder.list.trim_spare();
        Self {
            picture: Picture::from_list(recorder.list),
            live: recorder.live,
            sent: false,
        }
    }

    /// Whether a [`sample`](Self::sample) call has work: an animated
    /// change arrived or a track still runs. A plain cell read — a target
    /// may probe every content per frame.
    #[must_use]
    pub fn needs_sample(&self) -> bool {
        self.live.needs_sample.get()
    }

    /// Connects the content's live operands to the surface `owner` and its
    /// sampling flag.
    ///
    /// A target calls it once, on install: a signal change then reaches
    /// the owner through [`LiveOwner::changed`], and an animated change
    /// sets `flag` so the target knows a frame must sample. A constant
    /// recording attaches nothing. [`retire`](Self::retire) detaches both.
    pub fn attach(&mut self, owner: Weak<dyn LiveOwner>, flag: &SampleFlag) {
        // Constant recordings need no owner or weak-count traffic.
        if !self.live.guards.borrow().is_empty() {
            *self.live.owner.borrow_mut() = Some(owner);
            self.live.attach_animated(flag);
        }
    }

    /// Freezes the latest received operand values into a shareable picture.
    /// This consumes the content and releases its subscriptions; later signal
    /// changes cannot alter the picture. Prefer installing live `Content`
    /// directly when changes should continue to reach the engine.
    #[must_use]
    pub fn into_picture(mut self) -> Picture {
        let updates = self.live.pending.take();
        if !updates.is_empty() {
            let _ = self.picture.list_mut().apply(updates);
        }
        self.picture
    }

    /// Commands in the recorded list.
    #[must_use]
    pub fn len(&self) -> usize {
        self.picture.display_list().len()
    }

    /// Whether the recorded list has no commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.picture.display_list().is_empty()
    }

    /// The recorded list as a render target lowers it, with every change
    /// [`take_change`](Self::take_change) has emitted applied. See
    /// [`DisplayListView`].
    #[must_use]
    pub fn view(&self) -> DisplayListView<'_> {
        self.picture.display_list().view()
    }

    /// Whether any command, including those of nested pictures, names
    /// `resource`: a glyph run's font, an image draw, or an image or
    /// shader paint — a target's resource-liveness bookkeeping.
    #[must_use]
    pub fn references(&self, resource: ResourceId) -> bool {
        self.picture.display_list().references(resource)
    }

    /// Samples every running operand animation at `now`, queueing the
    /// operand updates [`take_change`](Self::take_change) drains. The
    /// [`Animating`] answer tells the target whether another frame is
    /// required to keep them moving.
    ///
    /// A target samples before it reads the list for a commit, and never
    /// samples while it is reading the list.
    #[must_use]
    pub fn sample(&mut self, now: Instant) -> Animating {
        Animating(self.live.sample(now, self.picture.display_list()))
    }

    /// The change to send at the next commit, if any. The first call sends the
    /// whole display list; later calls send only the slots whose signals
    /// changed.
    pub fn take_change(&mut self) -> Option<ContentChange> {
        let updates = self.live.pending.take();
        if !updates.is_empty() {
            let _ = self.picture.list_mut().apply(updates.iter().cloned());
        }
        if !self.sent {
            self.sent = true;
            return Some(ContentChange::Replace(self.picture.clone()));
        }
        (!updates.is_empty()).then_some(ContentChange::Update(updates))
    }

    /// The display list with every signal change received so far applied, for
    /// capturing a frame.
    pub fn snapshot(&mut self) -> &DisplayList {
        let updates = self.live.pending.take();
        if !updates.is_empty() {
            let _ = self.picture.list_mut().apply(updates.iter().cloned());
            if self.sent {
                // Keep the pending updates for the render thread.
                *self.live.pending.borrow_mut() = updates;
            }
        }
        self.picture.display_list()
    }
}

/// What a commit sends to the render target for one content.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ContentChange {
    /// The whole display list, shared by reference when the content is first
    /// committed.
    Replace(Picture),
    /// New values for slots, sent when signals change afterwards.
    Update(Vec<SlotUpdate>),
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use kurbo::{Circle, Rect};
    use nami::{SignalExt, binding};

    use super::*;
    use crate::color::{Color, Srgb};
    use crate::shape::ShapeData;
    use crate::{Curve, WorkingColor};

    fn red() -> Color<Srgb> {
        Color::new([1., 0., 0., 1.])
    }

    #[test]
    fn explicit_recording_keeps_live_operands_until_frozen() {
        let radius = binding::<f64>(1.0);
        let mut recorder = Recorder::new(
            DisplayList::default(),
            Rc::default(),
            None,
            LayoutSize::new(),
        );
        recorder.fill(radius.map(|r| Circle::new((0.0, 0.0), r)), red());
        let mut content = recorder.finish();
        let Some(ContentChange::Replace(original)) = content.take_change() else {
            panic!("initial picture");
        };
        radius.set(2.0);
        assert!(matches!(
            content.take_change(),
            Some(ContentChange::Update(_))
        ));
        radius.set(3.0);
        let frozen = content.into_picture();
        radius.set(4.0);
        let Command::Fill { shape, .. } = &frozen.display_list().commands()[0] else {
            panic!("frozen fill");
        };
        assert_eq!(*shape, ShapeData::Circle(Circle::new((0.0, 0.0), 3.0)));
        let Command::Fill { shape, .. } = &original.display_list().commands()[0] else {
            panic!("original fill");
        };
        assert_eq!(*shape, ShapeData::Circle(Circle::new((0.0, 0.0), 1.0)));
    }

    #[test]
    fn owned_path_recording_preserves_storage_and_fill_rule() {
        let elements = vec![
            kurbo::PathEl::MoveTo((0.0, 0.0).into()),
            kurbo::PathEl::LineTo((1.0, 1.0).into()),
        ];
        let elements: std::sync::Arc<[kurbo::PathEl]> = elements.into();
        let pointer = elements.as_ptr();
        let shape = ShapeData::Path {
            elements,
            rule: crate::FillRule::EvenOdd,
        };
        let mut recorder = Recorder::new(
            DisplayList::default(),
            Rc::default(),
            None,
            LayoutSize::new(),
        );
        recorder.fill(Fixed(shape), red());
        let picture = recorder.finish().into_picture();
        let Command::Fill {
            shape: ShapeData::Path { elements, rule },
            ..
        } = &picture.display_list().commands()[0]
        else {
            panic!("path fill");
        };
        assert_eq!(elements.as_ptr(), pointer);
        assert_eq!(*rule, crate::FillRule::EvenOdd);
    }

    #[test]
    fn shared_retired_picture_falls_back_to_fresh_storage() {
        let mut content = Content::record(&LayoutSize::new(), |_| {});
        let pointer = std::ptr::from_ref(content.picture.display_list());
        let held = content.take_change();
        let reused = Content::record_into(content.retire(), &LayoutSize::new(), |_| {});
        assert_ne!(std::ptr::from_ref(reused.picture.display_list()), pointer);
        drop(held);
    }

    #[test]
    fn fixed_glyph_runs_move_their_storage_into_the_command() {
        let run = GlyphRun {
            font: crate::FontId::new(0),
            size: 12.0,
            coords: vec![123].into(),
            glyphs: vec![crate::Glyph {
                id: 7,
                x: 0.0,
                y: 0.0,
                transform: None,
            }]
            .into(),
            style: crate::GlyphStyle::Fill,
        };
        let glyphs = run.glyphs.as_ptr();
        let coords = run.coords.as_ptr();
        let content = Content::record(&LayoutSize::new(), |c| c.glyphs(Fixed(run), red()));
        let Command::Glyphs { run, .. } = &content.picture.display_list().commands()[0] else {
            panic!("the recorded glyph run");
        };
        assert_eq!(run.glyphs.as_ptr(), glyphs);
        assert_eq!(run.coords.as_ptr(), coords);
    }

    /// The recorder subscribes unconditionally — the `fires` skip is a
    /// property-binding optimization, so a slot's `watch` runs even when
    /// the signal's guard is zero-sized.
    #[test]
    fn a_recording_watches_a_signal_with_a_zero_sized_guard() {
        #[derive(Clone)]
        struct Observed(Rc<std::cell::Cell<usize>>);

        impl Signal for Observed {
            type Output = Rect;
            type Guard = ();

            fn snapshot(&self) -> Rect {
                Rect::new(0.0, 0.0, 8.0, 8.0)
            }

            fn watch(&self, _watcher: impl Fn(nami_core::watcher::Context<Rect>) + 'static) {
                self.0.set(self.0.get() + 1);
            }
        }

        let calls = Rc::new(std::cell::Cell::new(0));
        let content = Content::record(&LayoutSize::new(), |c| {
            c.fill(Observed(Rc::clone(&calls)), red());
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(content.len(), 1);
    }

    #[test]
    fn a_signal_change_regenerates_only_the_commands_that_reference_it() {
        let radius = binding::<f64>(8.);
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(Rect::new(0., 0., 10., 10.), red());
            c.fill(radius.map(|r| Circle::new((50., 50.), r)), red());
            c.fill(Rect::new(20., 20., 30., 30.), red());
        });

        let Some(ContentChange::Replace(mut remote)) = content.take_change() else {
            panic!("the first commit sends the whole list");
        };
        assert_eq!(remote.display_list().len(), 3);
        assert_eq!(content.take_change(), None, "nothing changed yet");

        radius.set(16.);
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a changed signal sends an update");
        };
        assert_eq!(updates.len(), 1);

        let dirty = remote.apply(updates);
        assert_eq!(dirty.ranges(), [Range { start: 1, end: 2 }]);
        let Command::Fill { shape, .. } = &remote.display_list().commands()[1] else {
            panic!("command 1 is the circle fill");
        };
        assert_eq!(*shape, ShapeData::Circle(Circle::new((50., 50.), 16.)));
    }

    #[test]
    fn a_text_layout_is_a_resource_whose_placement_is_a_slot() {
        let layout = TextLayoutId::new(7);
        let offset = binding::<f64>(0.);
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(Rect::new(0., 0., 10., 10.), red());
            c.text(layout, offset.map(|y| Affine::translate((4., y))));
        });
        assert!(content.references(ResourceId::TextLayout(layout)));
        assert!(!content.references(ResourceId::TextLayout(TextLayoutId::new(8))));

        let Some(ContentChange::Replace(mut remote)) = content.take_change() else {
            panic!("the first commit sends the whole list");
        };
        let slots: Vec<_> = remote
            .display_list()
            .view()
            .operands(1)
            .map(|(slot, _)| slot)
            .collect();
        assert_eq!(
            slots,
            [Slot {
                command: 1,
                operand: crate::display_list::OperandKind::Transform,
            }]
        );

        offset.set(12.);
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a changed placement sends an update");
        };
        let dirty = remote.apply(updates);
        assert_eq!(dirty.ranges(), [Range { start: 1, end: 2 }]);
        assert_eq!(
            remote.display_list().commands()[1],
            Command::Text {
                layout,
                transform: Affine::translate((4., 12.)),
            }
        );
        let fixed = Picture::record(|c| c.text(layout, Affine::IDENTITY));
        assert!(
            fixed
                .display_list()
                .references(ResourceId::TextLayout(layout))
        );
    }

    #[test]
    fn a_capacity_hint_records_the_same_list() {
        let first = Content::record_with_capacity(0, &LayoutSize::new(), |c| {
            for i in 0..4 {
                c.fill(Rect::new(f64::from(i), 0., 10., 10.), red());
            }
        });
        assert_eq!(first.len(), 4);
        let second = Content::record_with_capacity(first.len(), &LayoutSize::new(), |c| {
            for i in 0..4 {
                c.fill(Rect::new(f64::from(i), 0., 10., 10.), red());
            }
        });
        assert_eq!(second.len(), first.len());
    }

    #[test]
    fn a_scope_change_regenerates_the_whole_scope() {
        let offset = binding::<f64>(0.);
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(Rect::new(0., 0., 1., 1.), red());
            c.transform(offset.map(|x| Affine::translate((x, 0.))), |c| {
                c.fill(Rect::new(0., 0., 1., 1.), red());
                c.fill(Rect::new(1., 1., 2., 2.), red());
            });
            c.fill(Rect::new(5., 5., 6., 6.), red());
        });
        let Some(ContentChange::Replace(mut remote)) = content.take_change() else {
            panic!("the first commit sends the whole list");
        };

        offset.set(4.);
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a changed signal sends an update");
        };
        let dirty = remote.apply(updates);
        // BeginTransform (1), two fills (2, 3) and End (4).
        assert_eq!(dirty.ranges(), [Range { start: 1, end: 5 }]);
        assert!(!dirty.contains(0) && !dirty.contains(5));
    }

    #[test]
    fn repeated_changes_before_a_commit_send_one_update() {
        let radius = binding::<f64>(1.);
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(radius.map(|r| Circle::new((0., 0.), r)), red());
        });
        let _ = content.take_change();
        radius.set(2.);
        radius.set(3.);
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a changed signal sends an update");
        };
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0].value,
            Operand::Shape(ShapeData::Circle(Circle::new((0., 0.), 3.)))
        );
    }

    #[test]
    fn an_animated_operand_steps_into_take_change() {
        let colour = binding(WorkingColor::new([1., 0., 0., 1.]));
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(
                Rect::new(0., 0., 10., 10.),
                colour.clone().with(Animation::from(Curve::linear(
                    std::time::Duration::from_millis(400),
                ))),
            );
        });
        let Some(ContentChange::Replace(_)) = content.take_change() else {
            panic!("the first commit sends the whole list");
        };

        // The change itself queues nothing: the first sample emits the
        // start value, so the commit frame still shows `from`.
        colour.set(WorkingColor::new([0., 1., 0., 1.]));
        assert_eq!(content.take_change(), None, "the change defers to sampling");
        let start = Instant::now();
        assert!(content.sample(start).is_animating(), "the track is running");
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("the first sample sends an update");
        };
        assert_eq!(
            updates[0].value,
            Operand::Paint(Paint::Solid(WorkingColor::new([1., 0., 0., 1.])))
        );

        // Half-way the sampled paint is the endpoints' midpoint.
        assert!(
            content
                .sample(start + std::time::Duration::from_millis(200))
                .is_animating()
        );
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a mid-flight sample sends an update");
        };
        let Operand::Paint(Paint::Solid(mid)) = &updates[0].value else {
            panic!("the fill's paint operand");
        };
        assert!((mid.components[1] - 0.5).abs() < 0.01, "mid {mid:?}");

        // The settling sample reports the target exactly and stops running.
        assert!(
            !content
                .sample(start + std::time::Duration::from_millis(400))
                .is_animating()
        );
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("the settling sample sends an update");
        };
        assert_eq!(
            updates[0].value,
            Operand::Paint(Paint::Solid(WorkingColor::new([0., 1., 0., 1.])))
        );
        assert_eq!(
            content.take_change(),
            None,
            "a settled track queues nothing"
        );
    }

    #[test]
    fn a_retargeted_operand_animates_from_its_sampled_position() {
        let colour = binding(WorkingColor::new([1., 0., 0., 1.]));
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(
                Rect::new(0., 0., 10., 10.),
                colour.clone().with(Animation::from(Curve::linear(
                    std::time::Duration::from_millis(400),
                ))),
            );
        });
        let _ = content.take_change();
        let start = Instant::now();

        colour.set(WorkingColor::new([0., 1., 0., 1.]));
        assert!(
            content
                .sample(start + std::time::Duration::from_millis(200))
                .is_animating()
        );
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("the mid-flight sample sends an update");
        };
        let Operand::Paint(Paint::Solid(mid)) = updates[0].value.clone() else {
            panic!("the fill's paint operand");
        };

        // The retarget's first sample still holds the position it left at.
        colour.set(WorkingColor::new([0., 0., 1., 1.]));
        assert!(
            content
                .sample(start + std::time::Duration::from_millis(208))
                .is_animating()
        );
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("the retarget's first sample sends an update");
        };
        assert_eq!(
            updates[0].value,
            Operand::Paint(Paint::Solid(mid)),
            "the retarget starts where the previous track was"
        );

        // It settles on the new target, not the interrupted one.
        let mut t = start + std::time::Duration::from_millis(208);
        while content.sample(t).is_animating() {
            t += std::time::Duration::from_millis(16);
        }
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("the settling sample sends an update");
        };
        assert_eq!(
            updates[0].value,
            Operand::Paint(Paint::Solid(WorkingColor::new([0., 0., 1., 1.])))
        );
    }

    #[test]
    fn an_animated_change_without_shared_lanes_snaps() {
        let paint = binding(Paint::Solid(WorkingColor::new([1., 0., 0., 1.])));
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(
                Rect::new(0., 0., 10., 10.),
                paint.clone().with(Animation::from(Curve::linear(
                    std::time::Duration::from_millis(400),
                ))),
            );
        });
        let _ = content.take_change();

        // A Solid fill cannot interpolate into a gradient: the change lands
        // like an un-animated one instead of inventing lanes.
        let gradient = Paint::Linear(
            crate::paint::LinearGradient::new((0., 0.), (10., 0.))
                .stop(0., WorkingColor::new([1., 0., 0., 1.]))
                .stop(1., WorkingColor::new([0., 0., 1., 1.])),
        );
        paint.set(gradient.clone());
        // The snap resolves when the engine samples, like a frame does.
        let _ = content.sample(Instant::now());
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("the snap sends an update");
        };
        assert_eq!(updates[0].value, Operand::Paint(gradient));
        assert!(
            !content.sample(Instant::now()).is_animating(),
            "a snapped change starts no track"
        );
    }

    #[test]
    fn a_mapped_live_maps_every_value_and_keeps_the_animation() {
        let animation = Animation::from(Curve::linear(std::time::Duration::from_millis(300)));
        let x = binding::<f64>(1.0);
        let live: Live<f64> = x.with(animation).into();
        let mapped = live.map(|x| Circle::new((0., 0.), x * 2.));
        assert_eq!(*mapped.value(), Circle::new((0., 0.), 2.));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (start, guard) = mapped.watch({
            let seen = Rc::clone(&seen);
            move |context| {
                let animation = context.metadata().try_get::<Animation>();
                seen.borrow_mut().push((context.into_value(), animation));
            }
        });
        assert_eq!(start, Circle::new((0., 0.), 2.));
        x.set(3.0);
        assert_eq!(
            *seen.borrow(),
            [(Circle::new((0., 0.), 6.), Some(animation))],
            "the change is mapped and keeps its animation"
        );
        drop(guard);
        x.set(4.0);
        assert_eq!(seen.borrow().len(), 1, "dropping the guard unsubscribes");

        // A mapped constant maps its value and never subscribes.
        let (value, guard) = Live::from(Fixed(5.0_f64))
            .map(|x| x + 1.)
            .watch(|_| unreachable!("a constant never changes"));
        assert!((value - 6.).abs() < f64::EPSILON);
        assert!(guard.is_none());
    }

    #[test]
    fn dropping_content_releases_its_subscriptions() {
        let radius = binding::<f64>(1.);
        let content = Content::record(&LayoutSize::new(), |c| {
            c.fill(radius.map(|r| Circle::new((0., 0.), r)), red());
        });
        drop(content);
        // A watcher that outlived its content would upgrade a dead state;
        // setting must simply do nothing.
        radius.set(2.);
    }

    #[test]
    fn pictures_are_send_sync_and_shared() {
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<Picture>();
        assert_send_sync::<ContentChange>();

        let picture = Picture::record(|c| {
            c.fill(Rect::new(0., 0., 4., 4.), red());
            c.clip(Rect::new(0., 0., 2., 2.), |c| {
                c.fill(Circle::new((1., 1.), 1.), red());
            });
        });
        let shared = picture.clone();
        let from_thread = std::thread::spawn(move || shared.display_list().len())
            .join()
            .expect("the thread does not panic");
        assert_eq!(from_thread, 4);
        assert!(matches!(
            picture.display_list().commands()[1],
            Command::BeginClip { end: 3, .. }
        ));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn change_sets_round_trip_through_serde() {
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.stroke(
                Rect::new(0., 0., 1., 1.),
                Stroke::new(2.),
                crate::paint::LinearGradient::new((0., 0.), (1., 0.))
                    .stop(0., red())
                    .stop(1., Color::<Srgb>::new([0., 0., 1., 1.])),
            );
        });
        let change = content
            .take_change()
            .expect("the first commit sends the list");
        let json = serde_json::to_string(&change).expect("serializes");
        let back: ContentChange = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, change);
    }
}
