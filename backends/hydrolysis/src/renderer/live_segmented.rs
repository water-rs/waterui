//! Hydrolysis's realization of a segmented picker's [`SegmentedLook`]: its
//! parts recorded once, with every [`Driven`](crate::theme::Driven) value a
//! live operand.
//!
//! A [`LiveSegmented`] keeps the picker's inputs — the selection and the
//! thumb's position, each segment's interaction state, the press point and
//! the drag — and retargets the parts' operands when one changes, each
//! under the motion its change runs with: the look's
//! [`Motion`](crate::theme::Motion) for an interaction change, its value
//! motion for a selection change (read through the change's
//! [`ChangeAnimation`]), none for a drag that tracks the finger. The engine
//! animates the recorded commands in place, so a selection change, a hover,
//! a press or a drag records nothing and marks no node.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use cherenkov_record::{Paint, Recorder, WorkingColor};
use kurbo::{Affine, Point, Rect, Vec2};
use nami::{Binding, Computed, Signal};
use waterui_core::Environment;
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::id::Id;
use waterui_core::interaction::InteractionState;

use crate::renderer::chrome::{LayeredParts, LiveChrome, PartsBody, varies_live};
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::theme::live::Operand;
use crate::theme::{
    Change, Chrome, Inputs, InteractionMotion, LiveDriven, RealizedCustom, Retarget, Segment,
    SegmentedLook, SegmentedValue,
};

/// The interaction flags a segment's own pointer and keyboard state set.
const LIVE: InteractionState = InteractionState::HOVERED
    .union(InteractionState::PRESSED)
    .union(InteractionState::FOCUSED);

/// Where a segmented picker's parts lie, in the widget's record space, left
/// to right: the parts record against these frames, and a right-to-left
/// picker lists them mirrored about the row.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedFrames {
    /// The row the control parts draw on.
    pub row: Rect,
    /// Each segment's frame, leading first.
    pub segments: Vec<Rect>,
    /// Whether the parts are listed mirrored about the row.
    pub mirrored: bool,
}

impl SegmentedFrames {
    /// The mirror about the row, the identity when the parts are not
    /// mirrored.
    fn mirror(&self) -> Affine {
        if self.mirrored {
            Affine::new([-1.0, 0.0, 0.0, 1.0, self.row.x0 + self.row.x1, 0.0])
        } else {
            Affine::IDENTITY
        }
    }
}

/// One segmented picker's parts, recorded once with live operands.
///
/// The picker keeps it across renders while its look, its frames, its items
/// and its disabled state stay the same; the interaction handles bound to
/// each segment steer it between renders through [`Self::set_flag`] and
/// [`Self::begin_press`], and the picker's drag through
/// [`Self::begin_drag`], [`Self::drag_to`] and [`Self::end_drag`].
pub struct LiveSegmented {
    look: SegmentedLook,
    frames: SegmentedFrames,
    /// Each segment's item tag, leading first.
    tags: Vec<Id>,
    disabled: bool,
    value: Cell<SegmentedValue>,
    drag: Cell<Option<Vec2>>,
    dragged: Cell<bool>,
    press_point: Cell<Option<Point>>,
    /// Each segment's own hover, press and focus.
    states: Vec<Cell<InteractionState>>,
    below: Vec<RealizedCustom<SegmentedValue>>,
    /// Each segment's chrome, leading segment first; empty when the look
    /// has no segment chrome.
    chromes: Vec<Rc<LiveChrome>>,
    /// Each segment's parts, leading segment first.
    segments: Vec<Vec<RealizedCustom<Segment>>>,
    above: Vec<RealizedCustom<SegmentedValue>>,
    group_opacity: Rc<LiveDriven<f32, ()>>,
    /// Each segment's label colour.
    label_colors: Vec<Rc<LiveDriven<WorkingColor, Segment>>>,
    /// Each segment's label paint, following its colour operand.
    label_paints: Vec<Computed<Paint>>,
    /// The subscriptions to the selection and to the signals the parts'
    /// values follow.
    guards: RefCell<Vec<Box<dyn Any>>>,
    /// How many times the mount recorded the parts.
    #[cfg(test)]
    recordings: Cell<u32>,
}

impl std::fmt::Debug for LiveSegmented {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSegmented")
            .field("frames", &self.frames)
            .field("value", &self.value.get())
            .field("drag", &self.drag.get())
            .finish_non_exhaustive()
    }
}

/// What a signal one of the parts' values follows does to the picker when it
/// changes, under the animation the change carries.
type Retargeting = dyn Fn(&LiveSegmented, Option<&Animation>);

/// The index of the segment `tag` names among `tags`. A tag that names no
/// item is a contract violation by the app, reported; the picker then keeps
/// the selection it shows.
fn index_of(tags: &[Id], tag: Id) -> Option<usize> {
    let index = tags.iter().position(|known| *known == tag);
    if index.is_none() {
        waterui_core::contract_violation!(
            "hydrolysis segmented picker selection is not present in picker items; the picker keeps the segment it shows"
        );
    }
    index
}

impl LiveSegmented {
    /// The look realized on `frames` for the items `tags`, at rest with
    /// `selected` selected, each segment wearing its chrome of `chromes`
    /// (none when it is empty), resolving the colours in `env`.
    fn new(
        look: SegmentedLook,
        frames: SegmentedFrames,
        tags: Vec<Id>,
        selected: usize,
        disabled: bool,
        chromes: &[Chrome],
        env: &Environment,
    ) -> Self {
        let count = tags.len();
        let value = SegmentedValue {
            selected,
            position: position(selected),
        };
        let base = if disabled {
            InteractionState::DISABLED
        } else {
            InteractionState::empty()
        };
        let segment_state = |index: usize| {
            if index == selected {
                base | InteractionState::SELECTED
            } else {
                base
            }
        };
        let control = Inputs::at_rest(frames.row, base, &value);
        let below: Vec<_> = look
            .below()
            .iter()
            .map(|part| part.realize(&control))
            .collect();
        let above: Vec<_> = look
            .above()
            .iter()
            .map(|part| part.realize(&control))
            .collect();
        let segments: Vec<Vec<_>> = frames
            .segments
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let segment = Segment { index, count };
                let inputs = Inputs::at_rest(*frame, segment_state(index), &segment);
                look.segments()
                    .iter()
                    .map(|part| part.realize(&inputs))
                    .collect()
            })
            .collect();
        let chromes = chromes
            .iter()
            .zip(&frames.segments)
            .enumerate()
            .map(|(index, (chrome, frame))| {
                LiveChrome::realize(chrome.clone(), *frame, segment_state(index), env)
            })
            .collect();
        let color = {
            let env = env.clone();
            look.label()
                .get_color()
                .map(move |color| color.resolve(&env).snapshot())
        };
        let label_colors: Vec<_> = frames
            .segments
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let segment = Segment { index, count };
                Rc::new(LiveDriven::new(
                    color.clone(),
                    &Inputs::at_rest(*frame, segment_state(index), &segment),
                ))
            })
            .collect();
        let label_paints = label_colors
            .iter()
            .map(|color| Computed::new(nami::SignalExt::map(color.operand(), Paint::from)))
            .collect();
        let group_opacity = Rc::new(LiveDriven::new(
            look.get_group_opacity().clone(),
            &Inputs::at_rest(frames.row, base, &()),
        ));
        Self {
            states: (0..count)
                .map(|_| Cell::new(InteractionState::empty()))
                .collect(),
            look,
            frames,
            tags,
            disabled,
            value: Cell::new(value),
            drag: Cell::new(None),
            dragged: Cell::new(false),
            press_point: Cell::new(None),
            below,
            chromes,
            segments,
            above,
            group_opacity,
            label_colors,
            label_paints,
            guards: RefCell::new(Vec::new()),
            #[cfg(test)]
            recordings: Cell::new(0),
        }
    }

    /// Subscribes the picker to `selection` and to every signal its parts'
    /// values follow, so a change retargets the operands under the
    /// animation the change carries.
    fn subscribe(this: &Rc<Self>, selection: &Binding<Id>) {
        let mut guards: Vec<Box<dyn Any>> = Vec::new();
        let weak: Weak<Self> = Rc::downgrade(this);
        guards.push(Box::new(selection.watch(move |context| {
            if let Some(picker) = weak.upgrade()
                && let Some(index) = index_of(&picker.tags, *context.value())
            {
                picker.change_selection(index, &ChangeAnimation::of(context.metadata()));
            }
        })));
        let notify = |target: Box<Retargeting>| {
            let weak: Weak<Self> = Rc::downgrade(this);
            let target: Rc<Retargeting> = Rc::from(target);
            Rc::new(move |animation: Option<Animation>| {
                if let Some(picker) = weak.upgrade() {
                    target(&picker, animation.as_ref());
                }
            }) as Rc<dyn Fn(Option<Animation>)>
        };
        for driven in this.control_drivens() {
            let weak_driven = Rc::downgrade(&driven);
            let notify = notify(Box::new(move |picker, animation| {
                if let Some(driven) = weak_driven.upgrade() {
                    picker.with_control_inputs(|inputs| {
                        driven.retarget(inputs, Change::Animated(animation));
                    });
                }
            }));
            guards.extend(driven.subscribe(notify));
        }
        for index in 0..this.tags.len() {
            for driven in this.segment_drivens(index) {
                let weak_driven = Rc::downgrade(&driven);
                let notify = notify(Box::new(move |picker, animation| {
                    if let Some(driven) = weak_driven.upgrade() {
                        picker.with_segment_inputs(index, |inputs| {
                            driven.retarget(inputs, Change::Animated(animation));
                        });
                    }
                }));
                guards.extend(driven.subscribe(notify));
            }
        }
        let group = Rc::downgrade(&this.group_opacity);
        let notify = notify(Box::new(move |picker, animation| {
            if let Some(group) = group.upgrade() {
                picker.with_group_inputs(|inputs| {
                    group.retarget(inputs, Change::Animated(animation));
                });
            }
        }));
        guards.extend(this.group_opacity.subscribe(notify));
        *this.guards.borrow_mut() = guards;
    }

    fn control_drivens(&self) -> impl Iterator<Item = Rc<dyn Retarget<SegmentedValue>>> + '_ {
        self.below
            .iter()
            .chain(&self.above)
            .flat_map(|part| part.drivens.iter().cloned())
    }

    fn segment_drivens(
        &self,
        index: usize,
    ) -> impl Iterator<Item = Rc<dyn Retarget<Segment>>> + '_ {
        self.segments[index]
            .iter()
            .flat_map(|part| part.drivens.iter().cloned())
            .chain(core::iter::once(
                Rc::clone(&self.label_colors[index]) as Rc<dyn Retarget<Segment>>
            ))
    }

    /// The state every part shares: disabled or not.
    const fn base(&self) -> InteractionState {
        if self.disabled {
            InteractionState::DISABLED
        } else {
            InteractionState::empty()
        }
    }

    /// The thumb's state: the selected segment's hover, press and focus,
    /// dragged while a drag moves it, and disabled.
    fn control_state(&self) -> InteractionState {
        let mut state = self.base() | (self.states[self.value.get().selected].get() & LIVE);
        state.set(InteractionState::DRAGGED, self.dragged.get());
        state
    }

    /// Segment `index`'s state: its own hover, press and focus, selected
    /// while it is the selected segment, and disabled.
    fn segment_state(&self, index: usize) -> InteractionState {
        let mut state = self.base() | (self.states[index].get() & LIVE);
        state.set(
            InteractionState::SELECTED,
            index == self.value.get().selected,
        );
        state
    }

    fn with_control_inputs<R>(&self, f: impl FnOnce(&Inputs<'_, SegmentedValue>) -> R) -> R {
        let value = self.value.get();
        f(&Inputs {
            frame: self.frames.row,
            state: self.control_state(),
            value: &value,
            drag: self.drag.get(),
            press_point: self.press_point.get(),
        })
    }

    fn with_segment_inputs<R>(&self, index: usize, f: impl FnOnce(&Inputs<'_, Segment>) -> R) -> R {
        let state = self.segment_state(index);
        let segment = Segment {
            index,
            count: self.tags.len(),
        };
        f(&Inputs {
            frame: self.frames.segments[index],
            state,
            value: &segment,
            drag: None,
            press_point: self
                .press_point
                .get()
                .filter(|_| state.contains(InteractionState::PRESSED)),
        })
    }

    fn with_group_inputs<R>(&self, f: impl FnOnce(&Inputs<'_, ()>) -> R) -> R {
        f(&Inputs {
            frame: self.frames.row,
            state: self.control_state(),
            value: &(),
            drag: self.drag.get(),
            press_point: self.press_point.get(),
        })
    }

    /// Moves the control parts and the group opacity to the picker's
    /// current inputs under `change`.
    fn retarget_control(&self, change: Change<'_>) {
        self.with_control_inputs(|inputs| {
            for driven in self.control_drivens() {
                driven.retarget(inputs, change);
            }
        });
        self.with_group_inputs(|inputs| self.group_opacity.retarget(inputs, change));
    }

    /// Moves segment `index`'s parts and label colour to its current inputs
    /// under `change`, and its chrome to its current state under the
    /// chrome's own motion.
    fn retarget_segment(&self, index: usize, change: Change<'_>) {
        self.with_segment_inputs(index, |inputs| {
            for driven in self.segment_drivens(index) {
                driven.retarget(inputs, change);
            }
        });
        if let Some(chrome) = self.chromes.get(index) {
            chrome.sync(self.segment_state(index));
        }
    }

    /// Sets segment `index`'s own interaction flags to `state`'s hover,
    /// press and focus, retargeting under the look's motion.
    fn set_states(&self, index: usize, state: InteractionState) {
        let state = state & LIVE;
        if self.states[index].get() == state {
            return;
        }
        let motion = self.look.motion();
        let old_segment = self.segment_state(index);
        let old_control = self.control_state();
        self.states[index].set(state);
        if !self
            .states
            .iter()
            .any(|state| state.get().contains(InteractionState::PRESSED))
        {
            self.press_point.set(None);
        }
        self.retarget_segment(
            index,
            Change::Interaction {
                old: old_segment,
                motion,
            },
        );
        if index == self.value.get().selected {
            self.retarget_control(Change::Interaction {
                old: old_control,
                motion,
            });
        }
    }

    /// Moves segment `index`'s operands to the interaction state its
    /// handles reported for this render; a change of the disabled state is
    /// a new realization.
    pub(crate) fn sync(&self, index: usize, state: InteractionState) {
        self.set_states(index, state);
    }

    /// Sets or clears one live interaction flag (hover, press or focus) of
    /// segment `index`.
    pub(crate) fn set_flag(&self, index: usize, flag: InteractionState, on: bool) {
        if self.disabled {
            return;
        }
        let mut state = self.states[index].get();
        state.set(flag, on);
        self.set_states(index, state);
    }

    /// Starts a press of segment `index` at `origin`, in the widget's
    /// record space (`None` for a keyboard or synthetic press): the press
    /// point the parts read is `origin` in their mirrored space, and the
    /// segment's chrome starts ripple wave `wave` there.
    pub(crate) fn begin_press(&self, index: usize, wave: usize, origin: Option<Point>) {
        if self.disabled {
            return;
        }
        let origin = origin.map(|origin| self.frames.mirror() * origin);
        self.press_point.set(origin);
        if let Some(chrome) = self.chromes.get(index) {
            chrome.begin_wave(wave, origin);
        }
        self.set_flag(index, InteractionState::PRESSED, true);
    }

    /// Moves segment `index`'s active press point to `point`, in the
    /// widget's record space: its chrome's values that read it follow the
    /// pointer.
    pub(crate) fn move_press(&self, index: usize, point: Point) {
        if let Some(chrome) = self.chromes.get(index) {
            chrome.move_press(self.frames.mirror() * point);
        }
    }

    /// Fades ripple wave `wave` of segment `index`'s chrome out.
    pub(crate) fn fade_wave(&self, index: usize, wave: usize) {
        if let Some(chrome) = self.chromes.get(index) {
            chrome.fade_wave(wave);
        }
    }

    /// Drops every wave of segment `index`'s chrome without animating.
    pub(crate) fn clear_waves(&self, index: usize) {
        if let Some(chrome) = self.chromes.get(index) {
            chrome.clear_waves();
        }
    }

    /// The selection became segment `index` under `change`: the thumb's
    /// operands move under the look's value motion, the segments' selected
    /// flags snap.
    pub(crate) fn change_selection(&self, index: usize, change: &ChangeAnimation) {
        let previous = self.value.get().selected;
        let target = self.position_for(index);
        if previous == index && self.value.get().position.to_bits() == target.to_bits() {
            return;
        }
        let motion = self.look.motion();
        let old_previous = self.segment_state(previous);
        let old_next = self.segment_state(index);
        self.value.set(SegmentedValue {
            selected: index,
            position: target,
        });
        self.retarget_control(Change::Value { change, motion });
        self.retarget_segment(
            previous,
            Change::Interaction {
                old: old_previous,
                motion,
            },
        );
        if index != previous {
            self.retarget_segment(
                index,
                Change::Interaction {
                    old: old_next,
                    motion,
                },
            );
        }
    }

    /// The thumb's position with `selected` selected under the current
    /// drag.
    fn position_for(&self, selected: usize) -> f64 {
        self.look.get_drag().map_or_else(
            || position(selected),
            |drag| {
                drag.position(
                    selected,
                    self.drag.get(),
                    self.segment_width(),
                    self.tags.len(),
                )
            },
        )
    }

    fn segment_width(&self) -> f64 {
        self.frames.segments.first().map_or(0.0, Rect::width)
    }

    /// The index of the selected segment.
    pub(crate) const fn selected(&self) -> usize {
        self.value.get().selected
    }

    /// Starts a drag of the thumb: the control reads as dragged.
    pub(crate) fn begin_drag(&self) {
        if self.dragged.get() {
            return;
        }
        let old = self.control_state();
        self.dragged.set(true);
        self.retarget_control(Change::Interaction {
            old,
            motion: self.look.motion(),
        });
    }

    /// Moves the drag to `offset` from where the press landed. The thumb
    /// tracks it with no animation, so it follows the finger.
    pub(crate) fn drag_to(&self, offset: Vec2) {
        self.drag.set(Some(offset));
        let mut value = self.value.get();
        value.position = self.position_for(value.selected);
        self.value.set(value);
        self.retarget_control(Change::Animated(None));
    }

    /// Ends the drag with the thumb settling on the nearest segment: its
    /// tag is written to `selection` when it is not the selection already,
    /// and the thumb settles there under the look's value motion from the
    /// position and velocity the drag left it at.
    pub(crate) fn end_drag(&self, selection: &Binding<Id>) {
        let Some(drag) = self.look.get_drag() else {
            return;
        };
        let settled = drag.settles_on(self.value.get().position, self.tags.len());
        let tag = self.tags[settled];
        if selection.snapshot() == tag {
            self.cancel_drag();
        } else {
            let old = self.control_state();
            self.drag.set(None);
            self.dragged.set(false);
            // The selection's change reaches the operands through the
            // subscription, with the drag already gone.
            selection.set(tag);
            // The thumb no longer reads as dragged.
            self.retarget_control(Change::Interaction {
                old,
                motion: self.look.motion(),
            });
        }
    }

    /// Ends the drag with the thumb returning to the selected segment under
    /// the look's value motion, from where the drag left it.
    pub(crate) fn cancel_drag(&self) {
        let old = self.control_state();
        self.drag.set(None);
        self.dragged.set(false);
        let mut value = self.value.get();
        value.position = position(value.selected);
        self.value.set(value);
        self.retarget_control(Change::Value {
            change: &ChangeAnimation::Unspecified,
            motion: self.look.motion(),
        });
        // The thumb no longer reads as dragged.
        self.retarget_control(Change::Interaction {
            old,
            motion: self.look.motion(),
        });
    }

    /// Whether the thumb follows a drag.
    pub(crate) const fn drags(&self) -> bool {
        self.look.get_drag().is_some()
    }

    /// The group opacity operand, when the look's group opacity varies
    /// between renders: with a signal, or with the thumb's hover, press or
    /// focus. A fixed opacity is [`Self::fixed_group_opacity`].
    pub(crate) fn live_group_opacity(&self) -> Option<Operand<f32>> {
        varies_live(
            self.look.get_group_opacity(),
            self.frames.row,
            self.base(),
            &[()],
        )
        .then(|| self.group_opacity.operand().clone())
    }

    /// The group opacity of the picker at rest.
    pub(crate) fn fixed_group_opacity(&self) -> f32 {
        self.look
            .get_group_opacity()
            .target(&Inputs::at_rest(self.frames.row, self.base(), &()))
    }

    /// The area the parts draw in: the row and the look's reach, and where
    /// the segments' chromes draw, mirrored with them.
    pub(crate) fn visual_bounds(&self) -> Rect {
        let reach = self.look.layout().reach;
        let mirror = self.frames.mirror();
        self.chromes
            .iter()
            .map(|chrome| mirror.transform_rect_bbox(chrome.visual_bounds()))
            .fold(self.frames.row.inflate(reach, reach), |bounds, chrome| {
                bounds.union(chrome)
            })
    }

    /// The paint segment `index`'s label draws in, following its colour.
    pub(crate) fn label_paint(&self, index: usize) -> Computed<Paint> {
        self.label_paints[index].clone()
    }

    /// The interaction motion a segment's interaction handles run with,
    /// the segment wearing `chrome`: the engine animates the parts and the
    /// chrome, so the handles' scalars snap and only the look's press delay
    /// and the chrome's ripple hold and fade-out matter.
    pub(crate) fn handle_motion(
        look: &SegmentedLook,
        chrome: Option<&Chrome>,
    ) -> InteractionMotion {
        let control = LiveChrome::handle_motion(look.chrome());
        match chrome {
            Some(chrome) => InteractionMotion {
                touch_delay: control.touch_delay,
                ..LiveChrome::handle_motion(chrome)
            },
            None => control,
        }
    }

    /// Whether this realizes `frames` for `tags` in the disabled state
    /// `disabled`.
    fn realizes(&self, frames: &SegmentedFrames, tags: &[Id], disabled: bool) -> bool {
        self.frames == *frames && self.tags == tags && self.disabled == disabled
    }

    fn record_below(&self, recorder: &mut Recorder) {
        #[cfg(test)]
        self.recordings.set(self.recordings.get() + 1);
        for part in &self.below {
            part.record(recorder, self.frames.row);
        }
        for (index, (parts, frame)) in self.segments.iter().zip(&self.frames.segments).enumerate() {
            if let Some(chrome) = self.chromes.get(index) {
                chrome.record_layers(recorder);
            }
            for part in parts {
                part.record(recorder, *frame);
            }
        }
    }

    fn record_above(&self, recorder: &mut Recorder) {
        #[cfg(test)]
        self.recordings.set(self.recordings.get() + 1);
        for part in &self.above {
            part.record(recorder, self.frames.row);
        }
    }

    /// How many times the mount recorded the parts.
    #[cfg(test)]
    pub(crate) const fn recordings(&self) -> u32 {
        self.recordings.get()
    }

    /// The picker's value the operands show.
    #[cfg(test)]
    pub(crate) const fn value(&self) -> SegmentedValue {
        self.value.get()
    }

    /// The thumb's state the operands show.
    #[cfg(test)]
    pub(crate) fn thumb_state(&self) -> InteractionState {
        self.control_state()
    }
}

/// A count of segments as a position along the row.
const fn position(index: usize) -> f64 {
    crate::num_cast::usize_as_f64(index)
}

/// The recordings a segmented picker's program lists.
#[derive(Default)]
struct Layers {
    below: Option<LayeredParts>,
    above: Option<LayeredParts>,
}

/// A segmented picker's live parts and the recordings its program lists:
/// the identity the mount keeps them by.
#[derive(Clone)]
pub struct LiveSegmentedParts {
    picker: Rc<LiveSegmented>,
    layers: Rc<RefCell<Layers>>,
}

impl LiveSegmentedParts {
    /// The look `look` on `frames` for the items `tags`, each segment
    /// wearing its chrome of `chromes`, following `selection`: the parts in
    /// `slot` when they already realize it, otherwise fresh parts at rest,
    /// replacing them. `slot` is emptied by the caller whenever the look is
    /// requested again, and `chromes` holds with the look.
    #[expect(
        clippy::too_many_arguments,
        reason = "the realization's independent inputs: the look and its segments' chromes, its frames, the items, the selection, the disabled state and the environment the colours resolve in"
    )]
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        look: &SegmentedLook,
        chromes: &[Chrome],
        frames: SegmentedFrames,
        tags: &[Id],
        selected: usize,
        disabled: bool,
        selection: &Binding<Id>,
        env: &Environment,
    ) -> &'a Self {
        if !slot
            .as_ref()
            .is_some_and(|parts| parts.picker.realizes(&frames, tags, disabled))
        {
            let picker = Rc::new(LiveSegmented::new(
                look.clone(),
                frames,
                tags.to_vec(),
                selected,
                disabled,
                chromes,
                env,
            ));
            LiveSegmented::subscribe(&picker, selection);
            *slot = Some(Self {
                picker,
                layers: Rc::default(),
            });
        }
        slot.as_ref()
            .expect("the segmented parts were just ensured")
    }

    pub(crate) const fn picker(&self) -> &Rc<LiveSegmented> {
        &self.picker
    }

    /// `ctx` with the parts' mirror about the row: the transform their
    /// recordings and their materials' members are listed at, not a scope
    /// inside their recording, which a backdrop material cannot record in.
    fn mirrored(&self, ctx: RenderContext) -> RenderContext {
        let mut mirrored = ctx;
        mirrored.set_local(ctx.local() * self.picker.frames.mirror());
        mirrored
    }

    /// Lists the parts below the labels in `ctx`: the control parts, then
    /// each segment's.
    pub(crate) fn push_below<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        if self.picker.below.is_empty()
            && self.picker.chromes.is_empty()
            && self.picker.segments.iter().all(Vec::is_empty)
        {
            return;
        }
        let scope = renderer.material_scope();
        let mut layers = self.layers.borrow_mut();
        let picker = Rc::clone(&self.picker);
        LayeredParts::ensure(&mut layers.below, scope, move || -> PartsBody {
            Rc::new(move |recorder: &mut Recorder| picker.record_below(recorder))
        })
        .push(renderer, self.mirrored(ctx));
    }

    /// Lists the control parts over the labels in `ctx`.
    pub(crate) fn push_above<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        if self.picker.above.is_empty() {
            return;
        }
        let scope = renderer.material_scope();
        let mut layers = self.layers.borrow_mut();
        let picker = Rc::clone(&self.picker);
        LayeredParts::ensure(&mut layers.above, scope, move || -> PartsBody {
            Rc::new(move |recorder: &mut Recorder| picker.record_above(recorder))
        })
        .push(renderer, self.mirrored(ctx));
    }
}
