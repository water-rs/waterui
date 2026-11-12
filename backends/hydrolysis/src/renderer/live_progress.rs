//! Hydrolysis's realization of a progress indicator's [`ProgressControl`]:
//! its parts recorded once, with every [`Driven`](crate::theme::Driven)
//! value a live operand.
//!
//! A [`LiveProgress`] keeps the indicator's value and retargets the parts'
//! operands when it changes, each under the operand's
//! [`ValueMotion`](crate::theme::ValueMotion) read through the change's
//! [`ChangeAnimation`]; the engine animates the recorded commands in place,
//! so a value change records nothing, walks no tree and runs no layout
//! pass. The motion of an indeterminate indicator is the engine's: the
//! parts' repeating values start along their tracks when the part records,
//! and a part that rotates has its layer's rotation bound to a track the
//! engine runs, so a running indicator does no work here at all.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use cherenkov_record::Recorder;
use kurbo::{Rect, Vec2};
use nami::{Computed, Signal};
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::interaction::InteractionState;

use crate::renderer::chrome::LiveSource;
use crate::renderer::recording::{LoweredResources, SceneResources};
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::theme::{Change, Inputs, LayerRotation, ProgressControl, RealizedCustom, Retarget};

/// The value an indicator's parts read: a determinate value held to
/// `0.0..=1.0`, and zero for an indeterminate one.
const fn normalized(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// One progress control's parts, recorded once with live operands.
///
/// The indicator keeps it across renders while its [`ProgressControl`] and
/// frame stay the same.
pub struct LiveProgress {
    control: ProgressControl,
    /// The control's frame in the widget's record space.
    frame: Rect,
    value: Cell<f64>,
    /// The parts, bottom first.
    parts: Vec<RealizedCustom<f64>>,
    /// Every operand of the control, for retargeting.
    drivens: Vec<Rc<dyn Retarget<f64>>>,
    /// The subscriptions to the value and to the signals the parts' values
    /// follow.
    guards: RefCell<Vec<Box<dyn Any>>>,
}

impl std::fmt::Debug for LiveProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveProgress")
            .field("frame", &self.frame)
            .field("value", &self.value.get())
            .finish_non_exhaustive()
    }
}

impl LiveProgress {
    /// The control realized on `frame`, showing `value`.
    fn new(control: ProgressControl, frame: Rect, value: f64) -> Self {
        let value = normalized(value);
        let inputs = Inputs::at_rest(frame, InteractionState::empty(), &value);
        let parts = control.realize(&inputs);
        let drivens = parts
            .iter()
            .flat_map(|part| part.drivens.iter().cloned())
            .collect();
        Self {
            control,
            frame,
            value: Cell::new(value),
            parts,
            drivens,
            guards: RefCell::new(Vec::new()),
        }
    }

    /// Subscribes the indicator to `value` and to every signal its parts'
    /// values follow, so a change retargets the operands under the
    /// animation the change carries.
    fn subscribe(this: &Rc<Self>, value: &Computed<f64>) {
        let mut guards: Vec<Box<dyn Any>> = Vec::new();
        let weak: Weak<Self> = Rc::downgrade(this);
        guards.push(Box::new(value.watch(move |context| {
            if let Some(progress) = weak.upgrade() {
                progress.change_value(*context.value(), &ChangeAnimation::of(context.metadata()));
            }
        })));
        for driven in &this.drivens {
            let weak: Weak<Self> = Rc::downgrade(this);
            let target = Rc::downgrade(driven);
            let notify: Rc<dyn Fn(Option<Animation>)> =
                Rc::new(move |animation: Option<Animation>| {
                    if let (Some(progress), Some(target)) = (weak.upgrade(), target.upgrade()) {
                        progress.with_inputs(|inputs| {
                            target.retarget(inputs, Change::Animated(animation.as_ref()));
                        });
                    }
                });
            if let Some(guard) = driven.subscribe(notify) {
                guards.push(guard);
            }
        }
        *this.guards.borrow_mut() = guards;
    }

    fn with_inputs<R>(&self, f: impl FnOnce(&Inputs<'_, f64>) -> R) -> R {
        let value = self.value.get();
        f(&Inputs::at_rest(
            self.frame,
            InteractionState::empty(),
            &value,
        ))
    }

    /// The indicator's value became `value` under `change`: its operands
    /// move under their value motion.
    fn change_value(&self, value: f64, change: &ChangeAnimation) {
        let value = normalized(value);
        if self.value.replace(value).to_bits() == value.to_bits() {
            return;
        }
        let motion = self.control.motion();
        self.with_inputs(|inputs| {
            for driven in &self.drivens {
                driven.retarget(inputs, Change::Value { change, motion });
            }
        });
    }

    /// Whether this indicator realizes `control` on `frame`.
    fn realizes(&self, control: &ProgressControl, frame: Rect) -> bool {
        self.control.same(control) && self.frame == frame
    }
}

/// The source that records one part of an indicator: its own engine layer,
/// so a part that rotates turns alone.
struct PartSource {
    progress: Rc<LiveProgress>,
    index: usize,
}

impl LiveSource for PartSource {
    fn record(
        &self,
        recorder: &mut Recorder,
        _resources: &SceneResources,
    ) -> Option<LoweredResources> {
        self.progress.parts[self.index].record(recorder, self.progress.frame);
        None
    }

    fn rotations(&self) -> Vec<LayerRotation> {
        self.progress.parts[self.index].rotations().to_vec()
    }

    fn pivot(&self) -> Vec2 {
        self.progress.frame.center().to_vec2()
    }
}

/// An indicator's live control and the sources its program lists: the
/// identity the mount keeps the recordings by.
#[derive(Clone)]
pub struct LiveProgressParts {
    progress: Rc<LiveProgress>,
    sources: Vec<Rc<dyn LiveSource>>,
}

impl LiveProgressParts {
    fn new(progress: LiveProgress, value: &Computed<f64>) -> Self {
        let progress = Rc::new(progress);
        LiveProgress::subscribe(&progress, value);
        let sources = (0..progress.parts.len())
            .map(|index| -> Rc<dyn LiveSource> {
                Rc::new(PartSource {
                    progress: Rc::clone(&progress),
                    index,
                })
            })
            .collect();
        Self { progress, sources }
    }

    /// The control `control` on `frame` showing `value`: the parts in
    /// `slot` when they already realize it, otherwise fresh parts recorded
    /// on the next commit, replacing them.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        control: &ProgressControl,
        frame: Rect,
        value: &Computed<f64>,
    ) -> &'a Self {
        if !slot
            .as_ref()
            .is_some_and(|parts| parts.progress.realizes(control, frame))
        {
            *slot = Some(Self::new(
                LiveProgress::new(control.clone(), frame, value.snapshot()),
                value,
            ));
        }
        slot.as_ref().expect("the progress parts were just ensured")
    }

    /// Lists the control's parts in `ctx`, bottom first, a part with an
    /// opacity of its own in a live scope following it.
    pub(crate) fn push<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        for (index, (source, part)) in self.sources.iter().zip(&self.progress.parts).enumerate() {
            let key = crate::renderer::mount::ScopeKey {
                role: "progress-part",
                item: index as u64,
            };
            renderer.with_part_opacity(key, part.opacity(), ctx, |renderer| {
                renderer.push_live(ctx, source);
            });
        }
    }
}
