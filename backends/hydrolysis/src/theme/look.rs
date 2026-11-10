//! The look vocabulary of the theme contract (water-rs/waterui#1434): the
//! pieces every widget look is built from.
//!
//! A look is plain data a theme returns for one widget presentation. Its
//! values that follow the widget's live inputs — the interaction state, the
//! frame, the value, a drag, the press point — are [`Driven`]s: a pure
//! function from those [`Inputs`] to an operand value, plus the [`Motion`]
//! each kind of change runs with. Hydrolysis keeps every `Driven` as a live
//! operand of the recording and retargets it when an input changes, so the
//! engine animates the recorded command — continuous in position and
//! velocity — and a hover, press, focus, value or drag change records
//! nothing, walks no tree and runs no layout. No snapshot of interaction
//! state ever reaches a theme.
//!
//! The engine interpolates a derived operand lane by lane between targets;
//! it does not re-run the theme's function every frame. A derivation should
//! therefore be lane-linear in its inputs: translation, scale, opacity,
//! colour, radii.

use core::marker::PhantomData;
use core::time::Duration;
use std::any::Any;
use std::rc::Rc;

use cherenkov_record::{KeyframeAnimation, Keyframes, Live, Paint, Recorder, Repeat};
use kurbo::{Point, Rect, Size, Vec2};
use nami::Signal;
use nami::signal::IntoComputed;
use nami::watcher::Context;
use waterui::theme::Tint;
use waterui_controls::ControlSize;
use waterui_controls::button::ButtonStyle;
use waterui_core::Environment;
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::interaction::{InteractionState, StateValue};
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;
use waterui_text::font::Font;

use super::Chrome;
#[cfg(hydrolysis_runtime)]
use super::live::Operand;

/// The animations a change runs with as a condition sets in and as it
/// clears.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    /// The animation when the condition sets in: the pointer enters, the
    /// press lands, focus arrives.
    pub enter: Animation,
    /// The animation when the condition clears.
    pub exit: Animation,
}

impl Transition {
    /// `enter` as the condition sets in, `exit` as it clears.
    #[must_use]
    pub const fn new(enter: Animation, exit: Animation) -> Self {
        Self { enter, exit }
    }

    /// `animation` in both directions.
    #[must_use]
    pub fn symmetric(animation: Animation) -> Self {
        Self::new(animation.clone(), animation)
    }

    /// The animation for the condition setting in (`entering`) or clearing.
    #[must_use]
    pub const fn animation(&self, entering: bool) -> &Animation {
        if entering { &self.enter } else { &self.exit }
    }
}

/// How a look's operands follow a change of the widget's value.
///
/// A value change states how it asks to animate as a [`ChangeAnimation`]:
/// with an animation, with none named, or with animation disabled
/// ([`NoAnimation`](waterui_core::animation::NoAnimation)). Every variant
/// documents what it does in each case.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: the operand follows the caller's animation, its own whenever the change animates, or its own always"
)]
pub enum ValueMotion {
    /// The change's own animation; this one when the change names none. A
    /// change that disables animation snaps.
    Follow(Animation),
    /// This animation for a change that names an animation or none,
    /// whatever animation it names. A change that disables animation snaps.
    Own(Animation),
    /// This animation for every change, one made with animation disabled
    /// included.
    Always(Animation),
}

impl ValueMotion {
    /// The animation a value change made under `change` runs with; `None`
    /// snaps.
    #[must_use]
    pub fn animation(&self, change: ChangeAnimation) -> Option<Animation> {
        match self {
            Self::Follow(own) => change.or(own.clone()),
            Self::Own(own) => match change {
                ChangeAnimation::Animated(_) | ChangeAnimation::Unspecified => Some(own.clone()),
                ChangeAnimation::Disabled => None,
            },
            Self::Always(own) => Some(own.clone()),
        }
    }
}

/// The motion each kind of input change runs with.
///
/// A [`Driven`] retargets under the transition of the input that changed:
/// [`Self::press`] for a physical press, [`Self::drag`] for a drag,
/// [`Self::focus`] for keyboard focus, [`Self::hover`] for hover — the
/// first that applies, in that order — and [`Self::value`] for a value
/// change. A change of the disabled or selected state reaches the widget
/// through a render and snaps.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    /// Hover entering and leaving.
    pub hover: Transition,
    /// Keyboard focus arriving and leaving.
    pub focus: Transition,
    /// A physical press landing and releasing. A spring that overshoots
    /// passes its overshoot through to every operand it drives.
    pub press: Transition,
    /// A drag starting and ending. Drag movement itself retargets with no
    /// animation, so the operand tracks the finger.
    pub drag: Transition,
    /// A change of the widget's value.
    pub value: ValueMotion,
}

impl Motion {
    /// `animation` for every interaction transition, and value changes
    /// following their caller with `animation` when they name none.
    #[must_use]
    pub fn uniform(animation: Animation) -> Self {
        let transition = Transition::symmetric(animation.clone());
        Self {
            hover: transition.clone(),
            focus: transition.clone(),
            press: transition.clone(),
            drag: transition,
            value: ValueMotion::Follow(animation),
        }
    }

    /// The animation an interaction change from `old` to `new` runs with,
    /// `None` when it snaps.
    #[must_use]
    pub fn interaction(&self, old: InteractionState, new: InteractionState) -> Option<&Animation> {
        let changed = old ^ new;
        if changed.intersects(InteractionState::DISABLED | InteractionState::SELECTED) {
            return None;
        }
        [
            (InteractionState::PRESSED, &self.press),
            (InteractionState::DRAGGED, &self.drag),
            (InteractionState::FOCUSED, &self.focus),
            (InteractionState::HOVERED, &self.hover),
        ]
        .into_iter()
        .find(|(flag, _)| changed.contains(*flag))
        .map(|(flag, transition)| transition.animation(new.contains(flag)))
    }
}

/// The live inputs a [`Driven`] value is a function of.
#[derive(Debug)]
pub struct Inputs<'a, V> {
    /// The part's frame in the widget's own space.
    pub frame: Rect,
    /// The widget's interaction state.
    pub state: InteractionState,
    /// The widget's value: `bool` for a toggle, `f64` normalized to its
    /// range for a slider or progress, the selected index for a segmented
    /// control, `()` for a widget without one.
    pub value: &'a V,
    /// The offset of an active drag on the widget.
    pub drag: Option<Vec2>,
    /// The current press's point in the widget's own space; `None` between
    /// presses and for a keyboard or synthetic press.
    pub press_point: Option<Point>,
}

impl<V> Clone for Inputs<'_, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for Inputs<'_, V> {}

impl<'a, V> Inputs<'a, V> {
    /// Inputs at rest: `frame` in `state` showing `value`, with no drag
    /// and no press point.
    #[must_use]
    pub const fn at_rest(frame: Rect, state: InteractionState, value: &'a V) -> Self {
        Self {
            frame,
            state,
            value,
            drag: None,
            press_point: None,
        }
    }
}

type Target<T, V> = Rc<dyn Fn(&Inputs<'_, V>) -> T>;
type Notify = Rc<dyn Fn(Option<Animation>)>;
type Follows = Rc<dyn Fn(Notify) -> Box<dyn Any>>;

/// A value that follows the widget's live [`Inputs`]: a pure function from
/// them to an operand value, plus the [`Motion`] each kind of change runs
/// with.
///
/// The renderer keeps it as a live operand of the recording and retargets
/// it when an input changes; the theme's function is never sampled per
/// frame. A `Driven` built with [`Self::follow`] also retargets when its
/// signal changes — a theme token read from the environment follows a
/// colour-scheme or application override in place, under the animation
/// the change carries.
///
/// A `Driven` with no motion of its own runs with the motion of the look
/// part that holds it.
pub struct Driven<T, V = ()> {
    target: Target<T, V>,
    follows: Option<Follows>,
    motion: Option<Motion>,
    value_motion: Option<ValueMotionOf<V>>,
    repeating: Option<Repeating>,
}

/// The track a repeating value runs, built for the frame it records on.
type Repeating = Rc<dyn Fn(Rect) -> KeyframeAnimation>;

/// The value motion of a change to a given value.
type ValueMotionOf<V> = Rc<dyn Fn(&V) -> ValueMotion>;

impl<T, V> Clone for Driven<T, V> {
    fn clone(&self) -> Self {
        Self {
            target: Rc::clone(&self.target),
            follows: self.follows.clone(),
            motion: self.motion.clone(),
            value_motion: self.value_motion.clone(),
            repeating: self.repeating.clone(),
        }
    }
}

impl<T, V> core::fmt::Debug for Driven<T, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Driven")
            .field("follows", &self.follows.is_some())
            .field("motion", &self.motion)
            .field("value_motion", &self.value_motion.is_some())
            .field("repeating", &self.repeating.is_some())
            .finish_non_exhaustive()
    }
}

impl<T: 'static, V: 'static> Driven<T, V> {
    /// The value `target` derives from the widget's inputs.
    #[must_use]
    pub fn new(target: impl Fn(&Inputs<'_, V>) -> T + 'static) -> Self {
        Self {
            target: Rc::new(target),
            follows: None,
            motion: None,
            value_motion: None,
            repeating: None,
        }
    }

    /// A value that runs the `keyframes` of the frame its part records on,
    /// under `repeat`, once the part is recorded, whatever the other inputs.
    ///
    /// The engine samples the track on its own clock: a running indicator
    /// records nothing, walks no tree and does no Hydrolysis work per frame,
    /// until its part records again. The value at rest is the track's first
    /// one. The track's values must share one lane layout, and be of the
    /// type the operand records: a [`ShapeData`](cherenkov_record::ShapeData)
    /// for a shape, a [`Paint`], a [`Stroke`](cherenkov_record::Stroke).
    #[must_use]
    pub fn repeating(keyframes: impl Fn(Rect) -> Keyframes<T> + 'static, repeat: Repeat) -> Self
    where
        T: Clone,
    {
        let keyframes = Rc::new(keyframes);
        Self {
            target: Rc::new({
                let keyframes = Rc::clone(&keyframes);
                move |inputs| keyframes(inputs.frame).first().clone()
            }),
            follows: None,
            motion: None,
            value_motion: None,
            repeating: Some(Rc::new(move |frame| {
                KeyframeAnimation::new(keyframes(frame), repeat)
            })),
        }
    }

    /// One value, whatever the inputs.
    #[must_use]
    pub fn constant(value: T) -> Self
    where
        T: Clone,
    {
        Self::new(move |_| value.clone())
    }

    /// The value `target` derives from `signal`'s current value and the
    /// widget's inputs. A change of `signal` retargets the operand under
    /// the animation the change carries, or snaps it when it carries none.
    #[must_use]
    pub fn follow<O: Clone + 'static>(
        signal: impl IntoComputed<O>,
        target: impl Fn(&O, &Inputs<'_, V>) -> T + 'static,
    ) -> Self {
        let source = signal.into_computed();
        let watched = source.clone();
        Self {
            target: Rc::new(move |inputs| source.with_ref(|value| target(value, inputs))),
            follows: Some(Rc::new(move |notify: Notify| -> Box<dyn Any> {
                Box::new(watched.watch(move |context: Context<O>| {
                    notify(ChangeAnimation::of(context.metadata()).animation());
                }))
            })),
            motion: None,
            value_motion: None,
            repeating: None,
        }
    }

    /// Runs this value's changes with `motion` instead of the motion of the
    /// look part that holds it.
    #[must_use]
    pub const fn motion(mut self, motion: Motion) -> Self {
        self.motion = Some(motion);
        self
    }

    /// The motion of a change of the widget's value *to* the value `motion`
    /// receives — the new value, not the old one — so a value moves one way
    /// differently from the other. A switch's track fade is the reason: it
    /// turns on by a slower spring than it turns off by.
    ///
    /// It replaces the `value` of the [`Motion`] this value otherwise runs
    /// with ([`Self::motion`], else its part's) and leaves that motion's
    /// interaction transitions as they are. A value with none runs every
    /// value change with [`Motion::value`].
    #[must_use]
    pub fn value_motion(mut self, motion: impl Fn(&V) -> ValueMotion + 'static) -> Self {
        self.value_motion = Some(Rc::new(motion));
        self
    }

    /// The value at `inputs`.
    #[must_use]
    pub fn target(&self, inputs: &Inputs<'_, V>) -> T {
        (self.target)(inputs)
    }

    /// The motion this value runs with, when it has its own.
    #[must_use]
    pub const fn get_motion(&self) -> Option<&Motion> {
        self.motion.as_ref()
    }

    /// The motion of a change to `value`, when the value declares one
    /// ([`Self::value_motion`]).
    #[must_use]
    pub fn get_value_motion(&self, value: &V) -> Option<ValueMotion> {
        self.value_motion.as_ref().map(|motion| motion(value))
    }

    /// The track the engine runs this value along once its part records on
    /// `frame`, when it is [`repeating`](Self::repeating).
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn get_repeating(&self, frame: Rect) -> Option<KeyframeAnimation> {
        self.repeating.as_ref().map(|track| track(frame))
    }

    /// Calls `notify` with each change's animation whenever a signal this
    /// value follows changes; the returned guard keeps the subscription.
    /// `None` when the value follows no signal.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn subscribe(&self, notify: Rc<dyn Fn(Option<Animation>)>) -> Option<Box<dyn Any>> {
        self.follows.as_ref().map(|follows| follows(notify))
    }

    /// Whether this value follows a signal.
    #[cfg(hydrolysis_runtime)]
    pub(crate) const fn follows_signal(&self) -> bool {
        self.follows.is_some()
    }

    /// Whether `other` is this value: the same derivation, not an equal
    /// one.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.target, &other.target)
    }

    /// This value passed through `f`, following the same signals with the
    /// same motion.
    #[cfg(hydrolysis_runtime)]
    ///
    /// # Panics
    ///
    /// Panics on a [repeating](Self::repeating) value: its track holds
    /// values of the source type.
    pub(crate) fn map<U: 'static>(&self, f: impl Fn(T) -> U + 'static) -> Driven<U, V> {
        assert!(
            self.repeating.is_none(),
            "a repeating value cannot be mapped: its keyframes hold the source type"
        );
        let target = Rc::clone(&self.target);
        Driven {
            target: Rc::new(move |inputs| f(target(inputs))),
            follows: self.follows.clone(),
            motion: self.motion.clone(),
            value_motion: self.value_motion.clone(),
            repeating: None,
        }
    }
}

/// A [`StateValue`] is a [`Driven`] over the interaction state alone.
impl<T: Clone + 'static, V: 'static> From<StateValue<T>> for Driven<T, V> {
    fn from(value: StateValue<T>) -> Self {
        Self::new(move |inputs| value.resolve(inputs.state).clone())
    }
}

macro_rules! constant_driven {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl<V: 'static> From<$ty> for Driven<$ty, V> {
                fn from(value: $ty) -> Self {
                    Self::constant(value)
                }
            }
        )+
    };
}

constant_driven!(
    Paint,
    Color,
    Font,
    f32,
    f64,
    super::Border,
    super::Elevation,
    super::Corners
);

impl<V: 'static> From<cherenkov_record::WorkingColor> for Driven<Paint, V> {
    fn from(color: cherenkov_record::WorkingColor) -> Self {
        Self::constant(Paint::from(color))
    }
}

impl<V: 'static> From<StateValue<cherenkov_record::WorkingColor>> for Driven<Paint, V> {
    fn from(value: StateValue<cherenkov_record::WorkingColor>) -> Self {
        Self::new(move |inputs| Paint::from(*value.resolve(inputs.state)))
    }
}

/// Keeps [`DrivenSet`] implementable by this module only.
mod sealed {
    pub trait Sealed {}
}

/// The realization terms of a [`DrivenSet`]; nameable by Hydrolysis only, and
/// compiled only where a renderer exists to realize a look.
#[cfg(hydrolysis_runtime)]
mod realized {
    use std::any::Any;
    use std::rc::Rc;

    use waterui_core::animation::{Animation, ChangeAnimation};

    use super::{Inputs, Motion};

    /// What retargets one realized [`Driven`](super::Driven).
    #[derive(Clone, Copy)]
    pub enum Change<'a> {
        /// The interaction state moved from `old` to the inputs' state; a
        /// value without motion of its own runs with `motion`.
        Interaction {
            old: waterui_core::interaction::InteractionState,
            motion: &'a Motion,
        },
        /// The widget's value changed under `change`; a value without
        /// motion of its own runs with `motion`'s value motion.
        Value {
            change: &'a ChangeAnimation,
            motion: &'a Motion,
        },
        /// A signal the value follows, the frame or the press point
        /// changed, under this animation (`None` snaps).
        Animated(Option<&'a Animation>),
    }

    /// One realized [`Driven`](super::Driven): its live operand, retargeted
    /// by the renderer.
    pub trait Retarget<V> {
        fn retarget(&self, inputs: &Inputs<'_, V>, change: Change<'_>);
        fn subscribe(&self, notify: Rc<dyn Fn(Option<Animation>)>) -> Option<Box<dyn Any>>;

        /// Starts the track a repeating value runs once its part has
        /// recorded on `frame`; every other value has none.
        fn run(&self, _frame: kurbo::Rect) {}
    }

    /// A realized set: a factory for the tuple of [`Live`](cherenkov_record::Live)
    /// handles a recording closure receives, and the operands behind them.
    pub struct Realized<L, V> {
        pub(crate) live: Box<dyn Fn() -> L>,
        pub(crate) drivens: Vec<Rc<dyn Retarget<V>>>,
    }

    impl<L, V> core::fmt::Debug for Realized<L, V> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Realized")
                .field("drivens", &self.drivens.len())
                .finish_non_exhaustive()
        }
    }
}

#[cfg(hydrolysis_runtime)]
pub use realized::{Change, Realized, Retarget};

/// One [`Driven`] realized as a live operand.
#[cfg(hydrolysis_runtime)]
pub struct LiveDriven<T: 'static, V> {
    driven: Driven<T, V>,
    operand: Operand<T>,
}

#[cfg(hydrolysis_runtime)]
impl<T, V> core::fmt::Debug for LiveDriven<T, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveDriven")
            .field("driven", &self.driven)
            .finish_non_exhaustive()
    }
}

#[cfg(hydrolysis_runtime)]
impl<T: Clone + PartialEq + 'static, V: 'static> LiveDriven<T, V> {
    pub(crate) fn new(driven: Driven<T, V>, inputs: &Inputs<'_, V>) -> Self {
        let operand = Operand::new(driven.target(inputs));
        Self { driven, operand }
    }

    pub(crate) fn live(&self) -> Live<T> {
        self.operand.live()
    }

    /// The operand the value drives.
    pub(crate) const fn operand(&self) -> &Operand<T> {
        &self.operand
    }
}

#[cfg(hydrolysis_runtime)]
impl<T: Clone + PartialEq + 'static, V: 'static> Retarget<V> for LiveDriven<T, V> {
    fn retarget(&self, inputs: &Inputs<'_, V>, change: Change<'_>) {
        let animation = match change {
            Change::Interaction { old, motion } => self
                .driven
                .get_motion()
                .unwrap_or(motion)
                .interaction(old, inputs.state)
                .cloned(),
            Change::Value { change, motion } => {
                let motion = self.driven.get_motion().unwrap_or(motion);
                self.driven
                    .get_value_motion(inputs.value)
                    .as_ref()
                    .unwrap_or(&motion.value)
                    .animation(change.clone())
            }
            Change::Animated(animation) => animation.cloned(),
        };
        self.operand
            .set(self.driven.target(inputs), animation.as_ref());
    }

    fn subscribe(&self, notify: Rc<dyn Fn(Option<Animation>)>) -> Option<Box<dyn Any>> {
        self.driven.subscribe(notify)
    }

    fn run(&self, frame: Rect) {
        if let Some(track) = self.driven.get_repeating(frame) {
            self.operand.run(&track);
        }
    }
}

/// A tuple of [`Driven`] values a [`Custom`] part declares: its recording
/// closure receives the matching tuple of [`Live`] handles, so each handle's
/// type ties it to its declaration with no index lookup.
///
/// Implemented for `()` — a part that reads no driven value — and for tuples
/// of one to six `Driven`s over the same value type.
pub trait DrivenSet<V>: sealed::Sealed {
    /// The tuple of live handles the recording closure receives.
    type Live;

    /// Realizes every value as a live operand at `inputs`.
    #[cfg(hydrolysis_runtime)]
    #[doc(hidden)]
    fn realize(&self, inputs: &Inputs<'_, V>) -> Realized<Self::Live, V>;
}

macro_rules! driven_set {
    ($($ty:ident $index:tt),+) => {
        impl<V: 'static, $($ty: Clone + PartialEq + 'static),+> sealed::Sealed
            for ($(Driven<$ty, V>,)+)
        {
        }

        impl<V: 'static, $($ty: Clone + PartialEq + 'static),+> DrivenSet<V>
            for ($(Driven<$ty, V>,)+)
        {
            type Live = ($(Live<$ty>,)+);

            #[cfg(hydrolysis_runtime)]
            fn realize(&self, inputs: &Inputs<'_, V>) -> Realized<Self::Live, V> {
                let operands = ($(Rc::new(LiveDriven::new(self.$index.clone(), inputs)),)+);
                let drivens: Vec<Rc<dyn Retarget<V>>> =
                    vec![$(Rc::clone(&operands.$index) as Rc<dyn Retarget<V>>),+];
                Realized {
                    live: Box::new(move || ($(operands.$index.live(),)+)),
                    drivens,
                }
            }
        }
    };
}

impl sealed::Sealed for () {}

/// A part that reads no driven value: its closure receives `()`.
impl<V: 'static> DrivenSet<V> for () {
    type Live = ();

    #[cfg(hydrolysis_runtime)]
    fn realize(&self, _inputs: &Inputs<'_, V>) -> Realized<Self::Live, V> {
        Realized {
            live: Box::new(|| ()),
            drivens: Vec::new(),
        }
    }
}

driven_set!(A 0);
driven_set!(A 0, B 1);
driven_set!(A 0, B 1, C 2);
driven_set!(A 0, B 1, C 2, D 3);
driven_set!(A 0, B 1, C 2, D 3, E 4);
driven_set!(A 0, B 1, C 2, D 3, E 4, F 5);

type RecordCustom<L> = Rc<dyn Fn(&mut Recorder, Rect, L)>;

/// A look part the theme records itself: a tuple of [`Driven`] values and a
/// recording closure over [`Recorder`] that receives them as the matching
/// tuple of [`Live`] handles.
///
/// The closure runs when the part records — when the widget records for a
/// spec, frame or content change — and never on an interaction or value
/// change: those move the handles' operands. It records in the widget's
/// own space, `frame` being the part's frame, and may declare backdrop
/// materials ([`Recorder::backdrop_material`]).
pub struct Custom<V, D: DrivenSet<V>> {
    driven: D,
    record: RecordCustom<D::Live>,
    rotation: Option<KeyframeAnimation>,
    /// The rotation's first angle: the layer's rotation at rest.
    rest: f64,
    value: PhantomData<fn(&V)>,
}

impl<V, D: DrivenSet<V> + core::fmt::Debug> core::fmt::Debug for Custom<V, D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Custom")
            .field("driven", &self.driven)
            .field("rotating", &self.rotation.is_some())
            .finish_non_exhaustive()
    }
}

impl<V, D: DrivenSet<V> + Clone> Clone for Custom<V, D> {
    fn clone(&self) -> Self {
        Self {
            driven: self.driven.clone(),
            record: Rc::clone(&self.record),
            rotation: self.rotation.clone(),
            rest: self.rest,
            value: PhantomData,
        }
    }
}

impl<V, D: DrivenSet<V>> Custom<V, D> {
    /// The part `driven` records with `record`.
    #[must_use]
    pub fn new(driven: D, record: impl Fn(&mut Recorder, Rect, D::Live) + 'static) -> Self {
        Self {
            driven,
            record: Rc::new(record),
            rotation: None,
            rest: 0.0,
            value: PhantomData,
        }
    }

    /// Turns the part's whole layer about the centre of its frame, along
    /// `keyframes` — the unwrapped angle in radians, clockwise — under
    /// `repeat`.
    ///
    /// The engine runs the track on the layer's rotation property: the part
    /// records once, and a spinning part records nothing and does no
    /// Hydrolysis work per frame. Use it for the motion of the part as a
    /// whole; a [`Driven::repeating`] value moves one operand of the
    /// recording.
    #[must_use]
    pub fn rotating(mut self, keyframes: Keyframes<f64>, repeat: Repeat) -> Self {
        self.rest = *keyframes.first();
        self.rotation = Some(KeyframeAnimation::new(keyframes, repeat));
        self
    }
}

/// A [`Custom`] part realized: its recording, and the operands it reads.
#[cfg(hydrolysis_runtime)]
pub struct RealizedCustom<V> {
    record: RecordRealized,
    pub(crate) drivens: Vec<Rc<dyn Retarget<V>>>,
    rotation: Option<LayerRotation>,
}

#[cfg(hydrolysis_runtime)]
type RecordRealized = Box<dyn Fn(&mut Recorder, Rect)>;

/// The rotation a part's layer runs: its operand, the track and where the
/// layer turns about.
#[cfg(hydrolysis_runtime)]
#[derive(Clone)]
pub struct LayerRotation {
    operand: Operand<f64>,
    track: KeyframeAnimation,
}

#[cfg(hydrolysis_runtime)]
impl LayerRotation {
    /// The rotation property's live angle.
    pub(crate) fn live(&self) -> Live<f64> {
        self.operand.live()
    }

    /// Starts the track along the layer's property, which must be bound to
    /// [`Self::live`] already.
    pub(crate) fn run(&self) {
        self.operand.run(&self.track);
    }
}

#[cfg(hydrolysis_runtime)]
impl<V> RealizedCustom<V> {
    /// Records the part on `frame`, then starts the tracks of its repeating
    /// values: the recording has subscribed to every operand by then.
    pub(crate) fn record(&self, recorder: &mut Recorder, frame: Rect) {
        (self.record)(recorder, frame);
        for driven in &self.drivens {
            driven.run(frame);
        }
    }

    /// The part's layer rotation, when it turns.
    pub(crate) const fn rotation(&self) -> Option<&LayerRotation> {
        self.rotation.as_ref()
    }
}

/// The object-safe face of a [`Custom`] part, so a look stores parts of
/// different [`DrivenSet`]s side by side.
pub trait CustomPart<V> {
    #[cfg(hydrolysis_runtime)]
    fn realize(&self, inputs: &Inputs<'_, V>) -> RealizedCustom<V>;
}

impl<V: 'static, D: DrivenSet<V>> CustomPart<V> for Custom<V, D>
where
    D::Live: 'static,
{
    #[cfg(hydrolysis_runtime)]
    fn realize(&self, inputs: &Inputs<'_, V>) -> RealizedCustom<V> {
        let Realized { live, drivens } = self.driven.realize(inputs);
        let record = Rc::clone(&self.record);
        RealizedCustom {
            record: Box::new(move |recorder, frame| record(recorder, frame, live())),
            drivens,
            rotation: self.rotation.as_ref().map(|track| LayerRotation {
                operand: Operand::new(self.rest),
                track: track.clone(),
            }),
        }
    }
}

/// How a look styles the label content the renderer lays out: its font, its
/// colour and whether it follows the chrome's press scale.
///
/// The font and the colour are content styling, applied when the widget
/// renders; they default to the label's own. A label laid out from a view
/// (an icon, a composed label) takes them as its defaults, never over the
/// caller's own styling. The font is a [`Driven`] so it can vary with the
/// spec and the state; a label whose font or colour varies with hover,
/// press or focus re-renders on those changes.
#[derive(Debug, Clone, Default)]
pub struct Label {
    font: Option<Driven<Font>>,
    color: Option<Driven<Color>>,
    follows_press_scale: bool,
}

impl Label {
    /// A label in its own font and colour.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            font: None,
            color: None,
            follows_press_scale: false,
        }
    }

    /// Sets the label's font.
    #[must_use]
    pub fn font(mut self, font: impl Into<Driven<Font>>) -> Self {
        self.font = Some(font.into());
        self
    }

    /// Sets the label's colour.
    #[must_use]
    pub fn color(mut self, color: impl Into<Driven<Color>>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Grows the label with the chrome's press scale
    /// ([`PressFeedback::scale`](super::PressFeedback::scale)).
    #[must_use]
    pub const fn follows_press_scale(mut self) -> Self {
        self.follows_press_scale = true;
        self
    }

    /// The label's font; `None` keeps its own.
    #[must_use]
    pub const fn get_font(&self) -> Option<&Driven<Font>> {
        self.font.as_ref()
    }

    /// The label's colour; `None` keeps its own.
    #[must_use]
    pub const fn get_color(&self) -> Option<&Driven<Color>> {
        self.color.as_ref()
    }

    /// Whether the label grows with the chrome's press scale.
    #[must_use]
    pub const fn get_follows_press_scale(&self) -> bool {
        self.follows_press_scale
    }
}

/// Where an interactive part takes touches: a minimum target size centred
/// on its chrome frame, the tap slop and the cancel distance.
///
/// The target is the widget's bounds grown to cover the chrome frame grown
/// to `minimum` — a 28 pt button keeps a 44 pt target, and an icon button
/// laid out at its touch target keeps it. A touch press that moves no
/// farther than `slop` from where it landed stays a press while it waits
/// out its `press_delay`; a release within `cancel_distance` of the target
/// activates, and one beyond it cancels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitRegion {
    minimum: Size,
    slop: f64,
    cancel_distance: f64,
    press_delay: Duration,
}

impl Default for HitRegion {
    fn default() -> Self {
        Self::NONE
    }
}

impl HitRegion {
    /// The widget's bounds, with no slop, no cancel distance and no press
    /// delay.
    pub const NONE: Self = Self {
        minimum: Size::ZERO,
        slop: 0.0,
        cancel_distance: 0.0,
        press_delay: Duration::ZERO,
    };

    /// Grows the target to at least `minimum` around the chrome frame.
    #[must_use]
    pub const fn minimum(mut self, minimum: Size) -> Self {
        self.minimum = minimum;
        self
    }

    /// Keeps a waiting touch press alive while it moves no farther than
    /// `slop` from where it landed.
    #[must_use]
    pub const fn slop(mut self, slop: f64) -> Self {
        self.slop = slop;
        self
    }

    /// Activates on a release within `distance` of the target.
    #[must_use]
    pub const fn cancel_distance(mut self, distance: f64) -> Self {
        self.cancel_distance = distance;
        self
    }

    /// Holds a touch press back for `delay` before it shows, so a scroll
    /// that starts under the finger does not flash it. A mouse press shows
    /// at once.
    #[must_use]
    pub const fn press_delay(mut self, delay: Duration) -> Self {
        self.press_delay = delay;
        self
    }

    /// The minimum target size.
    #[must_use]
    pub const fn get_minimum(&self) -> Size {
        self.minimum
    }

    /// The tap slop.
    #[must_use]
    pub const fn get_slop(&self) -> f64 {
        self.slop
    }

    /// The cancel distance.
    #[must_use]
    pub const fn get_cancel_distance(&self) -> f64 {
        self.cancel_distance
    }

    /// The touch press delay.
    #[must_use]
    pub const fn get_press_delay(&self) -> Duration {
        self.press_delay
    }

    /// The touch target of a widget laid out at `bounds` whose chrome lies
    /// in `frame`.
    #[must_use]
    pub fn target(&self, bounds: Rect, frame: Rect) -> Rect {
        let size = Size::new(
            frame.width().max(self.minimum.width),
            frame.height().max(self.minimum.height),
        );
        bounds.union(Rect::from_center_size(frame.center(), size))
    }
}

/// What Hydrolysis asks a theme's [`button`](super::WidgetTheme::button)
/// look for: everything structural about one button.
///
/// A spec change requests the look again; everything else reaches the look
/// through its [`Driven`] values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonSpec {
    style: ButtonStyle,
    size: ControlSize,
    icon_only: bool,
    direction: LayoutDirection,
    tint: SpecTint,
}

/// The tint a widget's spec carries: the nearest [`Tint`] of its subtree,
/// or the theme's `Accent` token when the subtree sets none.
///
/// Every spec that exposes a tint holds one. Two are equal when they come
/// from the same installed `Tint`, so a spec is requested again when the
/// subtree is re-tinted and never when a tint signal changes: the colour
/// is a live [`Color`] the look draws, which updates in place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecTint(Option<Tint>);

impl SpecTint {
    /// The tint of the subtree `env` belongs to.
    #[must_use]
    pub fn resolve(env: &Environment) -> Self {
        Self(Tint::inherited(env).cloned())
    }

    /// The tint colour: resolved against the environment the look draws
    /// in, and live when the tint is a signal.
    #[must_use]
    pub fn color(&self) -> Color {
        Tint::color_or_accent(self.0.as_ref())
    }
}

impl ButtonSpec {
    /// The spec of a `style` button of `size`, laid out `direction`, whose
    /// label resolved to an icon-only presentation when `icon_only`, drawn
    /// with `tint`.
    #[must_use]
    pub const fn new(
        style: ButtonStyle,
        size: ControlSize,
        icon_only: bool,
        direction: LayoutDirection,
        tint: SpecTint,
    ) -> Self {
        Self {
            style,
            size,
            icon_only,
            direction,
            tint,
        }
    }

    /// The button's style.
    #[must_use]
    pub const fn style(&self) -> ButtonStyle {
        self.style
    }

    /// The button's control size.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// Whether the label resolved to `IconOnly`: the look lays the button
    /// out at its touch target, its chrome the icon-button container.
    #[must_use]
    pub const fn icon_only(&self) -> bool {
        self.icon_only
    }

    /// The layout direction the button is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The button's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live: a tint signal's change
    /// reaches the look's drawing without requesting the look again.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }
}

/// How a button looks and responds: its [`Chrome`], its [`Label`] styling,
/// the padding and minimum size it measures with, and its group opacity.
///
/// Hydrolysis measures, places, hit-tests, records and animates the button
/// from this one value, so its layout and its drawing cannot diverge. The
/// field order is the paint order: the chrome's surface, the label, the
/// chrome's interaction layers.
#[derive(Debug, Clone)]
pub struct ButtonLook {
    chrome: Chrome,
    label: Label,
    padding: Vec2,
    minimum: Size,
    group_opacity: Driven<f32>,
}

impl ButtonLook {
    /// A button of `chrome` styling its label with `label`, with no padding
    /// and no minimum size.
    #[must_use]
    pub fn new(chrome: Chrome, label: Label) -> Self {
        Self {
            chrome,
            label,
            padding: Vec2::ZERO,
            minimum: Size::ZERO,
            group_opacity: Driven::constant(1.0),
        }
    }

    /// Pads the label by `horizontal` on each side and `vertical` above and
    /// below.
    #[must_use]
    pub const fn padding(mut self, horizontal: f64, vertical: f64) -> Self {
        self.padding = Vec2::new(horizontal, vertical);
        self
    }

    /// Lays the button out at least `minimum`, capped by a tighter
    /// proposal.
    #[must_use]
    pub const fn minimum_size(mut self, minimum: Size) -> Self {
        self.minimum = minimum;
        self
    }

    /// Draws the whole button as one group at `opacity` — a disabled button
    /// that fades as a whole. An opacity that varies with hover, press or
    /// focus re-renders the button on those changes.
    #[must_use]
    pub fn group_opacity(mut self, opacity: impl Into<Driven<f32>>) -> Self {
        self.group_opacity = opacity.into();
        self
    }

    /// The button's chrome.
    #[must_use]
    pub const fn chrome(&self) -> &Chrome {
        &self.chrome
    }

    /// The label styling.
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// The label padding: horizontal per side, vertical above and below.
    #[must_use]
    pub const fn get_padding(&self) -> Vec2 {
        self.padding
    }

    /// The minimum size.
    #[must_use]
    pub const fn get_minimum_size(&self) -> Size {
        self.minimum
    }

    /// The group opacity.
    #[must_use]
    pub const fn get_group_opacity(&self) -> &Driven<f32> {
        &self.group_opacity
    }

    /// Replaces the chrome, keeping the label styling and the metrics.
    #[must_use]
    pub fn with_chrome(mut self, chrome: Chrome) -> Self {
        self.chrome = chrome;
        self
    }

    /// Replaces the label styling.
    #[must_use]
    pub fn with_label(mut self, label: Label) -> Self {
        self.label = label;
        self
    }
}
