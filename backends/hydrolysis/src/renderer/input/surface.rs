//! Generic embedded-surface input routing.
//!
//! An embedded surface is a rectangle of the window that draws its own
//! interactive content and therefore owns the input landing on it: a browser
//! engine, any [`GpuContentView`](waterui_graphics::GpuContentView) whose
//! view asks for input with
//! [`wants_input_events`](waterui_graphics::GpuContentView::wants_input_events),
//! or any [`SceneView`](waterui_graphics::SceneView) whose content asks the same
//! through
//! [`SceneContent::wants_input_events`](waterui_graphics::SceneContent::wants_input_events).
//!
//! There is one target list, one hit-test arbitration and one focus/capture
//! state machine for both, reached through [`EmbeddedInputSink`], and one
//! vocabulary at the far end of it: every sink translates into the
//! backend-neutral
//! [`SurfaceInputEvent`](waterui_graphics::input::SurfaceInputEvent). The
//! browser engines are ordinary GPU surfaces now — the CEF and WPE crates own
//! their own input ABIs — so the renderer knows nothing about any of them.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::gpu_view::GpuContentRuntime;
use waterui_graphics::SceneContent;
use waterui_graphics::input::{
    Code, Key, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};

/// One key transition, in the W3C UI Events vocabulary.
pub struct KeyDelivery<'a> {
    pub(crate) pressed: bool,
    pub(crate) logical: &'a Key,
    pub(crate) code: Code,
    pub(crate) repeat: bool,
    pub modifiers: Modifiers,
}

/// Backend-owned input sink for one embedded surface.
///
/// Positions are logical and surface-local: the surface's own top-left is
/// `(0, 0)`.
pub trait EmbeddedInputSink {
    /// A pointer that identifies the *owner* of this sink, stable across
    /// frames.
    ///
    /// Targets are re-emitted from scratch every frame, so the sink object a
    /// frame registers is not the one the previous frame registered. Focus and
    /// pointer capture are held across frames and must therefore compare
    /// owners, never sink allocations — comparing the latter retires focus on
    /// the very next frame and silently swallows every keystroke after the
    /// first.
    fn identity(&self) -> *const ();
    fn set_focus(&self, focused: bool);
    fn set_modifiers(&self, modifiers: Modifiers);
    fn pointer_move(&self, position: kurbo::Point);
    fn pointer_button(&self, pressed: bool, button: PointerButton, position: kurbo::Point);
    fn scroll(
        &self,
        position: kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        unit: ScrollUnit,
        finished: bool,
    );
    fn key(&self, delivery: &KeyDelivery<'_>);
    fn text_input(&self, text: &str);
    fn composition_start(&self);
    fn composition_update(&self, text: &str, caret: Option<usize>);
    fn composition_commit(&self, text: &str);
    fn composition_cancel(&self);
    /// The surface's own text caret, in logical surface-local coordinates, for
    /// placing the platform's input-method candidate window.
    fn ime_caret(&self) -> Option<kurbo::Rect>;
}

#[derive(Clone)]
pub struct EmbeddedInputTarget {
    /// The surface owner's interaction identity — what keyboard focus and a
    /// `.focused(binding)` write address the surface by.
    pub(crate) interaction_key: InteractionKey,
    pub(crate) local_bounds: kurbo::Rect,
    /// The paint clip enclosing the surface when it flushed, in window
    /// hit-test space — a surface straddling a scroll viewport only takes
    /// input where it is painted (water-rs/hydrolysis#252).
    pub(crate) hit_clip: Option<kurbo::Rect>,
    pub(crate) inverse_transform: kurbo::Affine,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) sink: Rc<dyn EmbeddedInputSink>,
    /// The `OnKeyPress` scopes enclosing the view this target was registered
    /// from, innermost first — the chain an unconsumed key bubbles through
    /// while this surface holds keyboard focus.
    pub(crate) key_handlers: Option<Rc<KeyHandlerNode>>,
    /// Written by `.focused(binding)` when it wraps this surface.
    pub(crate) focus_binding: Option<Binding<bool>>,
    /// The node the surface emits for the semantic tree. Keyboard traversal
    /// reaches the surface through it, and an assistive `Focus` request on it
    /// resolves back to the surface's interaction identity.
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_node_id: Option<AccessibilityNodeId>,
}

impl EmbeddedInputTarget {
    pub(crate) fn local_position(&self, point: kurbo::Point) -> Option<kurbo::Point> {
        if self.hit_clip.is_some_and(|clip| !clip.contains(point)) {
            return None;
        }
        self.local_bounds
            .contains(self.inverse_transform * point)
            .then(|| self.local_position_unclamped(point))
    }

    /// The surface-local position of a window point, whether or not it is
    /// inside the surface. Used while this target holds the pointer capture, a
    /// drag that has left the surface still being the surface's drag.
    pub(crate) fn local_position_unclamped(&self, point: kurbo::Point) -> kurbo::Point {
        let local = self.inverse_transform * point;
        kurbo::Point::new(
            local.x - self.local_bounds.x0,
            local.y - self.local_bounds.y0,
        )
    }

    /// Maps a surface-local rect back into window hit-test space.
    pub(crate) fn to_window_rect(&self, local: kurbo::Rect) -> kurbo::Rect {
        self.inverse_transform.inverse().transform_rect_bbox(
            local + kurbo::Vec2::new(self.local_bounds.x0, self.local_bounds.y0),
        )
    }
}

/// An embedded surface that took a bubbled key press — the matching release
/// belongs to it, not to whichever sink holds keyboard focus.
pub struct BubbledKeySink {
    pub(crate) logical: Key,
    pub(crate) code: Code,
    pub(crate) modifiers: Modifiers,
    pub(crate) sink: Rc<dyn EmbeddedInputSink>,
}

/// Something that consumes the neutral [`SurfaceInputEvent`] vocabulary: the
/// runtime of an embedded [`GpuContentView`](waterui_graphics::GpuContentView),
/// or the content of a self-drawn [`SceneView`](waterui_graphics::SceneView).
pub trait SurfaceInputReceiver {
    fn input(&mut self, event: &SurfaceInputEvent);
    /// The receiver's text caret, in logical surface-local coordinates.
    fn ime_caret(&self) -> Option<kurbo::Rect>;
}

impl SurfaceInputReceiver for GpuContentRuntime {
    fn input(&mut self, event: &SurfaceInputEvent) {
        self.view.input(event);
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.view.ime_caret()
    }
}

/// Scene content redraws through the invalidator it was handed at build time,
/// so delivering an event requests no frame here: content whose drawing the
/// event changed calls that invalidator itself.
impl SurfaceInputReceiver for Box<dyn SceneContent> {
    fn input(&mut self, event: &SurfaceInputEvent) {
        SceneContent::input(&mut **self, event);
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        SceneContent::ime_caret(&**self)
    }
}

/// Bridges a [`SurfaceInputReceiver`] to the renderer's embedded input routing.
///
/// Constructed fresh on every registration; [`Self::identity`] reports the
/// receiver it drives, which outlives the frame.
pub struct SurfaceInputSink<R> {
    receiver: Rc<RefCell<R>>,
}

impl<R: SurfaceInputReceiver> SurfaceInputSink<R> {
    pub(crate) const fn new(receiver: Rc<RefCell<R>>) -> Self {
        Self { receiver }
    }

    fn send(&self, event: &SurfaceInputEvent) {
        self.receiver.borrow_mut().input(event);
    }
}

/// The W3C UI Events button vocabulary has no room for a platform's extra
/// buttons, so an unmapped button is dropped rather than reported as one the
/// view would act on.
const fn surface_pointer_button(button: PointerButton) -> Option<SurfacePointerButton> {
    match button {
        PointerButton::Primary => Some(SurfacePointerButton::Primary),
        PointerButton::Secondary => Some(SurfacePointerButton::Secondary),
        PointerButton::Middle => Some(SurfacePointerButton::Middle),
        PointerButton::Back => Some(SurfacePointerButton::Back),
        PointerButton::Forward => Some(SurfacePointerButton::Forward),
        PointerButton::Other(_) => None,
    }
}

impl<R: SurfaceInputReceiver> EmbeddedInputSink for SurfaceInputSink<R> {
    fn identity(&self) -> *const () {
        Rc::as_ptr(&self.receiver).cast()
    }

    fn set_focus(&self, focused: bool) {
        self.send(&SurfaceInputEvent::Focus(focused));
    }

    fn set_modifiers(&self, modifiers: Modifiers) {
        self.send(&SurfaceInputEvent::Modifiers(modifiers.into()));
    }

    fn pointer_move(&self, position: kurbo::Point) {
        self.send(&SurfaceInputEvent::PointerMove { position });
    }

    fn pointer_button(&self, pressed: bool, button: PointerButton, position: kurbo::Point) {
        let Some(button) = surface_pointer_button(button) else {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                button = ?button,
                "dropped a pointer button with no W3C meaning for an embedded surface"
            );
            return;
        };
        self.send(&SurfaceInputEvent::PointerButton {
            pressed,
            button,
            position,
        });
    }

    fn scroll(
        &self,
        position: kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        unit: ScrollUnit,
        finished: bool,
    ) {
        self.send(&SurfaceInputEvent::Scroll {
            position,
            // winit is positive right/down; SurfaceInputEvent uses the W3C WheelEvent sign.
            delta_x: -f64::from(delta_x),
            delta_y: -f64::from(delta_y),
            unit,
            finished,
        });
    }

    fn key(&self, delivery: &KeyDelivery<'_>) {
        self.send(&SurfaceInputEvent::Key {
            pressed: delivery.pressed,
            key: delivery.logical.clone(),
            code: delivery.code,
            modifiers: delivery.modifiers.into(),
            repeat: delivery.repeat,
        });
    }

    fn text_input(&self, text: &str) {
        self.send(&SurfaceInputEvent::TextInput(text.to_owned().into()));
    }

    fn composition_start(&self) {
        self.send(&SurfaceInputEvent::CompositionStart);
    }

    fn composition_update(&self, text: &str, caret: Option<usize>) {
        self.send(&SurfaceInputEvent::CompositionUpdate {
            text: text.to_owned().into(),
            caret,
        });
    }

    fn composition_commit(&self, text: &str) {
        self.send(&SurfaceInputEvent::CompositionCommit(
            text.to_owned().into(),
        ));
    }

    fn composition_cancel(&self) {
        self.send(&SurfaceInputEvent::CompositionCancel);
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.receiver.borrow().ime_caret()
    }
}

#[expect(
    clippy::needless_pass_by_ref_mut,
    reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
)]
impl SemanticCore {
    /// Registers an embedded input target at a laid-out surface's bounds.
    ///
    /// `transform` maps `local_bounds` into window hit-test space, which is
    /// already logical: the projection back through its inverse is exactly the
    /// logical surface-local position the sink is contracted to receive, with
    /// no display-scale division anywhere on the path.
    ///
    /// `interaction_key` is the surface owner's interaction identity — what
    /// keyboard focus and a `.focused(binding)` write address the surface by.
    /// `accessibility_node_id` is the surface's semantic node, when the
    /// semantic tree exists.
    pub(crate) fn register_embedded_input_target(
        &mut self,
        local_bounds: kurbo::Rect,
        transform: kurbo::Affine,
        sink: Rc<dyn EmbeddedInputSink>,
        interaction_key: InteractionKey,
        #[cfg(feature = "accessibility")] accessibility_node_id: Option<AccessibilityNodeId>,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let determinant = transform.determinant();
        assert!(
            determinant.is_finite() && determinant.abs() > f64::EPSILON,
            "embedded surface input transform must be finite and invertible"
        );
        let order = self.hit_test.next_hit_test_order();
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            bounds = ?local_bounds,
            window_bounds = ?transform.transform_rect_bbox(local_bounds),
            order,
            "registered an embedded surface input target"
        );
        let key_handlers = self.snapshot_key_handlers();
        self.hit_test
            .embedded_input_targets
            .push(EmbeddedInputTarget {
                interaction_key,
                local_bounds,
                hit_clip: self.hit_test.hit_clip_stack.last().copied(),
                inverse_transform: transform.inverse(),
                depth: self.render_depth,
                order,
                sink,
                key_handlers,
                focus_binding: None,
                #[cfg(feature = "accessibility")]
                accessibility_node_id,
            });
    }

    /// Registers a surface whose drawing asked for input: an embedded
    /// [`GpuContentView`](waterui_graphics::GpuContentView) runtime or a
    /// [`SceneView`](waterui_graphics::SceneView)'s content.
    ///
    /// `focus_node` is the surface's semantic node, when the semantic tree
    /// exists.
    pub(crate) fn register_surface_input_target<R: SurfaceInputReceiver + 'static>(
        &mut self,
        local_bounds: kurbo::Rect,
        transform: kurbo::Affine,
        receiver: Rc<RefCell<R>>,
        #[cfg(feature = "accessibility")] focus_node: Option<AccessibilityNodeId>,
    ) {
        let interaction_key = InteractionKey::for_rc(&receiver, 0);
        self.register_embedded_input_target(
            local_bounds,
            transform,
            Rc::new(SurfaceInputSink::new(receiver)),
            interaction_key,
            #[cfg(feature = "accessibility")]
            focus_node,
        );
    }

    pub(super) fn topmost_embedded_target_at(
        &self,
        point: kurbo::Point,
    ) -> Option<(usize, kurbo::Point)> {
        self.hit_test
            .embedded_input_targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                target
                    .local_position(point)
                    .map(|position| (index, position))
            })
            .max_by(|(left, _), (right, _)| {
                let left_target = &self.hit_test.embedded_input_targets[*left];
                let right_target = &self.hit_test.embedded_input_targets[*right];
                Self::target_hit_priority(left_target.depth, left_target.order, *left).cmp(
                    &Self::target_hit_priority(right_target.depth, right_target.order, *right),
                )
            })
    }

    pub(super) fn embedded_target_wins_at(
        &self,
        point: kurbo::Point,
        pointer_priority: Option<(usize, usize, usize)>,
        text_priority: Option<(usize, usize, usize)>,
    ) -> Option<(usize, EmbeddedInputTarget, kurbo::Point)> {
        let (index, position) = self.topmost_embedded_target_at(point)?;
        let target = &self.hit_test.embedded_input_targets[index];
        let embedded_priority = Self::target_hit_priority(target.depth, target.order, index);
        if pointer_priority.is_some_and(|priority| priority > embedded_priority)
            || text_priority.is_some_and(|priority| priority > embedded_priority)
        {
            return None;
        }
        Some((index, target.clone(), position))
    }

    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(crate) fn handle_embedded_pointer_move(&mut self, point: kurbo::Point) -> bool {
        if let Some(target) = self.hit_test.active_embedded_target.as_ref() {
            target
                .sink
                .pointer_move(target.local_position_unclamped(point));
            return true;
        }
        // Pointer-capture semantics: a press that landed on a gesture
        // recognizer, a `captures_drag` target, or a text-selection drag owns
        // the sequence until release — surfaces it crosses see none of its
        // moves. A surface that took the press itself keeps its own capture
        // through `active_embedded_target` above.
        if self.gesture_engine.has_active_recognizer()
            || self.hit_test.active_pointer_drag_target.is_some()
            || self.text_editing.selection_drag_index().is_some()
        {
            return false;
        }
        let pointer_priority = self
            .hit_test
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, target)| Self::target_hit_priority(target.depth, target.order, index))
            .max();
        let text_priority = self.topmost_text_input_index_at_point(point).map(|index| {
            let target = &self.text_editing.text_input_targets[index];
            Self::target_hit_priority(target.depth, target.order, index)
        });
        let Some((_, target, position)) =
            self.embedded_target_wins_at(point, pointer_priority, text_priority)
        else {
            return false;
        };
        target.sink.pointer_move(position);
        true
    }

    /// Delivers the release of a key whose press bubbled into an embedded
    /// surface back to that same surface — the focused sink never saw the
    /// press, so it must not see the release either.
    pub(crate) fn handle_bubbled_key_release(&mut self, delivery: &KeyDelivery<'_>) -> bool {
        if delivery.pressed {
            return false;
        }
        let Some(index) =
            self.hit_test.bubbled_key_sinks.iter().position(|entry| {
                entry.code == delivery.code && entry.logical == *delivery.logical
            })
        else {
            return false;
        };
        self.hit_test.bubbled_key_sinks[index].sink.key(delivery);
        self.hit_test.bubbled_key_sinks.remove(index);
        true
    }

    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(crate) fn handle_embedded_key(&mut self, delivery: &KeyDelivery<'_>) -> bool {
        // GTK's text-view convention: while a surface holds keyboard focus,
        // Tab and Shift-Tab are surface input like any other key — a
        // terminal needs them for completion and backtab. Ctrl+Tab and
        // Ctrl+Shift+Tab are the way out: the surface never sees them, the
        // traversal gate takes them, and the move sends the surface its
        // `Focus(false)` exactly as it does for any other focused control.
        if matches!(delivery.logical, Key::Named(NamedKey::Tab)) && delivery.modifiers.control {
            return false;
        }
        let Some(sink) = self.hit_test.focused_embedded_sink.as_ref() else {
            return false;
        };
        sink.key(delivery);
        true
    }

    pub(crate) fn handle_embedded_text_input(&mut self, text: &str) -> bool {
        let Some(sink) = self.hit_test.focused_embedded_sink.as_ref() else {
            return false;
        };
        sink.text_input(text);
        true
    }

    /// Drives the composition state machine for the focused surface.
    ///
    /// The platform reports pre-edit text; the W3C session (`start` → `update`
    /// … → `commit`/`cancel`) is derived here, once, so no sink has to infer
    /// it. An empty pre-edit while composing is the platform abandoning the
    /// session.
    pub(crate) fn handle_embedded_ime_preedit(&mut self, text: &str, caret: Option<usize>) -> bool {
        let Some(sink) = self.hit_test.focused_embedded_sink.clone() else {
            return false;
        };
        if text.is_empty() {
            if self.hit_test.embedded_composing {
                self.hit_test.embedded_composing = false;
                sink.composition_cancel();
            }
            return true;
        }
        if !self.hit_test.embedded_composing {
            self.hit_test.embedded_composing = true;
            sink.composition_start();
        }
        sink.composition_update(text, caret);
        true
    }

    pub(crate) fn handle_embedded_ime_commit(&mut self, text: &str) -> bool {
        let Some(sink) = self.hit_test.focused_embedded_sink.clone() else {
            return false;
        };
        // A platform may commit without ever having sent a pre-edit (a dead
        // key resolving, a candidate picked from a palette). That is still a
        // composition as far as the surface is concerned, so open the session
        // rather than passing the text off as a plain insertion.
        if !self.hit_test.embedded_composing {
            sink.composition_start();
        }
        self.hit_test.embedded_composing = false;
        sink.composition_commit(text);
        true
    }

    pub(crate) fn handle_embedded_ime_disabled(&mut self) -> bool {
        let Some(sink) = self.hit_test.focused_embedded_sink.clone() else {
            return false;
        };
        if !self.hit_test.embedded_composing {
            return false;
        }
        self.hit_test.embedded_composing = false;
        sink.composition_cancel();
        true
    }

    pub(crate) fn update_embedded_modifiers(&mut self, modifiers: Modifiers) {
        self.hit_test.modifiers = modifiers;
        if let Some(sink) = self.hit_test.focused_embedded_sink.as_ref() {
            sink.set_modifiers(modifiers);
        }
    }

    /// The window itself gained or lost focus.
    ///
    /// Blur is not a focus move: the sink holding keyboard focus keeps it —
    /// as platforms keep the focused element of an inactive window — and is
    /// only told focus left (`Focus(false)`), hearing it return
    /// (`Focus(true)`) on the refocus, which is what a terminal's focus
    /// reporting (DECSET 1004) needs. Element-focus moves made while the
    /// window is blurred emit no Focus events at all — both surfaces stay
    /// unfocused as far as focus reporting is concerned — so the holder
    /// hears a single `Focus(true)` on the refocus.
    pub(crate) fn handle_window_focused(&mut self, focused: bool) -> bool {
        if self.hit_test.window_blurred != focused {
            return false;
        }
        self.hit_test.window_blurred = !focused;
        let Some(sink) = self.hit_test.focused_embedded_sink.as_ref() else {
            return false;
        };
        sink.set_focus(focused);
        true
    }

    /// The embedded-input target registered for the surface owner behind
    /// `key`, if the surface is still mounted.
    pub(crate) fn embedded_index_for_key(&self, key: &InteractionKey) -> Option<usize> {
        self.hit_test
            .embedded_input_targets
            .iter()
            .position(|target| &target.interaction_key == key)
    }

    /// The embedded-input target behind the semantic node the surface
    /// emitted, when the tree walks it.
    #[cfg(feature = "accessibility")]
    pub(crate) fn embedded_index_for_node(&self, node: AccessibilityNodeId) -> Option<usize> {
        self.hit_test
            .embedded_input_targets
            .iter()
            .position(|target| target.accessibility_node_id == Some(node))
    }

    /// Whether the surface owner behind `key` holds embedded focus.
    pub(crate) fn is_focused_embedded(&self, key: &InteractionKey) -> bool {
        self.hit_test.focused_embedded_key.as_ref() == Some(key)
    }

    /// Moves embedded focus to the surface owner `key` names — the
    /// programmatic half of the shared focus model, reached by
    /// `.focused(binding)` writes and the keyboard traversal alike.
    ///
    /// Taking focus: the surface takes semantic focus with it, its sink
    /// receives `Focus(true)`, and editing on any text field ends — the same
    /// rule a pointer press on the surface follows. Releasing it drops
    /// semantic focus only while it still rests on that surface, then sends
    /// `Focus(false)`.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn set_focused_embedded_key(&mut self, focused: Option<InteractionKey>) -> bool {
        let previous = self.hit_test.focused_embedded_key.clone();
        if previous == focused {
            return false;
        }
        let index = focused
            .as_ref()
            .and_then(|key| self.embedded_index_for_key(key));
        let mut changed = false;
        if let Some(key) = focused.as_ref() {
            #[cfg(feature = "accessibility")]
            let node = self.focus_node_for_key(key);
            changed |= self.set_keyboard_focus_impl(
                Some(key.clone()),
                #[cfg(feature = "accessibility")]
                node,
                self.hit_test.keyboard_focus_visible,
            );
            changed |= self.hit_test.set_embedded_focus_index(index);
            // Landing on a surface ends text editing exactly as a
            // pointer press on one does (#95's rule).
            changed |= self.set_focused_text_input(None);
        } else {
            if self.hit_test.keyboard_focus == previous {
                changed |= self.set_keyboard_focus_impl(
                    None,
                    #[cfg(feature = "accessibility")]
                    None,
                    false,
                );
            }
            changed |= self.hit_test.set_embedded_focus_index(None);
        }
        changed
    }

    /// The focused embedded surface's caret, in window hit-test space.
    ///
    /// The surface reports it in its own logical coordinates; its live target
    /// supplies the transform, so a surface that has moved since it was
    /// focused still places the candidate window correctly.
    pub(crate) fn focused_embedded_ime_caret(&self) -> Option<kurbo::Rect> {
        let sink = self.hit_test.focused_embedded_sink.as_ref()?;
        let caret = sink.ime_caret()?;
        let target = self
            .hit_test
            .embedded_input_targets
            .iter()
            .find(|target| target.sink.identity() == sink.identity())?;
        Some(target.to_window_rect(caret))
    }
}
