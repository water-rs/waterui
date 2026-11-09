//! Hydrolysis's realization of the theme's chrome primitives
//! ([`Chrome`](crate::theme::Chrome)).
//!
//! A [`LiveChrome`] records a widget part's chrome once, as two live
//! sources the node's program lists around the widget's content: the
//! surface below it (elevation, fill, border) and the interaction layers
//! above it (state layer, ripple waves, focus ring). Every value that
//! follows interaction state is an [`Operand`] — a signal the recording
//! subscribes to — so a hover, press or focus change sets operands under
//! the theme's [`InteractionMotion`] animation and the engine animates the
//! recorded commands in place: nothing re-records, no node is marked and no
//! layout pass runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cherenkov_record::{Draw as _, Fixed, Live, Paint, Recorder, Shadow, ShapeData, Stroke};
use kurbo::{Point, Rect};
use nami::Signal;
use nami::watcher::{Context, Metadata, WatcherManager, WatcherManagerGuard};
use waterui_core::animation::Animation;
use waterui_core::interaction::InteractionState;
use waterui_graphics::animation::engine_animation;

use crate::renderer::recording::{LoweredResources, SceneResources};

use crate::theme::{
    Chrome, Elevation, FocusIndicator, InteractionMotion, MAX_PRESS_WAVES, PressFeedback, Ripple,
};

/// Content a node's program lists as a live item: recorded once into its
/// own engine layer and kept while the program lists the same source.
///
/// A source may contain fixed commands or operands read as signals. Signal
/// updates reach the engine without another recording. The mount re-records
/// only when the program lists a different source in its place, or when the
/// node mounts on a new engine surface.
pub trait LiveSource {
    /// Records the source's commands into `recorder`, in the record space of
    /// the item's transform. Returned registrations stay alive with the
    /// installed content, including while its retirement is pending.
    fn record(
        &self,
        recorder: &mut Recorder,
        resources: &SceneResources,
    ) -> Option<LoweredResources>;

    /// Fixed content available to scene inspection, when this source owns a recording.
    fn recording(&self) -> Option<&crate::renderer::recording::Recording> {
        None
    }
}

/// A value a recording reads live: a signal that carries the animation each
/// change was set under, so the engine animates the change itself.
struct Operand<T: 'static>(Rc<OperandCell<T>>);

struct OperandCell<T: 'static> {
    value: RefCell<T>,
    watchers: WatcherManager<T>,
}

impl<T> Clone for Operand<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Clone + PartialEq + 'static> Operand<T> {
    fn new(value: T) -> Self {
        Self(Rc::new(OperandCell {
            value: RefCell::new(value),
            watchers: WatcherManager::new(),
        }))
    }

    /// Moves the operand to `value`, animated with `animation` or snapped
    /// when `None`. An unchanged value notifies nothing.
    fn set(&self, value: T, animation: Option<&Animation>) {
        if *self.0.value.borrow() == value {
            return;
        }
        *self.0.value.borrow_mut() = value.clone();
        let metadata = animation.map_or_else(Metadata::new, |animation| {
            Metadata::new().with(engine_animation(animation))
        });
        self.0.watchers.notify_owned(Context::new(value, metadata));
    }

    fn live(&self) -> Live<T> {
        Live::from(self.clone())
    }
}

impl<T: Clone + PartialEq + 'static> Signal for Operand<T> {
    type Output = T;
    type Guard = WatcherManagerGuard<T>;

    fn snapshot(&self) -> T {
        self.0.value.borrow().clone()
    }

    fn watch(&self, watcher: impl Fn(Context<T>) + 'static) -> Self::Guard {
        self.0.watchers.register_as_guard(watcher)
    }
}

/// The focused editor's caret, retained across animation ticks.
pub(super) struct LiveCaret {
    rect: Operand<Rect>,
    clip: Operand<Rect>,
    paint: Operand<Paint>,
    group: Operand<cherenkov_record::Group>,
}

impl LiveCaret {
    pub(super) fn new(rect: Rect, clip: Rect, paint: Paint, alpha: f32) -> Self {
        Self {
            rect: Operand::new(rect),
            clip: Operand::new(clip),
            paint: Operand::new(paint),
            group: Operand::new(cherenkov_record::Group::new().opacity(alpha)),
        }
    }

    pub(super) fn update(&self, rect: Rect, clip: Rect, paint: Paint, alpha: f32) {
        self.rect.set(rect, None);
        self.clip.set(clip, None);
        self.paint.set(paint, None);
        self.group
            .set(cherenkov_record::Group::new().opacity(alpha), None);
    }
}

impl LiveSource for LiveCaret {
    fn record(
        &self,
        recorder: &mut Recorder,
        _resources: &SceneResources,
    ) -> Option<LoweredResources> {
        recorder.clip(self.clip.live(), |recorder| {
            recorder.group(self.group.live(), |recorder| {
                recorder.fill(self.rect.live(), self.paint.live());
            });
        });
        None
    }
}

struct BorderOperands {
    shape: Operand<ShapeData>,
    stroke: Operand<Stroke>,
    paint: Operand<Paint>,
}

struct WaveOperands {
    shape: Operand<ShapeData>,
    paint: Operand<Paint>,
}

struct RingOperands {
    shape: Operand<ShapeData>,
    stroke: Stroke,
    paint: Operand<Paint>,
}

/// One widget part's chrome, recorded once with live operands.
///
/// The widget keeps it across renders while its [`Chrome`], frame and
/// motion stay the same; the interaction handles bound to the widget steer
/// it between renders through [`Self::set_flag`], [`Self::begin_wave`] and
/// [`Self::fade_wave`].
pub struct LiveChrome {
    spec: Chrome,
    /// The chrome's frame in the widget's record space.
    frame: Rect,
    motion: InteractionMotion,
    state: Cell<InteractionState>,
    /// The widget's own reported-state binding, kept current between
    /// renders so content resolving per-state values follows the chrome.
    report: nami::Binding<InteractionState>,
    shape: Operand<ShapeData>,
    fill: Operand<Paint>,
    border: Option<BorderOperands>,
    /// The key and ambient shadows; `None` when no state elevates.
    elevation: Option<(Operand<Shadow>, Operand<Shadow>)>,
    tint: Operand<Paint>,
    waves: Option<(Ripple, [WaveOperands; MAX_PRESS_WAVES])>,
    ring: Option<RingOperands>,
    /// How many times the mount recorded either half.
    #[cfg(test)]
    recordings: Cell<u32>,
}

impl std::fmt::Debug for LiveChrome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveChrome")
            .field("frame", &self.frame)
            .field("state", &self.state.get())
            .finish_non_exhaustive()
    }
}

/// Every interaction state, for resolving which values a surface can take.
fn every_state() -> impl Iterator<Item = InteractionState> {
    (0..=InteractionState::all().bits()).map(InteractionState::from_bits_truncate)
}

impl LiveChrome {
    /// The chrome `spec` on `bounds` (the widget's record-space bounds), at
    /// rest in `state`, steered with `motion`. `report` is the widget's
    /// reported-state binding.
    fn new(
        spec: Chrome,
        bounds: Rect,
        motion: InteractionMotion,
        state: InteractionState,
        report: nami::Binding<InteractionState>,
    ) -> Self {
        let frame = spec.frame(bounds);
        let pressed = state.contains(InteractionState::PRESSED);
        let surface = &spec.surface();
        let border = surface.get_border().as_ref().map(|border| {
            let resolved = border.resolve(state);
            BorderOperands {
                shape: Operand::new(
                    surface
                        .shape()
                        .outset(frame, -resolved.width / 2.0, pressed),
                ),
                stroke: Operand::new(Stroke::new(resolved.width)),
                paint: Operand::new(resolved.paint.clone()),
            }
        });
        let elevates = every_state().any(|state| !surface.get_elevation().resolve(state).is_none());
        let elevation = elevates.then(|| {
            let resolved = surface.get_elevation().resolve(state);
            (Operand::new(resolved.key), Operand::new(resolved.ambient))
        });
        let waves = match spec.press() {
            PressFeedback::Ripple(ripple) => Some((
                ripple,
                core::array::from_fn(|_| WaveOperands {
                    shape: Operand::new(ShapeData::Circle(kurbo::Circle::new(frame.center(), 0.0))),
                    paint: Operand::new(spec.state_layer().paint(0.0)),
                }),
            )),
            PressFeedback::Tint => None,
        };
        let ring = match spec.focus() {
            FocusIndicator::Ring {
                width,
                offset,
                color,
            } => Some(RingOperands {
                shape: Operand::new(surface.shape().outset(frame, offset + width / 2.0, pressed)),
                stroke: Stroke::new(width),
                paint: Operand::new(Self::ring_paint(color, state)),
            }),
            FocusIndicator::Tint => None,
        };
        Self {
            shape: Operand::new(surface.shape().shape(frame, pressed)),
            fill: Operand::new(surface.fill().resolve(state).clone()),
            border,
            elevation,
            tint: Operand::new(
                spec.state_layer()
                    .paint(spec.state_layer().opacity(state, &spec.press())),
            ),
            waves,
            ring,
            state: Cell::new(state),
            report,
            spec,
            frame,
            motion,
            #[cfg(test)]
            recordings: Cell::new(0),
        }
    }

    fn ring_paint(color: cherenkov_record::WorkingColor, state: InteractionState) -> Paint {
        let alpha = if state.contains(InteractionState::FOCUSED)
            && !state.contains(InteractionState::DISABLED)
        {
            color.components[3]
        } else {
            0.0
        };
        Paint::from(color.with_alpha(alpha))
    }

    /// The interaction state the chrome's operands show.
    #[cfg(test)]
    pub(crate) const fn state(&self) -> InteractionState {
        self.state.get()
    }

    /// How many times the mount recorded either half of this chrome.
    #[cfg(test)]
    pub(crate) const fn recordings(&self) -> u32 {
        self.recordings.get()
    }

    /// Whether this chrome realizes `spec` on `bounds` with `motion`.
    fn realizes(&self, spec: &Chrome, bounds: Rect, motion: &InteractionMotion) -> bool {
        self.spec == *spec && self.frame == spec.frame(bounds) && self.motion == *motion
    }

    /// Moves every state-dependent operand to `state`. A change of a
    /// physical press animates with the press motion, a focus change with
    /// the focus motion, a hover change with the hover motion; any other
    /// change (disabled, selected) is applied by a render and snaps.
    pub(crate) fn sync(&self, state: InteractionState) {
        let old = self.state.replace(state);
        if old == state {
            return;
        }
        if self.report.snapshot() != state {
            self.report.set(state);
        }
        let changed = old ^ state;
        let motion = &self.motion;
        let animation = if changed.contains(InteractionState::PRESSED) {
            Some(if state.contains(InteractionState::PRESSED) {
                &motion.press_fade_in
            } else {
                &motion.press_fade_out
            })
        } else if changed.contains(InteractionState::FOCUSED) {
            Some(if state.contains(InteractionState::FOCUSED) {
                &motion.focus_enter
            } else {
                &motion.focus_exit
            })
        } else if changed.contains(InteractionState::HOVERED) {
            Some(if state.contains(InteractionState::HOVERED) {
                &motion.hover_enter
            } else {
                &motion.hover_exit
            })
        } else {
            None
        };
        let pressed = state.contains(InteractionState::PRESSED);
        let shape_animation = changed
            .contains(InteractionState::PRESSED)
            .then_some(&motion.press_shape);
        let surface = &self.spec.surface();
        self.shape
            .set(surface.shape().shape(self.frame, pressed), shape_animation);
        self.fill
            .set(surface.fill().resolve(state).clone(), animation);
        if let (Some(operands), Some(border)) = (&self.border, &surface.get_border()) {
            let resolved = border.resolve(state);
            operands.shape.set(
                surface
                    .shape()
                    .outset(self.frame, -resolved.width / 2.0, pressed),
                shape_animation,
            );
            operands.stroke.set(Stroke::new(resolved.width), animation);
            operands.paint.set(resolved.paint.clone(), animation);
        }
        if let Some((key, ambient)) = &self.elevation {
            let Elevation {
                key: key_shadow,
                ambient: ambient_shadow,
            } = *surface.get_elevation().resolve(state);
            key.set(key_shadow, animation);
            ambient.set(ambient_shadow, animation);
        }
        let layer = &self.spec.state_layer();
        self.tint.set(
            layer.paint(layer.opacity(state, &self.spec.press())),
            animation,
        );
        if let (
            Some(ring),
            FocusIndicator::Ring {
                width,
                offset,
                color,
            },
        ) = (&self.ring, self.spec.focus())
        {
            ring.shape.set(
                surface
                    .shape()
                    .outset(self.frame, offset + width / 2.0, pressed),
                shape_animation,
            );
            let focus_animation = if changed.contains(InteractionState::FOCUSED) {
                Some(if state.contains(InteractionState::FOCUSED) {
                    &motion.focus_enter
                } else {
                    &motion.focus_exit
                })
            } else {
                animation
            };
            ring.paint
                .set(Self::ring_paint(color, state), focus_animation);
        }
    }

    /// Sets or clears one live interaction flag (hover, press or focus).
    pub(crate) fn set_flag(&self, flag: InteractionState, on: bool) {
        let mut state = self.state.get();
        if state.contains(InteractionState::DISABLED) {
            return;
        }
        state.set(flag, on);
        self.sync(state);
    }

    /// Starts ripple wave `index` for a press at `origin`, in the widget's
    /// record space (`None` presses at the frame's centre): the wave snaps to
    /// its starting circle, then grows under the press-grow motion while it
    /// fades in.
    pub(crate) fn begin_wave(&self, index: usize, origin: Option<Point>) {
        let Some((ripple, waves)) = &self.waves else {
            return;
        };
        let (start, end) = ripple.wave(self.frame, origin);
        let wave = &waves[index];
        wave.shape.set(ShapeData::Circle(start), None);
        wave.shape
            .set(ShapeData::Circle(end), Some(&self.motion.press_grow));
        let layer = &self.spec.state_layer();
        wave.paint
            .set(layer.paint(layer.pressed), Some(&self.motion.press_fade_in));
    }

    /// Fades ripple wave `index` out under the press fade-out motion; its
    /// geometry holds.
    pub(crate) fn fade_wave(&self, index: usize) {
        let Some((_, waves)) = &self.waves else {
            return;
        };
        waves[index].paint.set(
            self.spec.state_layer().paint(0.0),
            Some(&self.motion.press_fade_out),
        );
    }

    /// Drops every wave without animating.
    pub(crate) fn clear_waves(&self) {
        let Some((_, waves)) = &self.waves else {
            return;
        };
        for wave in waves {
            wave.paint.set(self.spec.state_layer().paint(0.0), None);
        }
    }

    fn record_below(&self, recorder: &mut Recorder) {
        if let Some((key, ambient)) = &self.elevation {
            recorder.shadow(self.shape.live(), ambient.live());
            recorder.shadow(self.shape.live(), key.live());
        }
        recorder.fill(self.shape.live(), self.fill.live());
        if let Some(border) = &self.border {
            recorder.stroke(
                border.shape.live(),
                border.stroke.live(),
                border.paint.live(),
            );
        }
    }

    fn record_above(&self, recorder: &mut Recorder) {
        recorder.fill(self.shape.live(), self.tint.live());
        if let Some((_, waves)) = &self.waves {
            recorder.clip(self.shape.live(), |recorder| {
                for wave in waves {
                    recorder.fill(wave.shape.live(), wave.paint.live());
                }
            });
        }
        if let Some(ring) = &self.ring {
            recorder.stroke(
                ring.shape.live(),
                Fixed(ring.stroke.clone()),
                ring.paint.live(),
            );
        }
    }
}

/// Which half of a [`LiveChrome`] a source records.
#[derive(Clone, Copy)]
enum Half {
    /// Elevation, fill and border: beneath the widget's content.
    Below,
    /// State layer, ripple waves and focus ring: above it.
    Above,
}

struct ChromeSource {
    chrome: Rc<LiveChrome>,
    half: Half,
}

impl LiveSource for ChromeSource {
    fn record(
        &self,
        recorder: &mut Recorder,
        _resources: &SceneResources,
    ) -> Option<LoweredResources> {
        #[cfg(test)]
        self.chrome.recordings.set(self.chrome.recordings.get() + 1);
        match self.half {
            Half::Below => self.chrome.record_below(recorder),
            Half::Above => self.chrome.record_above(recorder),
        }
        None
    }
}

/// A widget part's live chrome and the two sources its program lists: the
/// identity the mount keeps the recordings by.
pub struct LiveChromeParts {
    chrome: Rc<LiveChrome>,
    below: Rc<dyn LiveSource>,
    above: Rc<dyn LiveSource>,
}

impl LiveChromeParts {
    fn new(chrome: LiveChrome) -> Self {
        let chrome = Rc::new(chrome);
        Self {
            below: Rc::new(ChromeSource {
                chrome: Rc::clone(&chrome),
                half: Half::Below,
            }),
            above: Rc::new(ChromeSource {
                chrome: Rc::clone(&chrome),
                half: Half::Above,
            }),
            chrome,
        }
    }

    /// The chrome `spec` on `bounds` with `motion`: the parts in `slot` when
    /// they already realize it — moved to `state` — otherwise fresh parts
    /// recorded at `state`, replacing them.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        spec: Chrome,
        bounds: Rect,
        motion: InteractionMotion,
        state: InteractionState,
        report: &nami::Binding<InteractionState>,
    ) -> &'a Self {
        match slot {
            Some(parts) if parts.chrome.realizes(&spec, bounds, &motion) => {
                parts.chrome.sync(state);
            }
            _ => {
                *slot = Some(Self::new(LiveChrome::new(
                    spec,
                    bounds,
                    motion,
                    state,
                    report.clone(),
                )));
            }
        }
        slot.as_ref().expect("the chrome parts were just ensured")
    }

    pub(crate) const fn chrome(&self) -> &Rc<LiveChrome> {
        &self.chrome
    }

    pub(crate) const fn below(&self) -> &Rc<dyn LiveSource> {
        &self.below
    }

    pub(crate) const fn above(&self) -> &Rc<dyn LiveSource> {
        &self.above
    }
}
