//! Replayable interaction state layers.
//!
//! Hover, press, and focus visuals are captured once per structural rebuild as
//! retained fragments wrapped in [`DynamicOpacityDraw`]/[`DynamicTransformDraw`]
//! nodes whose scalars are renderer-local animation handles. Pointer
//! interactions then *replay* the retained window frame (re-sampling those
//! handles) instead of re-dispatching the view tree, which keeps hover/press
//! feedback on the cheap window-refresh path.

use super::*;
use waterui_backend_core::widget::{InteractionMotion, WidgetInteractionState};

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
    focus_alpha: AnimatedScalarHandle,
    /// Origin of the active press in the widget's local coordinates, shared
    /// with the retained ripple transform so replay tracks the live press.
    origin: Cell<Option<vello::kurbo::Point>>,
    /// Maps window-space pointer coordinates into the captured fragment's
    /// local frame; refreshed whenever the widget's targets are (re)registered.
    window_to_local: Cell<vello::kurbo::Affine>,
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
        focus_alpha: AnimatedScalarHandle,
        motion: InteractionMotion,
    ) -> Self {
        Self {
            hover_alpha,
            press_alpha,
            press_progress,
            focus_alpha,
            origin: Cell::new(None),
            window_to_local: Cell::new(vello::kurbo::Affine::IDENTITY),
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
        self.window_to_local.set(previous.window_to_local.get());
        self.hovering.set(previous.hovering.get());
        self.pressing.set(previous.pressing.get());
        self.pressed_at.set(previous.pressed_at.get());
        self.released_at.set(previous.released_at.get());
    }

    /// The active press origin mapped back into window coordinates.
    pub(crate) fn origin_in_window(&self) -> Option<vello::kurbo::Point> {
        self.origin
            .get()
            .map(|origin| self.window_to_local.get().inverse() * origin)
    }

    pub(crate) fn origin(&self) -> Option<vello::kurbo::Point> {
        self.origin.get()
    }

    pub(crate) fn hover_alpha(&self) -> &AnimatedScalarHandle {
        &self.hover_alpha
    }

    pub(crate) fn press_alpha(&self) -> &AnimatedScalarHandle {
        &self.press_alpha
    }

    pub(crate) fn press_progress(&self) -> &AnimatedScalarHandle {
        &self.press_progress
    }

    pub(crate) fn focus_alpha(&self) -> &AnimatedScalarHandle {
        &self.focus_alpha
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


    /// Records where window-space pointer coordinates land in the captured
    /// fragment's local frame; called at every target (re)registration.
    pub(crate) fn set_window_to_local(&self, window_to_local: vello::kurbo::Affine) {
        self.window_to_local.set(window_to_local);
    }

    /// Starts a press at a window-space origin: the ripple snaps back to its
    /// origin scale and grows, while the press layer fades in.
    pub(crate) fn begin_press(&self, origin: vello::kurbo::Point, now: Instant) {
        self.origin.set(Some(self.window_to_local.get() * origin));
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

/// The press ripple's replayable kinematics: the fragment is painted at full
/// progress (centered, final size); replay scales and translates it back along
/// the Material ripple path as the progress handle is re-sampled.
pub(crate) struct DynamicRippleTransform {
    pub(super) progress: DynamicTransformScalar,
    /// Live interaction handles providing the press origin at replay time.
    pub(super) handles: Rc<InteractionLayerHandles>,
    /// Painted bounds of the state layer in local coordinates.
    pub(super) bounds: vello::kurbo::Rect,
    /// Ripple scale at progress 0 relative to its painted (final) size.
    pub(super) initial_scale: f64,
}

impl DynamicRippleTransform {
    pub(super) fn affine(&self, now: Instant) -> vello::kurbo::Affine {
        let progress = f64::from(self.progress.sample(now)).clamp(0.0, 1.0);
        let center = vello::kurbo::Point::new(
            self.bounds.x0 + self.bounds.width() * 0.5,
            self.bounds.y0 + self.bounds.height() * 0.5,
        );
        let origin = self.handles.origin().unwrap_or(center);
        let scale = self.initial_scale + (1.0 - self.initial_scale) * progress;
        // The fragment is painted centered at `center`; at progress `p` the
        // ripple center sits at origin.lerp(center, p) with scale `scale`.
        let target_center = vello::kurbo::Point::new(
            origin.x + (center.x - origin.x) * progress,
            origin.y + (center.y - origin.y) * progress,
        );
        vello::kurbo::Affine::translate((
            target_center.x - center.x * scale,
            target_center.y - center.y * scale,
        )) * vello::kurbo::Affine::scale(scale)
    }
}

impl HydrolysisRenderer {
    /// Paints a closure into a fresh fragment scene using the widget's local
    /// coordinates, leaving the live scene untouched.
    fn paint_fragment(
        &mut self,
        ctx: RenderContext,
        paint: &dyn Fn(&mut VelloDrawContext<'_>),
    ) -> DynamicSubtree {
        let mut subtree = DynamicSubtree::for_capture(self.render_depth);
        let mut fragment_scene = vello::Scene::new();
        core::mem::swap(&mut self.scene, &mut fragment_scene);
        {
            let local = ctx.with_identity_transforms(ctx.bounds);
            let mut draw = VelloDrawContext::with_root_transform(&mut self.scene, local.transform);
            paint(&mut draw);
        }
        core::mem::swap(&mut self.scene, &mut fragment_scene);
        // A state-layer fragment is pure paint: its whole content is one static
        // segment, which the wrapping opacity/transform draw modulates at replay.
        subtree
            .draw_ops
            .push(DynamicDrawOp::Static(fragment_scene));
        subtree
    }

    /// Captures the hover, press, and focus state layers of one interactive
    /// widget as replayable retained draws.
    ///
    /// `paint` is the widget's themed state-layer painter; it is invoked with
    /// synthetic [`WidgetInteractionState`] values that select exactly one
    /// layer at unit opacity and full progress, so the captured fragments can
    /// be modulated by the live animation handles at replay time.
    pub(crate) fn capture_state_layers(
        &mut self,
        ctx: RenderContext,
        handles: &Rc<InteractionLayerHandles>,
        layer_bounds: vello::kurbo::Rect,
        with_focus: bool,
        paint: &dyn Fn(&mut VelloDrawContext<'_>, WidgetInteractionState),
    ) {
        let now = self.frame_instant;
        handles.set_window_to_local(ctx.hit_transform.inverse());
        let local_bounds = layer_bounds;

        // Hover layer: painted at unit opacity, replayed with the hover alpha.
        let hover_state = WidgetInteractionState {
            hovered: true,
            state_layer_opacity: 1.0,
            ..WidgetInteractionState::NONE
        };
        let hover_subtree = self.paint_fragment(ctx, &|draw| paint(draw, hover_state));
        // The widget's own painted content (drawn before this call) becomes a
        // static segment; the state layers stack above it in draw order.
        self.flush_static_segment();
        self.draw_ops
            .push(DynamicDrawOp::Opacity(DynamicOpacityDraw::paint_only(
                DynamicTransformScalar::with_handle(
                    handles.hover_alpha().sample(now),
                    handles.hover_alpha().clone(),
                ),
                ctx,
                local_bounds,
                hover_subtree,
            )));

        // Press ripple: painted at full progress and unit opacity, replayed
        // through the Material ripple kinematics and the press alpha.
        let ripple_center = vello::kurbo::Point::new(
            local_bounds.x0 + local_bounds.width() * 0.5,
            local_bounds.y0 + local_bounds.height() * 0.5,
        );
        let press_state = WidgetInteractionState {
            pressed: true,
            press_layer_opacity: 1.0,
            press_origin: Some(ripple_center),
            press_progress: 1.0,
            ..WidgetInteractionState::NONE
        };
        let press_subtree = self.paint_fragment(ctx, &|draw| paint(draw, press_state));
        let ripple = DynamicRippleTransform {
            progress: DynamicTransformScalar::with_handle(
                handles.press_progress().sample(now),
                handles.press_progress().clone(),
            ),
            handles: Rc::clone(handles),
            bounds: local_bounds,
            initial_scale: RIPPLE_INITIAL_SCALE,
        };
        let mut press_wrapper = DynamicSubtree::for_capture(self.render_depth);
        press_wrapper
            .draw_ops
            .push(DynamicDrawOp::Transform(DynamicTransformDraw::paint_only(
                DynamicTransformComponents::ripple(ripple),
                local_bounds,
                press_subtree,
            )));
        self.flush_static_segment();
        self.draw_ops
            .push(DynamicDrawOp::Opacity(DynamicOpacityDraw::paint_only(
                DynamicTransformScalar::with_handle(
                    handles.press_alpha().sample(now),
                    handles.press_alpha().clone(),
                ),
                ctx,
                local_bounds,
                press_wrapper,
            )));

        // Focus affordance: painted at full progress, replayed with focus alpha.
        if with_focus {
            let focus_state = WidgetInteractionState {
                focus_visible: true,
                focus_progress: 1.0,
                ..WidgetInteractionState::NONE
            };
            let focus_subtree = self.paint_fragment(ctx, &|draw| paint(draw, focus_state));
            self.flush_static_segment();
            self.draw_ops
                .push(DynamicDrawOp::Opacity(DynamicOpacityDraw::paint_only(
                    DynamicTransformScalar::with_handle(
                        handles.focus_alpha().sample(now),
                        handles.focus_alpha().clone(),
                    ),
                    ctx,
                    local_bounds,
                    focus_subtree,
                )));
        }
    }

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

/// Scale of the Material press ripple at progress 0, relative to its final
/// solid-circle size. Material Web's ripple wave starts at `scale(0.4)` and
/// grows to `scale(1)` while its center drifts to the surface center.
const RIPPLE_INITIAL_SCALE: f64 = 0.4;
