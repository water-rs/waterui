//! Generic embedded-surface input routing.
//!
//! An embedded surface is a rectangle of the window that draws its own
//! interactive content and therefore owns the input landing on it: a browser
//! engine, or any [`GpuSurface`](waterui_graphics::GpuSurface) whose view asks
//! for input with
//! [`wants_input_events`](waterui_graphics::GpuView::wants_input_events).
//!
//! There is one target list, one hit-test arbitration and one focus/capture
//! state machine for both, reached through [`EmbeddedInputSink`], and one
//! vocabulary at the far end of it: every sink translates into the
//! backend-neutral
//! [`SurfaceInputEvent`](waterui_graphics::input::SurfaceInputEvent). The
//! browser engines are ordinary GPU surfaces now — the CEF and WPE crates own
//! their own input ABIs — so the renderer knows nothing about any of them.

use super::*;
use crate::renderer::render::EmbeddedGpuSurfaceRuntime;
use waterui_graphics::input::{Code, Key, ScrollUnit, SurfaceInputEvent, SurfacePointerButton};

/// One key transition, in the W3C UI Events vocabulary.
pub(crate) struct KeyDelivery<'a> {
    pub(crate) pressed: bool,
    pub(crate) logical: &'a Key,
    pub(crate) code: Code,
    pub(crate) repeat: bool,
    pub(crate) modifiers: Modifiers,
}

/// Backend-owned input sink for one embedded surface.
///
/// Positions are logical and surface-local: the surface's own top-left is
/// `(0, 0)`.
pub(crate) trait EmbeddedInputSink {
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
    fn pointer_move(&self, position: vello::kurbo::Point);
    fn pointer_button(&self, pressed: bool, button: PointerButton, position: vello::kurbo::Point);
    fn scroll(
        &self,
        position: vello::kurbo::Point,
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
    fn ime_caret(&self) -> Option<vello::kurbo::Rect>;
}

#[derive(Clone)]
pub(crate) struct EmbeddedInputTarget {
    pub(crate) local_bounds: vello::kurbo::Rect,
    pub(crate) inverse_transform: vello::kurbo::Affine,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) sink: Rc<dyn EmbeddedInputSink>,
}

impl EmbeddedInputTarget {
    pub(crate) fn local_position(&self, point: vello::kurbo::Point) -> Option<vello::kurbo::Point> {
        self.local_bounds
            .contains(self.inverse_transform * point)
            .then(|| self.local_position_unclamped(point))
    }

    /// The surface-local position of a window point, whether or not it is
    /// inside the surface. Used while this target holds the pointer capture, a
    /// drag that has left the surface still being the surface's drag.
    pub(crate) fn local_position_unclamped(
        &self,
        point: vello::kurbo::Point,
    ) -> vello::kurbo::Point {
        let local = self.inverse_transform * point;
        vello::kurbo::Point::new(
            local.x - self.local_bounds.x0,
            local.y - self.local_bounds.y0,
        )
    }

    /// Maps a surface-local rect back into window hit-test space.
    pub(crate) fn to_window_rect(&self, local: vello::kurbo::Rect) -> vello::kurbo::Rect {
        self.inverse_transform.inverse().transform_rect_bbox(
            local + vello::kurbo::Vec2::new(self.local_bounds.x0, self.local_bounds.y0),
        )
    }
}

/// Bridges an embedded [`GpuSurface`](waterui_graphics::GpuSurface) runtime to
/// the neutral [`SurfaceInputEvent`] vocabulary.
///
/// Constructed fresh on every registration; [`Self::identity`] reports the
/// runtime it drives, which outlives the frame.
pub(crate) struct GpuSurfaceInputSink {
    runtime: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
}

impl GpuSurfaceInputSink {
    pub(crate) const fn new(runtime: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>) -> Self {
        Self { runtime }
    }

    fn send(&self, event: &SurfaceInputEvent) {
        self.runtime.borrow_mut().input(event);
    }
}

/// The W3C UI Events button vocabulary has no room for a platform's extra
/// buttons, so an unmapped button is dropped rather than reported as one the
/// view would act on.
fn surface_pointer_button(button: PointerButton) -> Option<SurfacePointerButton> {
    match button {
        PointerButton::Primary => Some(SurfacePointerButton::Primary),
        PointerButton::Secondary => Some(SurfacePointerButton::Secondary),
        PointerButton::Middle => Some(SurfacePointerButton::Middle),
        PointerButton::Back => Some(SurfacePointerButton::Back),
        PointerButton::Forward => Some(SurfacePointerButton::Forward),
        PointerButton::Other(_) => None,
    }
}

impl EmbeddedInputSink for GpuSurfaceInputSink {
    fn identity(&self) -> *const () {
        Rc::as_ptr(&self.runtime).cast()
    }

    fn set_focus(&self, focused: bool) {
        self.send(&SurfaceInputEvent::Focus(focused));
    }

    fn set_modifiers(&self, modifiers: Modifiers) {
        self.send(&SurfaceInputEvent::Modifiers(modifiers.into()));
    }

    fn pointer_move(&self, position: vello::kurbo::Point) {
        self.send(&SurfaceInputEvent::PointerMove { position });
    }

    fn pointer_button(&self, pressed: bool, button: PointerButton, position: vello::kurbo::Point) {
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
        position: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        unit: ScrollUnit,
        finished: bool,
    ) {
        self.send(&SurfaceInputEvent::Scroll {
            position,
            delta_x: f64::from(delta_x),
            delta_y: f64::from(delta_y),
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

    fn ime_caret(&self) -> Option<vello::kurbo::Rect> {
        self.runtime.borrow().ime_caret()
    }
}

impl HydrolysisRenderer {
    /// Registers an embedded input target at a laid-out surface's bounds.
    ///
    /// `transform` maps `local_bounds` into window hit-test space, which is
    /// already logical: the projection back through its inverse is exactly the
    /// logical surface-local position the sink is contracted to receive, with
    /// no display-scale division anywhere on the path.
    pub(crate) fn register_embedded_input_target(
        &mut self,
        local_bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        sink: Rc<dyn EmbeddedInputSink>,
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
        self.hit_test
            .embedded_input_targets
            .push(EmbeddedInputTarget {
                local_bounds,
                inverse_transform: transform.inverse(),
                depth: self.render_depth,
                order,
                sink,
            });
    }

    /// Registers an embedded [`GpuSurface`](waterui_graphics::GpuSurface)
    /// runtime whose view asked for input.
    pub(crate) fn register_gpu_surface_input_target(
        &mut self,
        local_bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        runtime: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
    ) {
        self.register_embedded_input_target(
            local_bounds,
            transform,
            Rc::new(GpuSurfaceInputSink::new(runtime)),
        );
    }

    fn topmost_embedded_target_at(
        &self,
        point: vello::kurbo::Point,
    ) -> Option<(usize, vello::kurbo::Point)> {
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
        point: vello::kurbo::Point,
        pointer_priority: Option<(usize, usize, usize)>,
        text_priority: Option<(usize, usize, usize)>,
    ) -> Option<(EmbeddedInputTarget, vello::kurbo::Point)> {
        let (index, position) = self.topmost_embedded_target_at(point)?;
        let target = &self.hit_test.embedded_input_targets[index];
        let embedded_priority = Self::target_hit_priority(target.depth, target.order, index);
        if pointer_priority.is_some_and(|priority| priority > embedded_priority)
            || text_priority.is_some_and(|priority| priority > embedded_priority)
        {
            return None;
        }
        Some((target.clone(), position))
    }

    pub(crate) fn handle_embedded_pointer_move(&mut self, point: vello::kurbo::Point) -> bool {
        if let Some(target) = self.hit_test.active_embedded_target.as_ref() {
            target
                .sink
                .pointer_move(target.local_position_unclamped(point));
            return true;
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
        let Some((target, position)) =
            self.embedded_target_wins_at(point, pointer_priority, text_priority)
        else {
            return false;
        };
        target.sink.pointer_move(position);
        true
    }

    pub(crate) fn handle_embedded_scroll(
        &mut self,
        point: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        unit: ScrollUnit,
        finished: bool,
    ) -> bool {
        let Some((index, position)) = self.topmost_embedded_target_at(point) else {
            return false;
        };
        self.hit_test.embedded_input_targets[index]
            .sink
            .scroll(position, delta_x, delta_y, unit, finished);
        true
    }

    pub(crate) fn handle_embedded_key(&mut self, delivery: &KeyDelivery<'_>) -> bool {
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
        if let Some(sink) = self.hit_test.focused_embedded_sink.as_ref() {
            sink.set_modifiers(modifiers);
        }
    }

    /// The focused embedded surface's caret, in window hit-test space.
    ///
    /// The surface reports it in its own logical coordinates; its live target
    /// supplies the transform, so a surface that has moved since it was
    /// focused still places the candidate window correctly.
    pub(crate) fn focused_embedded_ime_caret(&self) -> Option<vello::kurbo::Rect> {
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
