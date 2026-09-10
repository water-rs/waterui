//! Interaction metadata: gesture recognition and hover routing.
//!
//! `Metadata<GestureObserver>` (`.gesture(...)`, `.on_tap(...)`) and
//! `Metadata<OnEvent>` (`.on_hover_*`) are the two wrappers a view uses to ask
//! for pointer semantics richer than "this rectangle is a control", and they
//! are what makes an interactive chart interactive. Dew answers both here.
//!
//! Recognition itself is not dew's to invent: the tap / long-press / drag /
//! pinch / rotate state machines live in `waterui-backend-core`, shared with
//! the GPU renderer, so a double tap means the same thing on a panel as it
//! does on a desktop window. Dew supplies the two halves the shared engine
//! cannot know about — where each target currently sits, and when a frame is
//! worth spending.
//!
//! Registration follows [`crate::pointer::PointerRouter`]: every frame clears
//! the target list and the retained tree re-registers as it renders, so a
//! target's hit rectangle is always the rectangle layout just placed it at.
//! The *state* those registrations drive is retained on the node instead — an
//! in-flight drag's recognizer, a hover target's inside/outside flag — so a
//! frame never restarts an interaction that is still in progress.
//!
//! Dirty regions are not this module's business. A handler writes signals; the
//! signals request a refresh; the refreshed display list is diffed against the
//! previous one and only the commands that actually changed are re-rasterized
//! (see [`crate::runtime`]). A hover that moves the pointer within a chart
//! without changing its focused point therefore costs nothing at all, which is
//! why a hover *move* over an already-hovered target deliberately does not
//! force a frame of its own.

use core::cell::{Cell, RefCell};
use std::rc::Rc;

use kurbo::{Point, Rect};
use waterui_backend_core::gesture::{GestureEngine, GestureTarget};
use waterui_backend_core::input::TouchPhase;
use waterui_backend_core::time::Instant;
use waterui_core::event::{Event, HoverEvent, OnEvent};
use waterui_core::gesture::{Gesture, GestureObserver};
use waterui_core::handler::BoxedAction;
use waterui_core::layout::{ProposalSize, StretchAxis, ViewDimensions};
use waterui_core::{AnyView, Environment};

use crate::board::PointerSample;
use crate::dispatch::{DewNode, DewRenderer, RenderContext, build_node};
use crate::text::DewState;
use crate::views::to_f32;

/// Dew registers interaction targets in draw order, and the shared engine
/// breaks priority ties by registration index — which is exactly the
/// painter's-algorithm rule [`crate::pointer::PointerRouter`] applies when it
/// searches its targets in reverse. Nesting depth, sibling order, and
/// hit-test group carry no information dew does not already have from that
/// order, so they stay uniform rather than encoding a second, redundant
/// z-order that could disagree with the one actually drawn.
const HIT_DEPTH: usize = 0;
const HIT_ORDER: usize = 0;
const HIT_GROUP: usize = 0;

/// Dew's interaction routing: the shared gesture engine plus hover targets.
#[derive(Default)]
pub struct InteractionRouter {
    gestures: GestureEngine,
    hovers: Vec<HoverTarget>,
    /// The last position the board's pointer device reported, kept so
    /// recognizers whose registration moved under them can be re-hit-tested
    /// after layout. `None` once the sequence was cancelled — a cancelled
    /// pointer has no position.
    pointer: Option<Point>,
}

impl core::fmt::Debug for InteractionRouter {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionRouter")
            .field("gesture_targets", &self.gestures.target_count())
            .field("hover_targets", &self.hovers.len())
            .field("pointer", &self.pointer)
            .finish()
    }
}

impl InteractionRouter {
    pub fn begin_frame(&mut self) {
        self.gestures.clear_targets();
        self.hovers.clear();
    }

    pub fn finish_frame(&mut self) {
        self.gestures.sync_after_layout(self.pointer);
    }

    pub fn register_gesture(
        &mut self,
        bounds: Rect,
        gesture: Gesture,
        action: BoxedAction<()>,
    ) -> GestureTarget {
        self.gestures
            .register_target(bounds, gesture, action, HIT_DEPTH, HIT_ORDER, HIT_GROUP)
    }

    pub fn register_existing_gesture(&mut self, target: GestureTarget) {
        self.gestures.register_existing_target(target);
    }

    pub fn register_hover(&mut self, bounds: Rect, state: Rc<HoverState>) {
        self.hovers.push(HoverTarget { bounds, state });
    }

    /// Routes one board pointer sample to the hover targets and the gesture
    /// recognizers, returning whether the frame must be refreshed for a reason
    /// the reactive graph cannot see for itself.
    pub fn dispatch(&mut self, sample: PointerSample, now: Instant, env: &Environment) -> bool {
        let point = Point::new(sample.x, sample.y);
        match sample.phase {
            // A press is also a position report: a touch panel's first sample
            // of a sequence is the only "the pointer is here" it ever sends,
            // so hover enter has to be resolved from it too.
            TouchPhase::Started => {
                self.pointer = Some(point);
                let hovered = self.sync_hover(point, env);
                hovered | self.gestures.handle_pointer_down(point, now, env)
            }
            // Moves arrive whether or not a button is held: an unpressed move
            // is a hover, a pressed one additionally drives the drag
            // recognizers, and the engine ignores a move that no recognizer is
            // active for.
            TouchPhase::Moved => {
                self.pointer = Some(point);
                let hovered = self.sync_hover(point, env);
                hovered | self.gestures.handle_pointer_move(point, now, env)
            }
            TouchPhase::Ended => {
                self.pointer = Some(point);
                self.gestures.handle_pointer_up(point, now, env)
            }
            // The pointer is gone rather than elsewhere, so every hovered
            // target exits and every in-flight recognizer fails.
            TouchPhase::Cancelled => {
                self.pointer = None;
                let exited = self.exit_hovers(env);
                exited | self.gestures.handle_pointer_cancel(now, env)
            }
        }
    }

    /// Advances time-driven recognition (a long press firing once its hold
    /// deadline passes). Only meaningful while a sequence is in flight, which
    /// is what keeps this off the cost of an idle frame.
    pub fn tick(&mut self, now: Instant, env: &Environment) -> bool {
        self.gestures.has_active_recognizer() && self.gestures.handle_tick(now, env)
    }

    /// Hover is not exclusive. Every registered target compares the pointer
    /// against its own rectangle, because hover handlers stack: an interactive
    /// chart wraps its canvas in a move handler *and* an exit handler, and a
    /// topmost-only search would deliver to one of them.
    fn sync_hover(&self, point: Point, env: &Environment) -> bool {
        let mut changed = false;
        for target in &self.hovers {
            changed |= target.state.pointer_at(Some(point), target.bounds, env);
        }
        changed
    }

    fn exit_hovers(&self, env: &Environment) -> bool {
        let mut changed = false;
        for target in &self.hovers {
            changed |= target.state.pointer_at(None, target.bounds, env);
        }
        changed
    }
}

/// One frame's registration of a hover handler: where it currently sits, and
/// the retained state it drives.
struct HoverTarget {
    bounds: Rect,
    state: Rc<HoverState>,
}

/// The retained half of an [`OnEvent`] handler.
///
/// `inside` is what turns a stream of positions into enter/exit edges, and it
/// belongs to the node rather than to a frame: the pointer does not re-enter a
/// chart because the chart re-rendered.
pub struct HoverState {
    event: Event,
    handler: RefCell<OnEvent>,
    /// The environment the handler was built in — the one its extractors have
    /// to resolve against, layered over whatever environment the pointer
    /// arrives in.
    env: Environment,
    inside: Cell<bool>,
}

impl HoverState {
    /// Applies the pointer's current position (or its absence) to this target,
    /// returning whether the frame must be refreshed for a reason the reactive
    /// graph cannot see.
    fn pointer_at(&self, point: Option<Point>, bounds: Rect, env: &Environment) -> bool {
        let inside = point.is_some_and(|point| bounds.contains(point));
        let crossed = self.inside.replace(inside) != inside;
        match self.event {
            Event::HoverEnter if crossed && inside => {
                self.handler.borrow_mut().handle(&self.env.layered_on(env));
                true
            }
            Event::HoverExit if crossed && !inside => {
                self.handler.borrow_mut().handle(&self.env.layered_on(env));
                true
            }
            Event::HoverMove if inside => {
                let point = point.expect("a hover move inside a target has a position");
                let hover = HoverEvent::new(waterui_core::layout::Point::new(
                    to_f32(point.x - bounds.x0),
                    to_f32(point.y - bounds.y0),
                ));
                let hover_env = self.env.layered_on(&env.extending(hover));
                self.handler.borrow_mut().handle(&hover_env);
                // Deliberately not a refresh request. Moves inside an already
                // hovered target are the high-rate case, and a handler that
                // changed something wrote it to a signal, which schedules the
                // frame — and the display-list diff then dirties only the part
                // that moved. Forcing a frame here would relayout the tree at
                // pointer rate for a chart that decided nothing changed.
                false
            }
            _ => false,
        }
    }
}

/// The retained node behind `Metadata<GestureObserver>`.
struct GestureNode {
    gesture: Gesture,
    /// Handed to the recognizer the first time this node renders; from then on
    /// the recognizer lives in `target` and is re-registered, which is what
    /// keeps a drag alive across the frames it spans.
    action: Option<BoxedAction<()>>,
    target: Option<GestureTarget>,
    child: Box<dyn DewNode>,
}

impl DewNode for GestureNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        self.child.measure(state, proposal)
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let bounds = ctx.window_bounds();
        let target = if let Some(target) = self.target.take() {
            let target = target.with_bounds_depth_and_group(bounds, HIT_DEPTH, HIT_GROUP);
            renderer.register_existing_gesture_target(target.clone());
            target
        } else {
            let action = self
                .action
                .take()
                .expect("dew gesture action is bound exactly once, on the node's first render");
            renderer.register_gesture_target(bounds, self.gesture.clone(), action)
        };
        self.target = Some(target);
        self.child.render(renderer, ctx);
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.child.stretch_axis()
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.child.patch(renderer)
    }
}

/// The retained node behind `Metadata<OnEvent>`.
struct HoverNode {
    state: Rc<HoverState>,
    child: Box<dyn DewNode>,
}

impl DewNode for HoverNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        self.child.measure(state, proposal)
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        renderer.register_hover_target(ctx.window_bounds(), Rc::clone(&self.state));
        self.child.render(renderer, ctx);
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.child.stretch_axis()
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.child.patch(renderer)
    }
}

/// Builds the retained node for a gesture-observed subtree.
pub fn build_gesture(
    renderer: &mut DewRenderer,
    content: AnyView,
    observer: GestureObserver,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    let GestureObserver {
        gesture, action, ..
    } = observer;
    Box::new(GestureNode {
        gesture,
        action: Some(layered_action(action, env)),
        target: None,
        child: build_node(renderer, content, env, depth),
    })
}

/// Builds the retained node for a hover-observed subtree.
pub fn build_hover(
    renderer: &mut DewRenderer,
    content: AnyView,
    handler: OnEvent,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    let event = handler.event();
    assert!(
        matches!(
            event,
            Event::HoverEnter | Event::HoverMove | Event::HoverExit
        ),
        "dew does not implement Event::{event:?}"
    );
    Box::new(HoverNode {
        state: Rc::new(HoverState {
            event,
            handler: RefCell::new(handler),
            env: env.clone(),
            inside: Cell::new(false),
        }),
        child: build_node(renderer, content, env, depth),
    })
}

/// Rebinds `action` to the environment its view was built in.
///
/// The gesture engine calls an action with the environment the *pointer*
/// arrived in, carrying the recognized event payload; the handler's extractors
/// need the scoped environment its view was built in. Layering the captured
/// environment over the runtime one gives both.
fn layered_action(mut action: BoxedAction<()>, env: &Environment) -> BoxedAction<()> {
    let captured = env.clone();
    Box::new(move |runtime: &Environment| action(&captured.layered_on(runtime)))
}
