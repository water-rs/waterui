//! Replayable interaction state layers.
//!
//! Hover, press, and focus visuals are captured once per structural rebuild as
//! retained fragments wrapped in [`DynamicOpacityDraw`]/[`DynamicTransformDraw`]
//! nodes whose scalars are renderer-local animation handles. Pointer
//! interactions then *replay* the retained window frame (re-sampling those
//! handles) instead of re-dispatching the view tree, which keeps hover/press
//! feedback on the cheap window-refresh path.

use super::*;
use waterui_backend_core::widget::InteractionMotion;

/// Shared handle bundle for one interactive widget's state layers.
///
/// Cloned into the widget's pointer/hover targets (and therefore into retained
/// subtrees), so input events occurring between structural rebuilds can apply
/// new animation targets directly without re-running `bind_widget_state`.
#[derive(Debug)]
pub(crate) struct InteractionLayerHandles {
    hover_alpha: AnimatedScalarHandle,
    press_alpha: AnimatedScalarHandle,
    press_progress: AnimatedScalarHandle,
    /// Origin of the active press in WINDOW coordinates (the raw pointer-down
    /// point). Widgets map it into their own local frame at draw time via
    /// `local_interaction_state` and the live hit transform, so it stays
    /// correct under arbitrary nesting and scroll offsets.
    origin: Cell<Option<vello::kurbo::Point>>,
    hovering: Cell<bool>,
    pressing: Cell<bool>,
    pressed_at: Cell<Option<Instant>>,
    released_at: Cell<Option<Instant>>,
    /// Widget chrome (not just the state layer) samples interaction state, so
    /// hover/press changes must re-render the widget instead of replaying.
    chrome_state_dependent: Cell<bool>,
    motion: InteractionMotion,
}

impl InteractionLayerHandles {
    pub(crate) fn new(
        hover_alpha: AnimatedScalarHandle,
        press_alpha: AnimatedScalarHandle,
        press_progress: AnimatedScalarHandle,
        motion: InteractionMotion,
    ) -> Self {
        Self {
            hover_alpha,
            press_alpha,
            press_progress,
            origin: Cell::new(None),
            hovering: Cell::new(false),
            pressing: Cell::new(false),
            pressed_at: Cell::new(None),
            released_at: Cell::new(None),
            chrome_state_dependent: Cell::new(false),
            motion,
        }
    }

    /// Marks this widget's chrome as sampling interaction state directly, so
    /// hover/press changes escalate to a re-render instead of a replay.
    pub(crate) fn mark_chrome_state_dependent(&self) {
        self.chrome_state_dependent.set(true);
    }

    pub(crate) fn chrome_state_dependent(&self) -> bool {
        self.chrome_state_dependent.get()
    }

    pub(crate) fn hovering(&self) -> bool {
        self.hovering.get()
    }

    pub(crate) fn pressing(&self) -> bool {
        self.pressing.get()
    }

    /// Whether the press layer should currently read as pressed: an active
    /// press, or a released press still inside the Material minimum press
    /// duration.
    pub(crate) fn visually_pressed(&self, now: Instant) -> bool {
        if self.pressing.get() {
            return true;
        }
        if self.released_at.get().is_none() {
            return false;
        }
        self.pressed_at.get().is_some_and(|pressed_at| {
            now.duration_since(pressed_at) < self.motion.minimum_press_duration
        })
    }

    /// Seeds the hover flag for a freshly created handle bundle.
    pub(crate) fn set_initial_hovering(&self, hovering: bool) {
        self.hovering.set(hovering);
    }

    /// Carries interaction state across a structural rebuild from the handle
    /// bundle the previous capture used for the same widget slot.
    pub(crate) fn copy_interaction_state_from(&self, previous: &Self) {
        self.origin.set(previous.origin.get());
        self.hovering.set(previous.hovering.get());
        self.pressing.set(previous.pressing.get());
        self.pressed_at.set(previous.pressed_at.get());
        self.released_at.set(previous.released_at.get());
    }

    /// The active press origin in window coordinates (the raw pointer-down point).
    pub(crate) fn origin_in_window(&self) -> Option<vello::kurbo::Point> {
        self.origin.get()
    }

    pub(crate) fn set_hovering(&self, hovering: bool, now: Instant) -> bool {
        if self.hovering.replace(hovering) == hovering {
            return false;
        }
        let (target, animation) = if hovering {
            (self.motion.hover_opacity, self.motion.hover_enter.clone())
        } else {
            (0.0, self.motion.hover_exit.clone())
        };
        self.hover_alpha.apply_target(target, Some(animation), now);
        true
    }

    /// Starts a press at a window-space origin: the ripple snaps back to its
    /// origin scale and grows, while the press layer fades in. The origin is
    /// stored in window space and mapped to the ripple's local frame at replay.
    pub(crate) fn begin_press(&self, origin: vello::kurbo::Point, now: Instant) {
        self.origin.set(Some(origin));
        self.pressing.set(true);
        self.pressed_at.set(Some(now));
        self.released_at.set(None);
        self.press_progress
            .apply_target(0.0, Some(Animation::linear(Duration::ZERO)), now);
        self.press_progress
            .apply_target(1.0, Some(self.motion.press_grow.clone()), now);
        self.press_alpha.apply_target(
            self.motion.pressed_opacity,
            Some(self.motion.press_fade_in.clone()),
            now,
        );
    }

    /// Ends a press. The fade-out is deferred until the Material minimum press
    /// duration has elapsed; [`Self::flush_release`] applies it from the
    /// animation tick.
    pub(crate) fn release(&self, now: Instant) -> bool {
        if !self.pressing.replace(false) {
            return false;
        }
        self.released_at.set(Some(now));
        self.flush_release(now);
        true
    }

    /// Applies the deferred press fade-out once the minimum press duration has
    /// elapsed. Returns `true` while a release is still pending, so the frame
    /// pump keeps scheduling animation frames until the fade-out starts.
    pub(crate) fn flush_release(&self, now: Instant) -> bool {
        if self.pressing.get() {
            return false;
        }
        let Some(released_at) = self.released_at.get() else {
            return false;
        };
        let minimum_elapsed = self.pressed_at.get().is_none_or(|pressed_at| {
            now.duration_since(pressed_at) >= self.motion.minimum_press_duration
        });
        if !minimum_elapsed {
            return true;
        }
        let _ = released_at;
        self.released_at.set(None);
        self.pressed_at.set(None);
        self.press_alpha
            .apply_target(0.0, Some(self.motion.press_fade_out.clone()), now);
        false
    }

    /// Drops all press state without animating (slot reuse by an unrelated
    /// widget across a rebuild).
    pub(crate) fn clear_press_state(&self) {
        self.origin.set(None);
        self.pressing.set(false);
        self.pressed_at.set(None);
        self.released_at.set(None);
    }

    /// Whether a released press is still waiting for its deferred fade-out
    /// (without applying it).
    pub(crate) fn has_pending_release(&self, now: Instant) -> bool {
        if self.pressing.get() || self.released_at.get().is_none() {
            return false;
        }
        self.pressed_at.get().is_some_and(|pressed_at| {
            now.duration_since(pressed_at) < self.motion.minimum_press_duration
        })
    }
}

impl HydrolysisRenderer {
    /// Applies any deferred press fade-outs and reports whether more
    /// animation frames are needed for pending releases.
    pub(crate) fn flush_interaction_releases(&mut self, now: Instant) -> bool {
        let mut pending = false;
        for target in &self.hit_test.pointer_targets {
            if let Some(handles) = &target.interaction {
                pending |= handles.flush_release(now);
            }
        }
        pending
    }

    /// Whether any released press is still waiting for its deferred fade-out.
    pub(crate) fn has_pending_interaction_releases(&self, now: Instant) -> bool {
        self.hit_test.pointer_targets.iter().any(|target| {
            target
                .interaction
                .as_ref()
                .is_some_and(|handles| handles.has_pending_release(now))
        })
    }
}
