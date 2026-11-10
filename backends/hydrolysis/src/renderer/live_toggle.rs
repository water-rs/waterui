//! Hydrolysis's realization of a toggle's [`ToggleControl`]: its parts
//! recorded once, with every [`Driven`](crate::theme::Driven) value a live
//! operand.
//!
//! A [`LiveToggle`] keeps the toggle's inputs — the value, the interaction
//! state, the press point and the drag — and retargets the parts' operands
//! when one changes, each under the motion its change runs with: the
//! control's [`Motion`](crate::theme::Motion) for an interaction change, the
//! operand's [`ValueMotion`](crate::theme::ValueMotion) for a value change
//! (read through the change's
//! [`ChangeAnimation`](waterui_core::animation::ChangeAnimation)), none for
//! a drag that tracks the finger. The engine animates the recorded commands
//! in place, so a toggle, hover, press or drag records nothing, walks no
//! tree and runs no layout pass.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use cherenkov_record::Recorder;
use kurbo::{Point, Rect, Vec2};
use nami::{Binding, Signal};
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::interaction::InteractionState;

use crate::renderer::chrome::LiveSource;
use crate::renderer::recording::{LoweredResources, SceneResources};
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::theme::{Change, Inputs, InteractionMotion, RealizedCustom, Retarget, ToggleControl};

/// One toggle control's parts, recorded once with live operands.
///
/// The toggle keeps it across renders while its [`ToggleControl`] and frame
/// stay the same; the interaction handles bound to the toggle steer it
/// between renders through [`Self::set_flag`] and [`Self::begin_wave`], and
/// the toggle's drag through [`Self::begin_drag`], [`Self::drag_to`] and
/// [`Self::end_drag`].
pub struct LiveToggle {
    control: ToggleControl,
    /// The control's frame in the widget's record space.
    frame: Rect,
    value: Cell<bool>,
    state: Cell<InteractionState>,
    press_point: Cell<Option<Point>>,
    drag: Cell<Option<Vec2>>,
    parts: Vec<RealizedCustom<bool>>,
    /// The subscriptions to the value and to the signals the parts' values
    /// follow.
    guards: RefCell<Vec<Box<dyn Any>>>,
    /// How many times the mount recorded the parts.
    #[cfg(test)]
    recordings: Cell<u32>,
}

impl std::fmt::Debug for LiveToggle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveToggle")
            .field("frame", &self.frame)
            .field("value", &self.value.get())
            .field("state", &self.state.get())
            .finish_non_exhaustive()
    }
}

impl LiveToggle {
    /// The control realized on `frame`, at rest in `state` showing `value`.
    fn new(control: ToggleControl, frame: Rect, state: InteractionState, value: bool) -> Self {
        let inputs = Inputs::at_rest(frame, state, &value);
        let parts = control
            .parts()
            .iter()
            .map(|part| part.realize(&inputs))
            .collect();
        Self {
            control,
            frame,
            value: Cell::new(value),
            state: Cell::new(state),
            press_point: Cell::new(None),
            drag: Cell::new(None),
            parts,
            guards: RefCell::new(Vec::new()),
            #[cfg(test)]
            recordings: Cell::new(0),
        }
    }

    /// Subscribes the toggle to `binding` and to every signal its parts'
    /// values follow, so a change retargets the operands under the
    /// animation the change carries.
    fn subscribe(this: &Rc<Self>, binding: &Binding<bool>) {
        let mut guards: Vec<Box<dyn Any>> = Vec::new();
        let weak: Weak<Self> = Rc::downgrade(this);
        guards.push(Box::new(binding.watch(move |context| {
            if let Some(toggle) = weak.upgrade() {
                toggle.change_value(*context.value(), &ChangeAnimation::of(context.metadata()));
            }
        })));
        for driven in this.drivens() {
            let weak: Weak<Self> = Rc::downgrade(this);
            let target = Rc::downgrade(&driven);
            let notify: Rc<dyn Fn(Option<Animation>)> =
                Rc::new(move |animation: Option<Animation>| {
                    if let (Some(toggle), Some(target)) = (weak.upgrade(), target.upgrade()) {
                        toggle.retarget_one(&*target, Change::Animated(animation.as_ref()));
                    }
                });
            if let Some(guard) = driven.subscribe(notify) {
                guards.push(guard);
            }
        }
        *this.guards.borrow_mut() = guards;
    }

    fn drivens(&self) -> impl Iterator<Item = Rc<dyn Retarget<bool>>> + '_ {
        self.parts
            .iter()
            .flat_map(|part| part.drivens.iter().cloned())
    }

    fn with_inputs<R>(&self, f: impl FnOnce(&Inputs<'_, bool>) -> R) -> R {
        let value = self.value.get();
        f(&Inputs {
            frame: self.frame,
            state: self.state.get(),
            value: &value,
            drag: self.drag.get(),
            press_point: self.press_point.get(),
        })
    }

    fn retarget_one(&self, driven: &dyn Retarget<bool>, change: Change<'_>) {
        self.with_inputs(|inputs| driven.retarget(inputs, change));
    }

    /// Moves every operand to the toggle's current inputs under `change`.
    fn retarget(&self, change: Change<'_>) {
        self.with_inputs(|inputs| {
            for driven in self.drivens() {
                driven.retarget(inputs, change);
            }
        });
    }

    /// The toggle's value became `value` under `change`: its operands move
    /// under their value motion.
    pub(crate) fn change_value(&self, value: bool, change: &ChangeAnimation) {
        if self.value.replace(value) == value {
            return;
        }
        self.retarget(Change::Value {
            change,
            motion: self.control.motion(),
        });
    }

    /// Moves every operand to `state`, each under its motion's transition
    /// for the change; a change of the disabled or selected state is applied
    /// by a render and snaps.
    pub(crate) fn sync(&self, state: InteractionState) {
        let old = self.state.replace(state);
        if old == state {
            return;
        }
        if !state.contains(InteractionState::PRESSED) {
            self.press_point.set(None);
        }
        self.retarget(Change::Interaction {
            old,
            motion: self.control.motion(),
        });
    }

    /// Sets or clears one live interaction flag (hover, press, focus or
    /// drag).
    pub(crate) fn set_flag(&self, flag: InteractionState, on: bool) {
        let mut state = self.state.get();
        if state.contains(InteractionState::DISABLED) {
            return;
        }
        state.set(flag, on);
        self.sync(state);
    }

    /// Starts a press at `origin`, in the widget's record space (`None`
    /// presses at the frame's centre): the press point the parts read moves
    /// there.
    pub(crate) fn begin_wave(&self, origin: Option<Point>) {
        self.press_point.set(origin);
    }

    /// Starts a drag of the knob: the toggle reads as dragged.
    pub(crate) fn begin_drag(&self) {
        self.set_flag(InteractionState::DRAGGED, true);
    }

    /// Moves the drag to `offset` from where the press landed. The operands
    /// track it with no animation, so they follow the finger.
    pub(crate) fn drag_to(&self, offset: Vec2) {
        self.drag.set(Some(offset));
        self.retarget(Change::Animated(None));
    }

    /// Ends the drag with the toggle settling on `settled`: the value is set
    /// on `binding` when it differs, and the operands settle there under
    /// their value motion from the position and velocity the drag left them
    /// at.
    pub(crate) fn end_drag(&self, settled: bool, binding: &Binding<bool>) {
        self.drag.set(None);
        if binding.snapshot() == settled {
            self.retarget(Change::Value {
                change: &ChangeAnimation::Unspecified,
                motion: self.control.motion(),
            });
        } else {
            // The value's change reaches the operands through the
            // subscription, with the drag already gone.
            binding.set(settled);
        }
        self.set_flag(InteractionState::DRAGGED, false);
    }

    /// The interaction motion the toggle's interaction handles run with:
    /// the engine animates the parts, so the handles' scalars snap and only
    /// the hit region's press delay matters.
    pub(crate) fn handle_motion(control: &ToggleControl) -> InteractionMotion {
        let snap = Animation::linear(core::time::Duration::ZERO);
        InteractionMotion {
            hover_opacity: 0.0,
            focus_opacity: 0.0,
            pressed_opacity: 0.0,
            dragged_opacity: 0.0,
            hover_enter: snap.clone(),
            hover_exit: snap.clone(),
            focus_enter: snap.clone(),
            focus_exit: snap.clone(),
            press_fade_in: snap.clone(),
            press_fade_out: snap.clone(),
            press_grow: snap,
            press_shape: control.motion().press.enter.clone(),
            minimum_press_duration: core::time::Duration::ZERO,
            touch_delay: control.get_hit().get_press_delay(),
        }
    }

    /// Whether this toggle realizes `control` on `frame`.
    fn realizes(&self, control: &ToggleControl, frame: Rect) -> bool {
        self.control.same(control) && self.frame == frame
    }

    /// How many times the mount recorded the parts.
    #[cfg(test)]
    pub(crate) const fn recordings(&self) -> u32 {
        self.recordings.get()
    }

    /// The interaction state the operands show.
    #[cfg(test)]
    pub(crate) const fn state(&self) -> InteractionState {
        self.state.get()
    }
}

/// The source that records a toggle's parts into the live layer.
struct ToggleSource {
    toggle: Rc<LiveToggle>,
}

impl LiveSource for ToggleSource {
    fn record(
        &self,
        recorder: &mut Recorder,
        _resources: &SceneResources,
    ) -> Option<LoweredResources> {
        #[cfg(test)]
        self.toggle.recordings.set(self.toggle.recordings.get() + 1);
        for part in &self.toggle.parts {
            part.record(recorder, self.toggle.frame);
        }
        None
    }
}

/// A toggle's live control and the source its program lists: the identity
/// the mount keeps the recording by.
#[derive(Clone)]
pub struct LiveToggleParts {
    toggle: Rc<LiveToggle>,
    source: Rc<dyn LiveSource>,
}

impl LiveToggleParts {
    fn new(toggle: LiveToggle, binding: &Binding<bool>) -> Self {
        let toggle = Rc::new(toggle);
        LiveToggle::subscribe(&toggle, binding);
        Self {
            source: Rc::new(ToggleSource {
                toggle: Rc::clone(&toggle),
            }),
            toggle,
        }
    }

    /// The control `control` on `frame` showing `binding`: the parts in
    /// `slot` when they already realize it — moved to `state` — otherwise
    /// fresh parts recorded at `state`, replacing them.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        control: &ToggleControl,
        frame: Rect,
        state: InteractionState,
        binding: &Binding<bool>,
    ) -> &'a Self {
        match slot {
            Some(parts) if parts.toggle.realizes(control, frame) => parts.toggle.sync(state),
            _ => {
                *slot = Some(Self::new(
                    LiveToggle::new(control.clone(), frame, state, binding.snapshot()),
                    binding,
                ));
            }
        }
        slot.as_ref().expect("the toggle parts were just ensured")
    }

    pub(crate) const fn toggle(&self) -> &Rc<LiveToggle> {
        &self.toggle
    }

    /// Lists the control's parts in `ctx`.
    pub(crate) fn push<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        renderer.push_live(ctx, &self.source);
    }
}
