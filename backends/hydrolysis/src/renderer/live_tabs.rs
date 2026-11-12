//! Hydrolysis's realization of a tab container's [`TabsLook`]: its bar,
//! divider, item chromes and highlights recorded once, with every
//! [`Driven`](crate::theme::Driven) value a live operand.
//!
//! A [`LiveTabs`] keeps the bar's inputs — the selected item, each item's
//! interaction state and the press point — and retargets the parts'
//! operands when one changes, each under the motion its change runs with:
//! the look's [`Motion`](crate::theme::Motion) for an interaction change,
//! its value motion for a selection change (read through the change's
//! [`ChangeAnimation`]). The engine animates the recorded commands in
//! place, so a selection change, a hover or a press records nothing and
//! marks no node.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use cherenkov_record::Recorder;
use kurbo::{Point, Rect};
use nami::{Binding, Computed, Signal, SignalExt as _};
use waterui_core::Environment;
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::id::Id;
use waterui_core::interaction::InteractionState;

use crate::renderer::chrome::{LayeredParts, LiveChrome, PartsBody};
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::theme::{Change, Chrome, Inputs, RealizedCustom, TabItem, TabItemGeometry, TabsLook};

/// The interaction flags an item's own pointer and keyboard state set.
const LIVE: InteractionState = InteractionState::HOVERED
    .union(InteractionState::PRESSED)
    .union(InteractionState::FOCUSED);

/// Where a tab container's parts lie, in the widget's record space.
#[derive(Debug, Clone, PartialEq)]
pub struct TabsFrames {
    /// The bar's surface, extended through the safe-area regions of every
    /// edge it touches.
    pub surface: Rect,
    /// The bar's inner edge.
    pub divider: Rect,
    /// Each item's geometry, leading first.
    pub items: Vec<TabItemGeometry>,
}

/// One tab container's parts, recorded once with live operands.
///
/// The container keeps it across renders while its look, its frames and its
/// items stay the same; the interaction handles bound to each item steer it
/// between renders through [`Self::set_flag`] and [`Self::begin_press`], and
/// the selection steers it through its subscription.
pub struct LiveTabs {
    look: TabsLook,
    frames: TabsFrames,
    /// Each item's tag, leading first.
    tags: Vec<Id>,
    /// The index of the selected item; `None` while the selection names no
    /// item.
    selected: Cell<Option<usize>>,
    press_point: Cell<Option<Point>>,
    /// Each item's own hover, press and focus.
    states: Vec<Cell<InteractionState>>,
    bar: RealizedCustom<()>,
    divider: RealizedCustom<()>,
    /// Each item's highlight, leading item first; `None` where the look
    /// draws none.
    highlights: Vec<Option<RealizedCustom<TabItem>>>,
    /// Each item's chrome as the look gave it, leading item first.
    specs: Vec<Chrome>,
    chromes: Vec<Rc<LiveChrome>>,
    /// The subscriptions to the selection and to the signals the highlights'
    /// values follow.
    guards: RefCell<Vec<Box<dyn Any>>>,
    /// How many times the mount recorded the parts.
    #[cfg(test)]
    recordings: Cell<u32>,
}

impl std::fmt::Debug for LiveTabs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveTabs")
            .field("frames", &self.frames)
            .field("selected", &self.selected.get())
            .finish_non_exhaustive()
    }
}

/// What a signal one of the highlights' values follow does to the container
/// when it changes, under the animation the change carries.
type Retargeting = dyn Fn(&LiveTabs, Option<&Animation>);

impl LiveTabs {
    /// The look realized on `frames` for the items `tags`, at rest with the
    /// item at `selected` selected, each item wearing the chrome the look
    /// gives it for its flag of `flags`, resolving the colours in `env`.
    fn new(
        look: TabsLook,
        frames: TabsFrames,
        tags: Vec<Id>,
        selected: Option<usize>,
        flags: &[Computed<bool>],
        env: &Environment,
    ) -> Self {
        let count = tags.len();
        let item = |index: usize| TabItem {
            index,
            count,
            selected: selected == Some(index),
            geometry: frames.items[index],
        };
        let state = |index: usize| {
            if selected == Some(index) {
                InteractionState::SELECTED
            } else {
                InteractionState::empty()
            }
        };
        let bar = look.bar().realize(&Inputs::at_rest(
            frames.surface,
            InteractionState::empty(),
            &(),
        ));
        let divider = look.divider().realize(&Inputs::at_rest(
            frames.divider,
            InteractionState::empty(),
            &(),
        ));
        let highlights = (0..count)
            .map(|index| {
                let item = item(index);
                look.item_highlight(&item).map(|part| {
                    part.realize(&Inputs::at_rest(item.geometry.bounds, state(index), &item))
                })
            })
            .collect();
        let specs: Vec<Chrome> = (0..count)
            .map(|index| look.item_chrome(&item(index), flags[index].clone()))
            .collect();
        let chromes = specs
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                LiveChrome::realize(spec.clone(), frames.items[index].bounds, state(index), env)
            })
            .collect();
        Self {
            states: (0..count)
                .map(|_| Cell::new(InteractionState::empty()))
                .collect(),
            look,
            frames,
            tags,
            selected: Cell::new(selected),
            press_point: Cell::new(None),
            bar,
            divider,
            highlights,
            specs,
            chromes,
            guards: RefCell::new(Vec::new()),
            #[cfg(test)]
            recordings: Cell::new(0),
        }
    }

    /// Subscribes the container to `selection` and to every signal the
    /// highlights' values follow, so a change retargets the operands under
    /// the animation the change carries.
    fn subscribe(this: &Rc<Self>, selection: &Binding<Id>) {
        let mut guards: Vec<Box<dyn Any>> = Vec::new();
        let weak: Weak<Self> = Rc::downgrade(this);
        guards.push(Box::new(selection.watch(move |context| {
            if let Some(tabs) = weak.upgrade() {
                // A tag the realization does not know belongs to a
                // membership change the next render realizes afresh.
                if let Some(index) = tabs.tags.iter().position(|tag| tag == context.value()) {
                    tabs.change_selection(index, &ChangeAnimation::of(context.metadata()));
                }
            }
        })));
        for index in 0..this.tags.len() {
            let Some(highlight) = &this.highlights[index] else {
                continue;
            };
            for driven in highlight.drivens.iter().cloned() {
                let weak_driven = Rc::downgrade(&driven);
                let target: Rc<Retargeting> = Rc::new(move |tabs, animation| {
                    if let Some(driven) = weak_driven.upgrade() {
                        tabs.with_item_inputs(index, |inputs| {
                            driven.retarget(inputs, Change::Animated(animation));
                        });
                    }
                });
                let weak: Weak<Self> = Rc::downgrade(this);
                let notify = Rc::new(move |animation: Option<Animation>| {
                    if let Some(tabs) = weak.upgrade() {
                        target(&tabs, animation.as_ref());
                    }
                }) as Rc<dyn Fn(Option<Animation>)>;
                guards.extend(driven.subscribe(notify));
            }
        }
        *this.guards.borrow_mut() = guards;
    }

    /// Item `index`'s state: its own hover, press and focus, and selected
    /// while it is the selected item.
    fn item_state(&self, index: usize) -> InteractionState {
        let mut state = self.states[index].get() & LIVE;
        state.set(
            InteractionState::SELECTED,
            self.selected.get() == Some(index),
        );
        state
    }

    /// Item `index` as the look's parts see it.
    fn item(&self, index: usize) -> TabItem {
        TabItem {
            index,
            count: self.tags.len(),
            selected: self.selected.get() == Some(index),
            geometry: self.frames.items[index],
        }
    }

    fn with_item_inputs<R>(&self, index: usize, f: impl FnOnce(&Inputs<'_, TabItem>) -> R) -> R {
        let state = self.item_state(index);
        let item = self.item(index);
        f(&Inputs {
            frame: item.geometry.bounds,
            state,
            value: &item,
            drag: None,
            press_point: self
                .press_point
                .get()
                .filter(|_| state.contains(InteractionState::PRESSED)),
        })
    }

    /// Moves item `index`'s highlight to its current inputs under `change`,
    /// and its chrome to its current state under the chrome's own motion.
    fn retarget_item(&self, index: usize, change: Change<'_>) {
        if let Some(highlight) = &self.highlights[index] {
            self.with_item_inputs(index, |inputs| {
                for driven in &highlight.drivens {
                    driven.retarget(inputs, change);
                }
            });
        }
        self.chromes[index].sync(self.item_state(index));
    }

    /// Sets item `index`'s own interaction flags to `state`'s hover, press
    /// and focus, retargeting under the look's motion.
    fn set_states(&self, index: usize, state: InteractionState) {
        let state = state & LIVE;
        if self.states[index].get() == state {
            return;
        }
        let old = self.item_state(index);
        self.states[index].set(state);
        if !self
            .states
            .iter()
            .any(|state| state.get().contains(InteractionState::PRESSED))
        {
            self.press_point.set(None);
        }
        self.retarget_item(
            index,
            Change::Interaction {
                old,
                motion: self.look.motion(),
            },
        );
    }

    /// Moves item `index`'s operands to the interaction state its handles
    /// reported for this render.
    pub(crate) fn sync(&self, index: usize, state: InteractionState) {
        self.set_states(index, state);
    }

    /// Sets or clears one live interaction flag (hover, press or focus) of
    /// item `index`.
    pub(crate) fn set_flag(&self, index: usize, flag: InteractionState, on: bool) {
        let mut state = self.states[index].get();
        state.set(flag, on);
        self.set_states(index, state);
    }

    /// Starts a press of item `index` at `origin`, in the widget's record
    /// space (`None` for a keyboard or synthetic press): the press point the
    /// highlight reads is `origin`, and the item's chrome starts ripple wave
    /// `wave` there.
    pub(crate) fn begin_press(&self, index: usize, wave: usize, origin: Option<Point>) {
        self.press_point.set(origin);
        self.chromes[index].begin_wave(wave, origin);
        self.set_flag(index, InteractionState::PRESSED, true);
    }

    /// Moves item `index`'s active press point to `point`: its chrome's
    /// values that read it follow the pointer.
    pub(crate) fn move_press(&self, index: usize, point: Point) {
        self.chromes[index].move_press(point);
    }

    /// Fades ripple wave `wave` of item `index`'s chrome out.
    pub(crate) fn fade_wave(&self, index: usize, wave: usize) {
        self.chromes[index].fade_wave(wave);
    }

    /// Drops every wave of item `index`'s chrome without animating.
    pub(crate) fn clear_waves(&self, index: usize) {
        self.chromes[index].clear_waves();
    }

    /// The selection became item `index` under `change`: the two items it
    /// touches move their highlights under the look's value motion and their
    /// chromes to their new selected state.
    pub(crate) fn change_selection(&self, index: usize, change: &ChangeAnimation) {
        let previous = self.selected.replace(Some(index));
        if previous == Some(index) {
            return;
        }
        let motion = self.look.motion();
        let change = Change::Value { change, motion };
        if let Some(previous) = previous {
            self.retarget_item(previous, change);
        }
        self.retarget_item(index, change);
    }

    /// The chrome item `index` wears, as the look gave it.
    pub(crate) fn chrome(&self, index: usize) -> &Chrome {
        &self.specs[index]
    }

    /// Whether this realizes `frames` for `tags`.
    fn realizes(&self, frames: &TabsFrames, tags: &[Id]) -> bool {
        self.frames == *frames && self.tags == tags
    }

    fn record(&self, recorder: &mut Recorder) {
        #[cfg(test)]
        self.recordings.set(self.recordings.get() + 1);
        self.bar.record(recorder, self.frames.surface);
        self.divider.record(recorder, self.frames.divider);
        for (index, geometry) in self.frames.items.iter().enumerate() {
            if let Some(highlight) = &self.highlights[index] {
                highlight.record(recorder, geometry.bounds);
            }
            self.chromes[index].record_layers(recorder);
        }
    }

    /// How many times the mount recorded the parts.
    #[cfg(test)]
    pub(crate) const fn recordings(&self) -> u32 {
        self.recordings.get()
    }

    /// The index of the selected item the operands show.
    #[cfg(test)]
    pub(crate) const fn selected(&self) -> Option<usize> {
        self.selected.get()
    }
}

/// A tab container's live parts and the recording its program lists: the
/// identity the mount keeps them by.
#[derive(Clone)]
pub struct LiveTabsParts {
    tabs: Rc<LiveTabs>,
    layers: Rc<RefCell<Option<LayeredParts>>>,
}

impl LiveTabsParts {
    /// The look `look` on `frames` for the items `tags`, following
    /// `selection`: the parts in `slot` when they already realize them,
    /// otherwise fresh parts at rest with the item at `selected` selected,
    /// replacing them. `slot` is emptied by the caller whenever the look is
    /// requested again.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        look: &TabsLook,
        frames: TabsFrames,
        tags: &[Id],
        selected: Option<usize>,
        selection: &Binding<Id>,
        env: &Environment,
    ) -> &'a Self {
        if !slot
            .as_ref()
            .is_some_and(|parts| parts.tabs.realizes(&frames, tags))
        {
            let flags: Vec<Computed<bool>> = tags
                .iter()
                .map(|tag| {
                    let tag = *tag;
                    selection
                        .clone()
                        .map(move |current: Id| current == tag)
                        .distinct()
                        .computed()
                })
                .collect();
            let tabs = Rc::new(LiveTabs::new(
                look.clone(),
                frames,
                tags.to_vec(),
                selected,
                &flags,
                env,
            ));
            LiveTabs::subscribe(&tabs, selection);
            *slot = Some(Self {
                tabs,
                layers: Rc::default(),
            });
        }
        slot.as_ref().expect("the tabs parts were just ensured")
    }

    pub(crate) const fn tabs(&self) -> &Rc<LiveTabs> {
        &self.tabs
    }

    /// Lists the parts beneath the items' icons and labels in `ctx`: the
    /// bar, the divider, then each item's highlight and chrome.
    pub(crate) fn push<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        let scope = renderer.material_scope();
        let mut layers = self.layers.borrow_mut();
        let tabs = Rc::clone(&self.tabs);
        LayeredParts::ensure(&mut layers, scope, move || -> PartsBody {
            Rc::new(move |recorder: &mut Recorder| tabs.record(recorder))
        })
        .push(renderer, ctx);
    }
}
