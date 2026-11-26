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
#[cfg(waterui_feature = "slider")]
use core::ops::RangeInclusive;
use core::time::Duration;
use std::any::Any;
use std::rc::Rc;

use cherenkov_record::{
    BlendSpace, Curve, KeyframeAnimation, Keyframes, Live, Paint, Recorder, Repeat, Spring, Timing,
    curve_value,
};
use kurbo::{Point, Rect, Size, Vec2};
use nami::Signal;
use nami::signal::IntoComputed;
use nami::watcher::Context;
use waterui_controls::ControlSize;
use waterui_controls::button::ButtonStyle;
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::layout::LayoutDirection;
use waterui_core::{Environment, contract_violation};
use waterui_graphics::animation::engine_animation;
use waterui_graphics::color::Color;
use waterui_graphics::color::Tint;
use waterui_interaction::interaction::{InteractionState, StateValue};
use waterui_text::font::Font;

use super::Chrome;
use super::live::Operand;
#[cfg(waterui_feature = "slider")]
use super::{CornerShape, FocusIndicator, PressFeedback, StateLayer, Surface};

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
        self.interaction_direction(old, new)
            .map(|(animation, _)| animation)
    }

    /// The animation an interaction change from `old` to `new` runs with,
    /// paired with whether the winning flag is entering (`true`) or exiting
    /// (`false`); `None` when the change snaps.
    #[must_use]
    pub fn interaction_direction(
        &self,
        old: InteractionState,
        new: InteractionState,
    ) -> Option<(&Animation, bool)> {
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
        .map(|(flag, transition)| {
            let entering = new.contains(flag);
            (transition.animation(entering), entering)
        })
    }
}

/// Hermite smoothstep interpolation over `[edge0, edge1]`: `0.0` for
/// `x <= edge0`, `1.0` for `x >= edge1`, and `3s² − 2s³` in between.
#[must_use]
pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    if !(edge0.is_finite() && edge1.is_finite() && x.is_finite()) {
        return 0.0;
    }
    let span = edge1 - edge0;
    if span <= 0.0 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let s = ((x - edge0) / span).clamp(0.0, 1.0);
    s * s * 2.0_f64.mul_add(-s, 3.0)
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
    /// Exponentially smoothed pointer drag velocity in logical points per
    /// second; `None` at rest.
    pub velocity: Option<Vec2>,
}

impl<V> Clone for Inputs<'_, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for Inputs<'_, V> {}

impl<'a, V> Inputs<'a, V> {
    /// Inputs at rest: `frame` in `state` showing `value`, with no drag,
    /// no press point and no velocity.
    #[must_use]
    pub const fn at_rest(frame: Rect, state: InteractionState, value: &'a V) -> Self {
        Self {
            frame,
            state,
            value,
            drag: None,
            press_point: None,
            velocity: None,
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
    smoothstep_window: Option<(f64, f64)>,
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
            smoothstep_window: self.smoothstep_window,
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
            .field("smoothstep_window", &self.smoothstep_window)
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
            smoothstep_window: None,
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
            smoothstep_window: None,
        }
    }

    /// A [repeating](Self::repeating) value whose keyframes are built from
    /// `signal`'s current value, and run again from their start whenever the
    /// signal changes: a track made of theme tokens follows a colour-scheme
    /// or application override on the running indicator.
    #[must_use]
    pub fn repeating_following<O: Clone + 'static>(
        signal: impl IntoComputed<O>,
        keyframes: impl Fn(&O, Rect) -> Keyframes<T> + 'static,
        repeat: Repeat,
    ) -> Self
    where
        T: Clone,
    {
        let source = signal.into_computed();
        let watched = source.clone();
        let keyframes = Rc::new(keyframes);
        Self {
            target: Rc::new({
                let (source, keyframes) = (source.clone(), Rc::clone(&keyframes));
                move |inputs| {
                    source.with_ref(|value| keyframes(value, inputs.frame).first().clone())
                }
            }),
            follows: Some(Rc::new(move |notify: Notify| -> Box<dyn Any> {
                Box::new(watched.watch(move |context: Context<O>| {
                    notify(ChangeAnimation::of(context.metadata()).animation());
                }))
            })),
            motion: None,
            value_motion: None,
            repeating: Some(Rc::new(move |frame| {
                source.with_ref(|value| KeyframeAnimation::new(keyframes(value, frame), repeat))
            })),
            smoothstep_window: None,
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
            smoothstep_window: None,
        }
    }

    /// Runs this value's changes with `motion` instead of the motion of the
    /// look part that holds it.
    #[must_use]
    pub const fn motion(mut self, motion: Motion) -> Self {
        self.motion = Some(motion);
        self
    }

    /// Applies a smoothstep transfer window `(edge0, edge1)` to animated
    /// transitions of this driven value.
    ///
    /// Along a transition's normalized progress `p` in `[0, 1]`, the operand
    /// holds its starting value for `p <= edge0`, moves smoothly from start
    /// to target along `3s² − 2s³` (`s = (p − edge0) / (edge1 − edge0)`)
    /// across `[edge0, edge1]`, and holds its target for `p >= edge1`.
    /// On an interaction exit (`PRESSED`, `DRAGGED`, `FOCUSED`, or `HOVERED`
    /// clearing), the window mirrors to `(1 − edge1, 1 − edge0)` so the
    /// transfer curve plays in reverse across the same progress band.
    ///
    /// # Contract
    /// `edge0` and `edge1` must be finite with `0.0 <= edge0 < edge1 <= 1.0`.
    /// Out-of-range edges are clamped to `[0.0, 1.0]`, and a degenerate
    /// window (`edge0 >= edge1`) is reported and ignored.
    #[must_use]
    pub fn smoothstep_window(mut self, edge0: f64, edge1: f64) -> Self {
        if !(edge0.is_finite() && edge1.is_finite()) {
            contract_violation!(
                "smoothstep_window edges ({edge0}, {edge1}) must be finite; the window is ignored"
            );
            return self;
        }
        let lo = edge0.clamp(0.0, 1.0);
        let hi = edge1.clamp(0.0, 1.0);
        if lo >= hi {
            contract_violation!(
                "smoothstep_window requires edge0 < edge1 in [0, 1], got ({edge0}, {edge1}); the window is ignored"
            );
            return self;
        }
        self.smoothstep_window = Some((lo, hi));
        self
    }

    /// The smoothstep transfer window `(edge0, edge1)`, when one is set.
    #[must_use]
    pub const fn get_smoothstep_window(&self) -> Option<(f64, f64)> {
        self.smoothstep_window
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
    #[doc(hidden)]
    #[must_use]
    pub fn get_repeating(&self, frame: Rect) -> Option<KeyframeAnimation> {
        self.repeating.as_ref().map(|track| track(frame))
    }

    /// Calls `notify` with each change's animation whenever a signal this
    /// value follows changes; the returned guard keeps the subscription.
    /// `None` when the value follows no signal.
    #[doc(hidden)]
    pub fn subscribe(&self, notify: Rc<dyn Fn(Option<Animation>)>) -> Option<Box<dyn Any>> {
        self.follows.as_ref().map(|follows| follows(notify))
    }

    /// Whether this value follows a signal.
    #[doc(hidden)]
    #[must_use]
    pub const fn follows_signal(&self) -> bool {
        self.follows.is_some()
    }

    /// Whether `other` is this value: the same derivation, not an equal
    /// one.
    #[doc(hidden)]
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.target, &other.target)
    }

    /// This value passed through `f`, following the same signals with the
    /// same motion.
    ///
    /// A [repeating](Self::repeating) value's track holds values of the
    /// source type, so mapping one is the theme's contract violation: it is
    /// reported and the mapped value runs without the track.
    #[doc(hidden)]
    #[must_use]
    pub fn map<U: 'static>(&self, f: impl Fn(T) -> U + 'static) -> Driven<U, V> {
        if self.repeating.is_some() {
            contract_violation!(
                "a repeating value cannot be mapped: its keyframes hold the source type; the mapped value runs without the repeating track"
            );
        }
        let target = Rc::clone(&self.target);
        Driven {
            target: Rc::new(move |inputs| f(target(inputs))),
            follows: self.follows.clone(),
            motion: self.motion.clone(),
            value_motion: self.value_motion.clone(),
            repeating: None,
            smoothstep_window: self.smoothstep_window,
        }
    }

    /// Resolves `(animation, reverse)` for `change` at `inputs`, where
    /// `reverse` is `true` when an interaction flag is exiting.
    #[doc(hidden)]
    #[must_use]
    pub fn resolve_change(
        &self,
        inputs: &Inputs<'_, V>,
        change: Change<'_>,
    ) -> (Option<Animation>, bool) {
        match change {
            Change::Interaction { old, motion } => self
                .get_motion()
                .unwrap_or(motion)
                .interaction_direction(old, inputs.state)
                .map_or((None, false), |(animation, entering)| {
                    (Some(animation.clone()), !entering)
                }),
            Change::Value { change, motion } => {
                let motion = self.get_motion().unwrap_or(motion);
                let animation = self
                    .get_value_motion(inputs.value)
                    .as_ref()
                    .unwrap_or(&motion.value)
                    .animation(change.clone());
                (animation, false)
            }
            Change::Animated(animation) => (animation.cloned(), false),
        }
    }

    /// Applies `target` to `operand` under `(animation, reverse)`, honoring
    /// [`Self::smoothstep_window`] through an engine-clock keyframe track.
    #[doc(hidden)]
    pub fn apply_value_to(
        &self,
        operand: &Operand<T>,
        target: T,
        animation: Option<&Animation>,
        reverse: bool,
    ) where
        T: Clone + PartialEq,
    {
        if let (Some(animation), Some(window)) = (animation, self.smoothstep_window) {
            let current = operand.snapshot();
            if current == target {
                return;
            }
            if let Some(track) =
                windowed_keyframes(current, target.clone(), animation, window, reverse)
            {
                operand.set_keyframed(target, track);
                return;
            }
        }
        operand.set(target, animation);
    }

    /// Retargets `operand` to `inputs` under `change`, honoring
    /// [`Self::smoothstep_window`].
    #[doc(hidden)]
    pub fn apply_to(&self, operand: &Operand<T>, inputs: &Inputs<'_, V>, change: Change<'_>)
    where
        T: Clone + PartialEq,
    {
        let (animation, reverse) = self.resolve_change(inputs, change);
        self.apply_value_to(operand, self.target(inputs), animation.as_ref(), reverse);
    }

    /// Returns an `Animation` shaped by [`Self::smoothstep_window`] for
    /// properties that take a single [`Animation`] rather than a keyframe track.
    #[doc(hidden)]
    #[must_use]
    pub fn shape_animation(
        &self,
        animation: Option<&Animation>,
        reverse: bool,
    ) -> Option<Animation> {
        let animation = animation?;
        let Some(window) = self.smoothstep_window else {
            return Some(animation.clone());
        };
        windowed_animation(animation, window, reverse).or_else(|| Some(animation.clone()))
    }
}

/// Finds normalized time `t in [0, 1]` where `curve` reaches `progress`.
fn invert_curve(curve: &Curve, progress: f64) -> f64 {
    if progress <= 0.0 {
        return 0.0;
    }
    if progress >= 1.0 {
        return 1.0;
    }
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    for _ in 0..32 {
        let mid = f64::midpoint(lo, hi);
        if curve_value(curve, mid) < progress {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    f64::midpoint(lo, hi)
}

/// Computes `(total_duration, t0, t1)` in normalized `[0, 1]` time for where a
/// damped spring crosses `win_lo` and `win_hi` on its step response `0 -> 1`.
fn spring_window_times(spring: &Spring, win_lo: f64, win_hi: f64) -> (Duration, f64, f64) {
    let response = spring.response.max(0.02);
    let damping = spring.damping.clamp(0.15, 2.0);
    let max_t = (response * 8.0 / damping).clamp(0.05, 4.0);
    let step = (response / 64.0).min(0.005);

    let progress = |t: f64| -> f64 { 1.0 + spring.sample(-1.0, 0.0, t).0 };
    let bisect = |mut lo: f64, mut hi: f64, target: f64| -> f64 {
        for _ in 0..24 {
            let mid = f64::midpoint(lo, hi);
            if progress(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        f64::midpoint(lo, hi)
    };

    let mut dt0 = if win_lo <= 0.0 { Some(0.0) } else { None };
    let mut dt1 = if win_hi <= 0.0 { Some(0.0) } else { None };
    let mut settled = None;
    let mut prev_t = 0.0;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "max_t / step is positive and bounded below 1000"
    )]
    let steps = (max_t / step).ceil() as u32;
    for index in 1..=steps {
        let t = (f64::from(index) * step).min(max_t);
        let (offset, vel) = spring.sample(-1.0, 0.0, t);
        let p = 1.0 + offset;
        if dt0.is_none() && p >= win_lo {
            dt0 = Some(bisect(prev_t, t, win_lo));
        }
        if dt1.is_none() && p >= win_hi {
            dt1 = Some(bisect(prev_t, t, win_hi));
        }
        if dt1.is_some() && offset.abs() <= 1e-3 && vel.abs() <= 1e-3 {
            settled = Some(t);
            break;
        }
        prev_t = t;
    }

    let dt0 = dt0.unwrap_or(0.0);
    let dt1 = dt1.unwrap_or(max_t).max(dt0);
    let total_s = settled.unwrap_or(max_t).max(dt1).max(1e-4);
    let t0 = (dt0 / total_s).clamp(0.0, 1.0);
    let t1 = (dt1 / total_s).clamp(t0, 1.0);
    (Duration::from_secs_f64(total_s), t0, t1)
}

/// Computes `(duration, t0, t1)` for `animation` over `window`, mirroring the
/// window when `reverse` is `true`.
fn resolve_window_timing(
    animation: &Animation,
    window: (f64, f64),
    reverse: bool,
) -> Option<(Duration, f64, f64)> {
    let (edge0, edge1) = window;
    let (win_lo, win_hi) = if reverse {
        (1.0 - edge1, 1.0 - edge0)
    } else {
        (edge0, edge1)
    };
    match engine_animation(animation).timing() {
        Some(Timing::Curve { curve, duration }) => {
            if duration <= 0.0 {
                return None;
            }
            let engine_curve = Curve::bezier(
                Duration::from_secs_f64(duration),
                curve.x1,
                curve.y1,
                curve.x2,
                curve.y2,
            );
            let t0 = invert_curve(&engine_curve, win_lo);
            let t1 = invert_curve(&engine_curve, win_hi).max(t0);
            Some((engine_curve.duration, t0, t1))
        }
        Some(Timing::Spring(spring)) => Some(spring_window_times(&spring, win_lo, win_hi)),
        None => None,
    }
}

/// Builds a one-shot keyframe track moving `from -> target` through an exact
/// Hermite smoothstep (`CubicBezier(1/3, 0, 2/3, 1)`) over `window`.
#[doc(hidden)]
#[must_use]
pub fn windowed_keyframes_raw<T: Clone + 'static>(
    from: T,
    target: T,
    animation: &Animation,
    window: (f64, f64),
    reverse: bool,
) -> Option<Keyframes<T>> {
    let (duration, t0, t1) = resolve_window_timing(animation, window, reverse)?;
    // CubicBezier(1/3, 0, 2/3, 1) has x(u) = u and y(u) = 3u² - 2u³ (exact smoothstep).
    let smooth = Curve::bezier(Duration::from_secs(1), 1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0);
    let linear = Curve::linear(Duration::from_secs(1));
    let mut track = Keyframes::new(duration, from.clone());
    if t0 > 0.0 {
        track = track.then(t0, linear, from);
    }
    track = track.then(t1, smooth, target.clone());
    if t1 < 1.0 {
        track = track.then(1.0, linear, target);
    }
    Some(track)
}

/// Builds a one-shot [`KeyframeAnimation`] for [`windowed_keyframes_raw`].
#[doc(hidden)]
#[must_use]
pub fn windowed_keyframes<T: Clone + 'static>(
    from: T,
    target: T,
    animation: &Animation,
    window: (f64, f64),
    reverse: bool,
) -> Option<KeyframeAnimation> {
    windowed_keyframes_raw(from, target, animation, window, reverse)
        .map(|track| KeyframeAnimation::new(track, Repeat::ONCE))
}

/// Approximates a smoothstep window as a single cubic Bézier [`Animation`]
/// for layer properties that accept an [`Animation`] rather than keyframes.
#[expect(
    clippy::cast_possible_truncation,
    reason = "normalized Bézier control points in [0, 1] quantize cleanly to f32"
)]
fn windowed_animation(
    animation: &Animation,
    window: (f64, f64),
    reverse: bool,
) -> Option<Animation> {
    let (duration, t0, t1) = resolve_window_timing(animation, window, reverse)?;
    let span = t1 - t0;
    let x1 = (span / 3.0 + t0).clamp(0.0, 1.0) as f32;
    let x2 = (span.mul_add(2.0 / 3.0, t0)).clamp(x1.into(), 1.0) as f32;
    Some(Animation::bezier(duration, x1, 0.0, x2, 1.0))
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

/// The realization terms of a [`DrivenSet`]; nameable by Hydrolysis only.
mod realized {
    use std::any::Any;
    use std::rc::Rc;

    use waterui_core::animation::{Animation, ChangeAnimation};

    use super::{Inputs, Motion};

    /// What retargets one realized [`Driven`](super::Driven).
    #[derive(Clone, Copy, Debug)]
    #[expect(
        clippy::exhaustive_enums,
        reason = "closed set of renderer retarget causes"
    )]
    pub enum Change<'a> {
        /// The interaction state moved from `old` to the inputs' state; a
        /// value without motion of its own runs with `motion`.
        Interaction {
            /// Previous interaction state.
            old: waterui_interaction::interaction::InteractionState,
            /// Fallback motion when the driven value has none of its own.
            motion: &'a Motion,
        },
        /// The widget's value changed under `change`; a value without
        /// motion of its own runs with `motion`'s value motion.
        Value {
            /// The value change animation context.
            change: &'a ChangeAnimation,
            /// Fallback motion when the driven value has none of its own.
            motion: &'a Motion,
        },
        /// A signal the value follows, the frame or the press point
        /// changed, under this animation (`None` snaps).
        Animated(Option<&'a Animation>),
    }

    /// One realized [`Driven`](super::Driven): its live operand, retargeted
    /// by the renderer.
    pub trait Retarget<V> {
        /// Retargets the live operand to `inputs` under `change`.
        fn retarget(&self, inputs: &Inputs<'_, V>, change: Change<'_>);
        /// Subscribes `notify` to any signal this driven value follows.
        fn subscribe(&self, notify: Rc<dyn Fn(Option<Animation>)>) -> Option<Box<dyn Any>>;

        /// Starts the track a repeating value runs once its part has
        /// recorded on `frame`; every other value has none.
        fn run(&self, _frame: kurbo::Rect) {}
    }

    /// A realized set: a factory for the tuple of [`Live`](cherenkov_record::Live)
    /// handles a recording closure receives, and the operands behind them.
    pub struct Realized<L, V> {
        #[doc(hidden)]
        pub live: Box<dyn Fn() -> L>,
        #[doc(hidden)]
        pub drivens: Vec<Rc<dyn Retarget<V>>>,
    }

    impl<L, V> core::fmt::Debug for Realized<L, V> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Realized")
                .field("drivens", &self.drivens.len())
                .finish_non_exhaustive()
        }
    }
}

#[doc(hidden)]
pub use realized::{Change, Realized, Retarget};

/// One [`Driven`] realized as a live operand.
#[doc(hidden)]
pub struct LiveDriven<T: 'static, V> {
    driven: Driven<T, V>,
    operand: Operand<T>,
}

impl<T, V> core::fmt::Debug for LiveDriven<T, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveDriven")
            .field("driven", &self.driven)
            .finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static, V: 'static> LiveDriven<T, V> {
    #[doc(hidden)]
    #[must_use]
    pub fn new(driven: Driven<T, V>, inputs: &Inputs<'_, V>) -> Self {
        let operand = Operand::new(driven.target(inputs));
        Self { driven, operand }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn live(&self) -> Live<T> {
        self.operand.live()
    }

    /// The operand the value drives.
    #[doc(hidden)]
    #[must_use]
    pub const fn operand(&self) -> &Operand<T> {
        &self.operand
    }

    /// Retargets the operand using a single [`Animation`] (shaped by any
    /// smoothstep window) rather than a keyframe track, for layer properties
    /// that do not accept keyframe tracks (such as backdrop clips and effects).
    #[doc(hidden)]
    pub fn retarget_shaped(&self, inputs: &Inputs<'_, V>, change: Change<'_>) {
        let (animation, reverse) = self.driven.resolve_change(inputs, change);
        let shaped = self.driven.shape_animation(animation.as_ref(), reverse);
        self.operand
            .set(self.driven.target(inputs), shaped.as_ref());
    }
}

impl<T: Clone + PartialEq + 'static, V: 'static> Retarget<V> for LiveDriven<T, V> {
    fn retarget(&self, inputs: &Inputs<'_, V>, change: Change<'_>) {
        self.driven.apply_to(&self.operand, inputs, change);
        // A plain change replaces a running track, so a signal the track is
        // built from changing — or the frame — starts the new one.
        if matches!(change, Change::Animated(_)) {
            self.run(inputs.frame);
        }
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
/// of one to eight `Driven`s over the same value type.
pub trait DrivenSet<V>: sealed::Sealed {
    /// The tuple of live handles the recording closure receives.
    type Live;

    /// Realizes every value as a live operand at `inputs`.
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
driven_set!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
driven_set!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);

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
    /// The rotation stages, innermost first: each track with its first
    /// angle, the layer's rotation at rest.
    rotations: Vec<(KeyframeAnimation, f64)>,
    /// The part's opacity; `None` draws it whole, with no layer of its own.
    opacity: Option<Driven<f32, V>>,
    value: PhantomData<fn(&V)>,
}

impl<V, D: DrivenSet<V> + core::fmt::Debug> core::fmt::Debug for Custom<V, D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Custom")
            .field("driven", &self.driven)
            .field("rotations", &self.rotations.len())
            .field("opacity", &self.opacity)
            .finish_non_exhaustive()
    }
}

impl<V, D: DrivenSet<V> + Clone> Clone for Custom<V, D> {
    fn clone(&self) -> Self {
        Self {
            driven: self.driven.clone(),
            record: Rc::clone(&self.record),
            rotations: self.rotations.clone(),
            opacity: self.opacity.clone(),
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
            rotations: Vec::new(),
            opacity: None,
            value: PhantomData,
        }
    }

    /// Draws the whole part at `opacity`, a function of the part's inputs
    /// animated by the part's motion like its other values. A part given
    /// none is drawn whole.
    ///
    /// Hydrolysis draws a part with an opacity in a layer of its own whose
    /// opacity follows the value, so a change never records the part again.
    /// At opacity 0 the part costs nothing: the engine skips the layer, a
    /// backdrop material it declares included — a lens that exists only
    /// while the control is pressed is recorded once and shows only then.
    #[must_use]
    pub fn opacity(mut self, opacity: impl Into<Driven<f32, V>>) -> Self {
        self.opacity = Some(opacity.into());
        self
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
    ///
    /// Calling it again adds a stage outside the ones before it: the part
    /// turns about the same centre by the sum of its stages, each repeating
    /// on its own period, as nested rotating layers do.
    #[must_use]
    pub fn rotating(mut self, keyframes: Keyframes<f64>, repeat: Repeat) -> Self {
        let rest = *keyframes.first();
        self.rotations
            .push((KeyframeAnimation::new(keyframes, repeat), rest));
        self
    }
}

/// A [`Custom`] part realized: its recording, and the operands it reads.
#[doc(hidden)]
pub struct RealizedCustom<V> {
    record: RecordRealized,
    #[doc(hidden)]
    pub drivens: Vec<Rc<dyn Retarget<V>>>,
    rotations: Vec<LayerRotation>,
    /// The operand the part's layer opacity follows, when it has one.
    opacity: Option<Operand<f32>>,
}

impl<V> core::fmt::Debug for RealizedCustom<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RealizedCustom")
            .field("drivens", &self.drivens.len())
            .field("rotations", &self.rotations.len())
            .field("opacity", &self.opacity)
            .finish_non_exhaustive()
    }
}

type RecordRealized = Box<dyn Fn(&mut Recorder, Rect)>;

/// The rotation a part's layer runs: its operand, the track and where the
/// layer turns about.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct LayerRotation {
    operand: Operand<f64>,
    track: KeyframeAnimation,
}

impl LayerRotation {
    /// The rotation property's live angle.
    #[doc(hidden)]
    #[must_use]
    pub fn live(&self) -> Live<f64> {
        self.operand.live()
    }

    /// Starts the track along the layer's property, which must be bound to
    /// [`Self::live`] already.
    #[doc(hidden)]
    pub fn run(&self) {
        self.operand.run(&self.track);
    }
}

impl<V> RealizedCustom<V> {
    /// Records the part on `frame`, then starts the tracks of its repeating
    /// values: the recording has subscribed to every operand by then.
    #[doc(hidden)]
    pub fn record(&self, recorder: &mut Recorder, frame: Rect) {
        (self.record)(recorder, frame);
        for driven in &self.drivens {
            driven.run(frame);
        }
    }

    /// The part's rotation stages, innermost first; none when it does not
    /// turn.
    #[doc(hidden)]
    #[must_use]
    pub fn rotations(&self) -> &[LayerRotation] {
        &self.rotations
    }

    /// The operand the part's layer opacity follows; `None` when the part
    /// is drawn whole, with no layer of its own.
    #[doc(hidden)]
    #[must_use]
    pub const fn opacity(&self) -> Option<&Operand<f32>> {
        self.opacity.as_ref()
    }
}

/// The object-safe face of a [`Custom`] part, so a look stores parts of
/// different [`DrivenSet`]s side by side.
#[doc(hidden)]
pub trait CustomPart<V> {
    /// Realizes this part's recording and live operands at `inputs`.
    fn realize(&self, inputs: &Inputs<'_, V>) -> RealizedCustom<V>;
}

impl<V: 'static, D: DrivenSet<V>> CustomPart<V> for Custom<V, D>
where
    D::Live: 'static,
{
    fn realize(&self, inputs: &Inputs<'_, V>) -> RealizedCustom<V> {
        let Realized { live, mut drivens } = self.driven.realize(inputs);
        let record = Rc::clone(&self.record);
        // The opacity retargets with the part's other values.
        let opacity = self.opacity.as_ref().map(|opacity| {
            let opacity = Rc::new(LiveDriven::new(opacity.clone(), inputs));
            let operand = opacity.operand().clone();
            drivens.push(opacity);
            operand
        });
        RealizedCustom {
            record: Box::new(move |recorder, frame| record(recorder, frame, live())),
            opacity,
            drivens,
            rotations: self
                .rotations
                .iter()
                .map(|(track, rest)| LayerRotation {
                    operand: Operand::new(*rest),
                    track: track.clone(),
                })
                .collect(),
        }
    }
}

/// Records `part` over `frame` with every value at rest: the way a look's
/// parts that have no live state of their own draw into a widget's own
/// recording.
#[doc(hidden)]
pub fn record_at_rest(part: &dyn CustomPart<()>, recorder: &mut Recorder, frame: Rect) {
    part.realize(&Inputs::at_rest(frame, InteractionState::empty(), &()))
        .record(recorder, frame);
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
    slop: f32,
    cancel_distance: f32,
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
    pub const fn slop(mut self, slop: f32) -> Self {
        self.slop = slop;
        self
    }

    /// Activates on a release within `distance` of the target.
    #[must_use]
    pub const fn cancel_distance(mut self, distance: f32) -> Self {
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
    pub const fn get_slop(&self) -> f32 {
        self.slop
    }

    /// The cancel distance.
    #[must_use]
    pub const fn get_cancel_distance(&self) -> f32 {
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
    group_blend_space: Option<BlendSpace>,
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
            group_blend_space: None,
        }
    }

    /// Pads the label by `horizontal` on each side and `vertical` above and
    /// below.
    #[must_use]
    pub const fn padding(mut self, horizontal: f32, vertical: f32) -> Self {
        self.padding = Vec2::new(horizontal as f64, vertical as f64);
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

    /// The space the group of [`Self::group_opacity`] composites in:
    /// its parts composite with each other in it, and the group fades onto
    /// what lies beneath in it. With none set, the group composites in the
    /// space of the level beneath it.
    #[must_use]
    pub const fn group_blend_space(mut self, space: BlendSpace) -> Self {
        self.group_blend_space = Some(space);
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

    /// The space the group composites in, when one is set.
    #[must_use]
    pub const fn get_group_blend_space(&self) -> Option<BlendSpace> {
        self.group_blend_space
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

/// What Hydrolysis asks a theme's [`slider`](super::WidgetTheme::slider)
/// look for: everything structural about one slider.
///
/// A spec change requests the look again; everything else reaches the look
/// through its [`Driven`] values.
#[derive(Debug, Clone, PartialEq)]
#[cfg(waterui_feature = "slider")]
pub struct SliderSpec {
    size: ControlSize,
    range: RangeInclusive<f64>,
    step: Option<f64>,
    direction: LayoutDirection,
    tint: SpecTint,
    label: bool,
    value_labels: bool,
    value_indicator: bool,
}

#[cfg(waterui_feature = "slider")]
impl SliderSpec {
    /// The spec of a `size` slider over `range` moving in `step`s, laid out
    /// `direction` and drawn with `tint`.
    ///
    /// `label` says whether the slider has a visible label, `value_labels`
    /// whether it has a label beside either end of its track, and
    /// `value_indicator` whether it shows its value above the knob while it
    /// is held.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the spec is the slider's structural inputs, each independent of the others"
    )]
    pub const fn new(
        size: ControlSize,
        range: RangeInclusive<f64>,
        step: Option<f64>,
        direction: LayoutDirection,
        tint: SpecTint,
        label: bool,
        value_labels: bool,
        value_indicator: bool,
    ) -> Self {
        Self {
            size,
            range,
            step,
            direction,
            tint,
            label,
            value_labels,
            value_indicator,
        }
    }

    /// The slider's control size.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// The range of values the slider takes.
    #[must_use]
    pub const fn range(&self) -> &RangeInclusive<f64> {
        &self.range
    }

    /// The increment the value moves in, anchored at the start of the range;
    /// `None` moves it continuously.
    #[must_use]
    pub const fn step(&self) -> Option<f64> {
        self.step
    }

    /// The layout direction the slider is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The slider's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live: a tint signal's change
    /// reaches the look's drawing without requesting the look again.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }

    /// Whether the slider has a visible label.
    #[must_use]
    pub const fn has_label(&self) -> bool {
        self.label
    }

    /// Whether the slider has a label beside either end of its track.
    #[must_use]
    pub const fn has_value_labels(&self) -> bool {
        self.value_labels
    }

    /// Whether the slider shows its value above the knob while it is held.
    #[must_use]
    pub const fn has_value_indicator(&self) -> bool {
        self.value_indicator
    }
}

/// The horizontal and vertical measures a slider is laid out by. Hydrolysis
/// measures and places the slider from these alone, so its size and its
/// drawing cannot diverge.
///
/// The slider is a label band above a control row. The row holds the track
/// between its two ends, each end optionally taking a label beside it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(waterui_feature = "slider")]
pub struct SliderLayout {
    /// The control row's height: the room the knob travels in.
    pub row_height: f32,
    /// The distance from the slider's edge to an end of the track with no
    /// label beside it: the room the knob needs beyond the track's end.
    pub track_inset: f32,
    /// The distance from the slider's edge to an end label.
    pub label_inset: f32,
    /// The gap between an end label and the track.
    pub label_gap: f32,
    /// The shortest the track is laid out.
    pub minimum_track: f32,
    /// The space between a label above the row and the row. `None` when the
    /// look draws no label: nothing is reserved for it.
    pub label_spacing: Option<f32>,
}

/// The knob of a slider: the extent its parts draw it in, and where it takes
/// touches.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(waterui_feature = "slider")]
pub struct Knob {
    /// The knob's resting extent, centred on the value's position on the
    /// track. A knob that changes size draws within its own parts.
    pub size: Size,
    /// The touch target about the knob, its minimum size centred on
    /// [`Self::size`]. A touch press holds back for no delay: a slider takes
    /// its press at once.
    pub hit: HitRegion,
}

#[cfg(waterui_feature = "slider")]
impl Knob {
    /// The knob's touch target when centred on `center`.
    #[must_use]
    pub fn target(&self, center: Point) -> Rect {
        let minimum = self.hit.get_minimum();
        Rect::from_center_size(
            center,
            Size::new(
                self.size.width.max(minimum.width),
                self.size.height.max(minimum.height),
            ),
        )
    }
}

/// How a drag on a slider maps to its value. The renderer executes it,
/// rounding the value to the spec's step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: a drag sets the value from the pointer's position or from its movement"
)]
#[cfg(waterui_feature = "slider")]
pub enum DragMapping {
    /// The value is the pointer's position along the track, so a press
    /// anywhere on the slider moves it there.
    Absolute,
    /// The value moves with the pointer's movement from where it was when
    /// the press landed, so a press moves nothing by itself. With
    /// `knob_only`, a press outside the knob's touch target does nothing at
    /// all.
    Relative {
        /// Whether only a press on the knob drags.
        knob_only: bool,
    },
}

/// The value a slider shows above its knob while it is held: the bubble's
/// container, and the style of the formatted value laid out in it.
///
/// The bubble sits centred above the knob, `gap` clear of the knob's top, and
/// follows it along the track; it is as wide as the widest formatted value
/// needs, no narrower than `minimum`.
#[derive(Clone)]
#[cfg(waterui_feature = "slider")]
pub struct ValueIndicator {
    font: Font,
    color: Color,
    padding: Vec2,
    minimum: Size,
    gap: f32,
    container: Rc<dyn CustomPart<f64>>,
}

#[cfg(waterui_feature = "slider")]
impl core::fmt::Debug for ValueIndicator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ValueIndicator")
            .field("font", &self.font)
            .field("color", &self.color)
            .field("padding", &self.padding)
            .field("minimum", &self.minimum)
            .field("gap", &self.gap)
            .finish_non_exhaustive()
    }
}

#[cfg(waterui_feature = "slider")]
impl ValueIndicator {
    /// An indicator whose text takes `font` and `color`, in a bubble that
    /// `container` records on the bubble's frame.
    #[must_use]
    pub fn new<D: DrivenSet<f64> + 'static>(
        font: Font,
        color: Color,
        container: Custom<f64, D>,
    ) -> Self
    where
        D::Live: 'static,
    {
        Self {
            font,
            color,
            padding: Vec2::ZERO,
            minimum: Size::ZERO,
            gap: 0.0,
            container: Rc::new(container),
        }
    }

    /// Pads the text by `horizontal` on each side and `vertical` above and
    /// below.
    #[must_use]
    pub const fn padding(mut self, horizontal: f32, vertical: f32) -> Self {
        self.padding = Vec2::new(horizontal as f64, vertical as f64);
        self
    }

    /// Makes the bubble at least `minimum`.
    #[must_use]
    pub const fn minimum_size(mut self, minimum: Size) -> Self {
        self.minimum = minimum;
        self
    }

    /// Clears the knob's top by `gap`.
    #[must_use]
    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// The text's font.
    #[must_use]
    pub const fn get_font(&self) -> &Font {
        &self.font
    }

    /// The text's colour.
    #[must_use]
    pub const fn get_color(&self) -> &Color {
        &self.color
    }

    /// The padding around the text: horizontal per side, vertical above and
    /// below.
    #[must_use]
    pub const fn get_padding(&self) -> Vec2 {
        self.padding
    }

    /// The bubble's minimum size.
    #[must_use]
    pub const fn get_minimum_size(&self) -> Size {
        self.minimum
    }

    /// The gap between the knob's top and the bubble's bottom.
    #[must_use]
    pub const fn get_gap(&self) -> f32 {
        self.gap
    }

    /// The bubble's container part.
    #[doc(hidden)]
    #[must_use]
    pub fn container(&self) -> &Rc<dyn CustomPart<f64>> {
        &self.container
    }
}

/// How a slider looks and responds: its parts over the normalized value,
/// the knob, the drag, the value indicator and the measures it is laid out
/// by.
///
/// Hydrolysis measures, places, hit-tests, records and animates the slider
/// from this one value. Its track, fill, knob and shadows are the [`Custom`]
/// parts the look is built from, in paint order, each a function of the
/// slider's [`Inputs`]: the frame is the row between the two ends of the
/// track, and the value is the position normalized to the range, `0` at the
/// start and `1` at the end. The look draws left to right; Hydrolysis
/// mirrors its parts in a right-to-left layout. A value change, a press, a
/// drag and a change of hover or focus move the parts' operands under the
/// look's [`Motion`] and record nothing.
#[derive(Clone)]
#[cfg(waterui_feature = "slider")]
pub struct SliderLook {
    chrome: Chrome,
    layout: SliderLayout,
    knob: Knob,
    drag: DragMapping,
    parts: Vec<Rc<dyn CustomPart<f64>>>,
    indicator: Option<ValueIndicator>,
    group_opacity: Driven<f32>,
    group_blend_space: Option<BlendSpace>,
}

#[cfg(waterui_feature = "slider")]
impl core::fmt::Debug for SliderLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SliderLook")
            .field("layout", &self.layout)
            .field("knob", &self.knob)
            .field("drag", &self.drag)
            .field("parts", &self.parts.len())
            .field("indicator", &self.indicator)
            .field("group_opacity", &self.group_opacity)
            .field("group_blend_space", &self.group_blend_space)
            .field("motion", self.chrome.motion())
            .finish()
    }
}

#[cfg(waterui_feature = "slider")]
impl SliderLook {
    /// A slider laid out by `layout` around `knob`, its values running with
    /// `motion` and dragged by `drag`; it draws nothing until parts are
    /// added.
    #[must_use]
    pub fn new(layout: SliderLayout, knob: Knob, drag: DragMapping, motion: Motion) -> Self {
        use waterui_graphics::color::token::Foreground;

        // The chrome carries what Hydrolysis binds interaction with — the
        // motion and the touch region — and draws nothing of its own: a
        // slider's parts depend on its value.
        let chrome = Chrome::new(
            Surface::clear(CornerShape::radius(0.0)),
            StateLayer::new(Color::new(Foreground)),
            PressFeedback::new(),
            FocusIndicator::Tint,
            motion,
        )
        .hit(knob.hit.press_delay(Duration::ZERO));
        Self {
            chrome,
            layout,
            knob,
            drag,
            parts: Vec::new(),
            indicator: None,
            group_opacity: Driven::constant(1.0),
            group_blend_space: None,
        }
    }

    /// Draws `part` over the parts already added.
    #[must_use]
    pub fn part<D: DrivenSet<f64> + 'static>(mut self, part: Custom<f64, D>) -> Self
    where
        D::Live: 'static,
    {
        self.parts.push(Rc::new(part));
        self
    }

    /// Shows the value above the knob while the slider is held, when the
    /// slider has a value indicator.
    #[must_use]
    pub fn value_indicator(mut self, indicator: ValueIndicator) -> Self {
        self.indicator = Some(indicator);
        self
    }

    /// Draws the whole slider as one group at `opacity` — a disabled slider
    /// that fades as a whole.
    #[must_use]
    pub fn group_opacity(mut self, opacity: impl Into<Driven<f32>>) -> Self {
        self.group_opacity = opacity.into();
        self
    }

    /// The space the group of [`Self::group_opacity`] composites in:
    /// its parts composite with each other in it, and the group fades onto
    /// what lies beneath in it. With none set, the group composites in the
    /// space of the level beneath it.
    #[must_use]
    pub const fn group_blend_space(mut self, space: BlendSpace) -> Self {
        self.group_blend_space = Some(space);
        self
    }

    /// The measures the slider is laid out by.
    #[must_use]
    pub const fn layout(&self) -> &SliderLayout {
        &self.layout
    }

    /// The knob.
    #[must_use]
    pub const fn knob(&self) -> &Knob {
        &self.knob
    }

    /// How a drag maps to the value.
    #[must_use]
    pub const fn get_drag(&self) -> DragMapping {
        self.drag
    }

    /// The motion the look's values run with.
    #[must_use]
    pub fn motion(&self) -> &Motion {
        self.chrome.motion()
    }

    /// The value indicator.
    #[must_use]
    pub const fn get_value_indicator(&self) -> Option<&ValueIndicator> {
        self.indicator.as_ref()
    }

    /// The group opacity.
    #[must_use]
    pub const fn get_group_opacity(&self) -> &Driven<f32> {
        &self.group_opacity
    }

    /// The space the group composites in, when one is set.
    #[must_use]
    pub const fn get_group_blend_space(&self) -> Option<BlendSpace> {
        self.group_blend_space
    }

    /// The number of parts the look draws.
    #[must_use]
    pub fn part_count(&self) -> usize {
        self.parts.len()
    }

    /// The chrome Hydrolysis binds the slider's interaction with.
    #[doc(hidden)]
    #[must_use]
    pub const fn chrome(&self) -> &Chrome {
        &self.chrome
    }

    /// The parts, in paint order.
    #[doc(hidden)]
    #[must_use]
    pub fn parts(&self) -> &[Rc<dyn CustomPart<f64>>] {
        &self.parts
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use approx::assert_relative_eq;
    use cherenkov_record::Repeat;
    use waterui_core::animation::Animation;

    use super::{smoothstep, windowed_keyframes_raw};

    #[test]
    fn smoothstep_matches_hermite_polynomial_across_window() {
        assert_relative_eq!(smoothstep(0.05, 0.55, 0.0), 0.0);
        assert_relative_eq!(smoothstep(0.05, 0.55, 0.05), 0.0);
        assert_relative_eq!(smoothstep(0.05, 0.55, 0.30), 0.5, epsilon = 1e-12);
        // s = 0.25 -> 3*(1/16) - 2*(1/64) = 5/32 = 0.15625
        assert_relative_eq!(smoothstep(0.05, 0.55, 0.175), 0.15625, epsilon = 1e-12);
        assert_relative_eq!(smoothstep(0.05, 0.55, 0.55), 1.0);
        assert_relative_eq!(smoothstep(0.05, 0.55, 1.0), 1.0);
    }

    #[test]
    fn windowed_keyframes_sample_exact_smoothstep_on_enter_and_mirrored_exit() {
        let linear = Animation::linear(Duration::from_millis(1000));
        let enter = windowed_keyframes_raw(0.0_f32, 1.0_f32, &linear, (0.05, 0.55), false)
            .expect("linear animation produces windowed keyframes");

        // Before edge0 (20ms = 0.02): holds 0.0
        assert_relative_eq!(
            enter
                .sample(Duration::from_millis(20), Repeat::ONCE)
                .unwrap(),
            0.0_f32,
            epsilon = 1e-5
        );
        // Quarter of window (175ms = 0.175): exact smoothstep(0.25) = 0.15625
        assert_relative_eq!(
            enter
                .sample(Duration::from_millis(175), Repeat::ONCE)
                .unwrap(),
            0.15625_f32,
            epsilon = 1e-4
        );
        // Midpoint of window (300ms = 0.30): exact 0.5
        assert_relative_eq!(
            enter
                .sample(Duration::from_millis(300), Repeat::ONCE)
                .unwrap(),
            0.5_f32,
            epsilon = 1e-4
        );
        // After edge1 (700ms = 0.70): holds 1.0
        assert_relative_eq!(
            enter
                .sample(Duration::from_millis(700), Repeat::ONCE)
                .unwrap(),
            1.0_f32,
            epsilon = 1e-5
        );

        // Exit mirrors (0.05, 0.55) to (0.45, 0.95) moving 1.0 -> 0.0
        let exit = windowed_keyframes_raw(1.0_f32, 0.0_f32, &linear, (0.05, 0.55), true)
            .expect("exit linear animation produces mirrored windowed keyframes");
        assert_relative_eq!(
            exit.sample(Duration::from_millis(300), Repeat::ONCE)
                .unwrap(),
            1.0_f32,
            epsilon = 1e-5
        );
        assert_relative_eq!(
            exit.sample(Duration::from_millis(700), Repeat::ONCE)
                .unwrap(),
            0.5_f32,
            epsilon = 1e-4
        );
        assert_relative_eq!(
            exit.sample(Duration::from_millis(980), Repeat::ONCE)
                .unwrap(),
            0.0_f32,
            epsilon = 1e-5
        );
    }

    #[test]
    fn windowed_keyframes_support_springs() {
        let spring = Animation::spring(300.0, 30.0);
        let track = windowed_keyframes_raw(0.0_f32, 1.0_f32, &spring, (0.05, 0.55), false)
            .expect("spring produces windowed keyframes");
        assert!(track.duration() > Duration::ZERO);
        assert_relative_eq!(
            track.sample(Duration::ZERO, Repeat::ONCE).unwrap(),
            0.0_f32,
            epsilon = 1e-5
        );
        let near_end = track.duration().saturating_sub(Duration::from_millis(1));
        assert_relative_eq!(
            track.sample(near_end, Repeat::ONCE).unwrap(),
            1.0_f32,
            epsilon = 1e-4
        );
    }
}
