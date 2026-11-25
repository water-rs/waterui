//! Anchored overlays: `.anchored_overlay(...)` (water-rs/waterui#1275).
//!
//! An anchored overlay is window-level content presented next to the view it
//! modifies while its `is_presented` binding reads `true`. The anchor's flush
//! registers it here every frame; after the tree flushes,
//! [`HydrolysisRenderer::render_anchored_overlays`] measures each open
//! overlay at its ideal size, runs the shared placement contract
//! (`waterui_backend_core::overlay::place_anchored_overlay`), and flushes the
//! content into its placed frame — above everything else the window drew.
//!
//! Re-registering every frame is what keeps the contract: the anchor moving,
//! the window resizing, or the anchor leaving the tree all show up in the
//! registration set — a moved/resized anchor produces a new frame, and an
//! anchor that stopped registering closes its overlay.
//!
//! Each presentation is a structural transition site (layout-spec.md §4.5): a
//! `.transition(...)` on the overlay body's root runs as the presentation's
//! insertion, and a dismissed presentation leaves the semantic tree at once
//! and keeps painting in the slot it was placed at until its removal
//! completes, while a re-present enters as a new instance beside it.

use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "menu")]
use cherenkov_record::Animation as EngineAnimation;
use kurbo::Rect;
#[cfg(feature = "menu")]
use kurbo::{Affine, Vec2};
use nami::Binding;
use waterui::metadata::anchored_overlay::{AnchorEdge, AnchorPlacement, Clamp, Dismissal};
use waterui_backend_core::overlay::{logical_edge, place_anchored_overlay};
use waterui_core::Environment;
use waterui_core::layout::{Size as LayoutSize, layout_direction};
#[cfg(feature = "menu")]
use waterui_graphics::animation::engine_animation;
use waterui_structural_transition::{ReduceMotion, TransitionSpec};

#[cfg(feature = "menu")]
use crate::num_cast::f64_as_f32;
#[cfg(feature = "menu")]
use crate::renderer::mount::ScopeKey;
use crate::renderer::tree::transition::{
    TransitionBlur, TransitionPaint, TransitionRun, claim_transition, flush_transitioned,
};
#[cfg(feature = "menu")]
use crate::renderer::{NodeCell, RenderNode, SafeAreaLayout, ScopeDelta};
#[cfg(feature = "menu")]
use crate::text::TextEngine;
#[cfg(feature = "menu")]
use crate::theme::{MenuPresentationMotion, MenuPresentationSample};
#[cfg(feature = "menu")]
use crate::time::Instant;

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

/// What an anchor's flush registered: the anchor frame in hit space plus the
/// handles the post-flush render pass needs. The presentations live on the
/// effect (`Rc`-shared) so a presented overlay's built node survives the
/// anchor's own reconcile.
pub struct RegisteredAnchoredOverlay<E: crate::text::TextEngine> {
    /// The anchor's frame in hit space this frame.
    pub(crate) anchor: kurbo::Rect,
    /// The placement contract from the metadata.
    pub(crate) placement: AnchorPlacement,
    /// How the overlay closes.
    pub(crate) dismissal: Dismissal,
    /// Whether the overlay is presented right now.
    pub(crate) presented: bool,
    /// The binding the renderer writes `false` to on outside dismissal, and
    /// the anchor's environment the overlay content is built and flushed in.
    pub(crate) is_presented: Binding<bool>,
    /// Written on every placement with the logical edge the overlay was
    /// placed against after any flip.
    pub(crate) placed_edge: Binding<AnchorEdge>,
    /// The node's environment — the overlay content inherits `.state` and
    /// plugin values from where the anchor declared it.
    pub(crate) env: Environment,
    /// The overlay's view builder: every presentation calls it, so each one
    /// gets an independent view instance.
    pub(crate) builder: Rc<dyn Fn() -> HeldChild<E>>,
    /// The presentations the owning effect keeps across frames.
    pub(crate) presentations: Rc<RefCell<AnchoredPresentations<E>>>,
    /// Identity shared between the effect and its registration, so the render
    /// pass can tell a re-registered anchor from a dropped one.
    pub(crate) marker: Rc<()>,
    /// Whether the placed panel overlays the trigger frame (top-aligned and
    /// horizontally centered on `anchor`, flipping to bottom-aligned when
    /// constrained) instead of opening beside an anchor edge.
    pub(crate) cover_trigger: bool,
}

/// Live presentation sample handed to [`PopupMenuSurface`] while an animated
/// anchored overlay is recording its subtree.
#[cfg(feature = "menu")]
pub struct ActiveMenuPresentation<E: TextEngine> {
    /// Sampled bounds, progress, and reveal factors in window coordinates.
    pub(crate) sample: MenuPresentationSample,
    /// Scope identity of the presentation owning this menu surface.
    pub(crate) scope: u64,
    /// Lazily allocated blur filter host for content reveal blur.
    pub(crate) blur: Rc<RefCell<Option<TransitionBlur<E>>>>,
    /// Root cell owning `blur` while the presentation is live.
    pub(crate) owner: Rc<NodeCell<E>>,
}

/// Content transform placing a menu panel's content at the centre of the
/// morphing `surface` scaled uniformly by `scale`.
#[cfg(feature = "menu")]
pub fn presentation_content_transform(panel: Rect, surface: Rect, scale: f64) -> Affine {
    let panel_center = panel.center();
    let surface_center = surface.center();
    Affine::translate((surface_center.x, surface_center.y))
        * Affine::scale(scale)
        * Affine::translate((-panel_center.x, -panel_center.y))
}

/// Theme-driven presentation morph progress (`0.0` at trigger -> `1.0` settled)
/// evaluated through the shared engine timing sampler so isotropic or
/// anisotropic springs preserve both position and velocity across mid-flight
/// dismissal.
#[cfg(feature = "menu")]
struct PresentationMorph {
    motion: MenuPresentationMotion,
    from: [f64; 2],
    velocity: [f64; 2],
    target: f64,
    animation_x: EngineAnimation,
    animation_y: EngineAnimation,
    start: Instant,
    dismissing: bool,
    settled: bool,
}

#[cfg(feature = "menu")]
impl PresentationMorph {
    fn present(motion: MenuPresentationMotion, now: Instant, reduce_motion: bool) -> Self {
        let animation_x = engine_animation(&motion.present);
        let animation_y =
            engine_animation(motion.present_vertical.as_ref().unwrap_or(&motion.present));
        if reduce_motion {
            Self {
                motion,
                from: [1.0, 1.0],
                velocity: [0.0, 0.0],
                target: 1.0,
                animation_x,
                animation_y,
                start: now,
                dismissing: false,
                settled: true,
            }
        } else {
            Self {
                motion,
                from: [0.0, 0.0],
                velocity: [0.0, 0.0],
                target: 1.0,
                animation_x,
                animation_y,
                start: now,
                dismissing: false,
                settled: false,
            }
        }
    }

    fn eval_axis(
        animation: &EngineAnimation,
        from: f64,
        velocity: f64,
        target: f64,
        dt: f64,
    ) -> (f64, f64, bool) {
        let mut position = [0.0_f64];
        let mut vel = [0.0_f64];
        let done = animation
            .playback()
            .expect("hydrolysis menu presentation animation is a curve or a spring")
            .sample_lanes(&[from], &[velocity], &[target], dt, &mut position, &mut vel);
        (position[0], vel[0], done)
    }

    fn eval_at(&self, at: Instant) -> ([f64; 2], [f64; 2], bool) {
        if self.settled {
            return ([self.target, self.target], [0.0, 0.0], true);
        }
        let dt = at.saturating_duration_since(self.start).as_secs_f64();
        let (px, vx, done_x) = Self::eval_axis(
            &self.animation_x,
            self.from[0],
            self.velocity[0],
            self.target,
            dt,
        );
        let (py, vy, done_y) = Self::eval_axis(
            &self.animation_y,
            self.from[1],
            self.velocity[1],
            self.target,
            dt,
        );
        ([px, py], [vx, vy], done_x && done_y)
    }

    fn advance(&mut self, now: Instant) -> ([f64; 2], bool) {
        let (progress, _, done) = self.eval_at(now);
        if done {
            self.settled = true;
            self.from = [self.target, self.target];
            self.velocity = [0.0, 0.0];
            ([self.target, self.target], true)
        } else {
            (progress, false)
        }
    }

    fn dismiss(&mut self, now: Instant) {
        let (progress, velocity, _) = self.eval_at(now);
        self.from = progress;
        self.velocity = velocity;
        self.target = 0.0;
        self.animation_x = engine_animation(&self.motion.dismiss);
        self.animation_y = engine_animation(
            self.motion
                .dismiss_vertical
                .as_ref()
                .unwrap_or(&self.motion.dismiss),
        );
        self.start = now;
        self.dismissing = true;
        self.settled = false;
    }

    fn sample_at(&self, origin: Rect, panel: Rect, now: Instant) -> MenuPresentationSample {
        let ([px, py], _, _) = self.eval_at(now);
        self.motion
            .sample_axes(origin, panel, px, py, self.dismissing)
    }

    fn trigger_opacity_at(&self, now: Instant) -> f64 {
        let ([px, py], _, _) = self.eval_at(now);
        let q = px.clamp(0.0, 1.0).min(py.clamp(0.0, 1.0));
        self.motion.trigger_opacity_for(q, self.dismissing)
    }

    fn trigger_chrome_opacity_at(&self, now: Instant) -> f64 {
        let ([px, py], _, _) = self.eval_at(now);
        let q = px.clamp(0.0, 1.0).min(py.clamp(0.0, 1.0));
        self.motion.trigger_chrome_opacity_for(q, self.dismissing)
    }
}

/// The presentations of one anchored overlay: the one presented now and the
/// dismissed ones still leaving. Each presentation is an independent
/// instance — a re-present while an exit is in flight builds a new one and
/// lets the old one finish its exit beside it.
pub struct AnchoredPresentations<E: crate::text::TextEngine> {
    current: Option<AnchoredPresentation<E>>,
    exiting: Vec<AnchoredPresentation<E>>,
}

#[cfg(feature = "menu")]
impl<E: TextEngine> AnchoredPresentations<E> {
    /// Whether an active (non-exiting) presentation is currently built.
    pub(crate) const fn has_current(&self) -> bool {
        self.current.is_some()
    }

    /// Whether a presentation is currently open or still running its dismiss
    /// animation.
    pub(crate) const fn has_presentations(&self) -> bool {
        self.current.is_some() || !self.exiting.is_empty()
    }

    /// Minimum trigger opacity across all active and exiting morphing
    /// presentations at `now`, or `1.0` when no presentation fades the trigger.
    pub(crate) fn trigger_opacity_at(&self, now: Instant) -> f64 {
        self.trigger_opacities_at(now).0
    }

    /// Minimum `(label_opacity, chrome_opacity)` for the triggering anchor
    /// across all active and exiting morphing presentations at `now`, or
    /// `(1.0, 1.0)` when no morphing presentation is active.
    pub(crate) fn trigger_opacities_at(&self, now: Instant) -> (f64, f64) {
        self.current
            .iter()
            .chain(&self.exiting)
            .filter_map(|presentation| presentation.morph.as_ref())
            .fold((1.0_f64, 1.0_f64), |(label, chrome), morph| {
                (
                    label.min(morph.trigger_opacity_at(now)),
                    chrome.min(morph.trigger_chrome_opacity_at(now)),
                )
            })
    }
}

/// One presentation of an anchored overlay: its content instance and the
/// transition the content's root declares (layout-spec.md §4.5), or the
/// theme's [`MenuPresentationMotion`] when no explicit transition was declared.
pub struct AnchoredPresentation<E: crate::text::TextEngine> {
    content: RetainedSubview<E>,
    /// The identity of this presentation's animation and paint scopes.
    marker: Rc<()>,
    /// The declaration claimed from the content's root when it was built.
    spec: Option<TransitionSpec>,
    /// The insertion (presented) or removal (exiting) running now.
    run: Option<TransitionRun<E>>,
    /// Theme-driven presentation morph progress when `spec` is `None` and the
    /// theme's `MenuLook` declares [`MenuPresentationMotion`].
    #[cfg(feature = "menu")]
    morph: Option<PresentationMorph>,
    /// Lazily allocated blur filter host used when `MenuPresentationSample::content_blur > 0`.
    #[cfg(feature = "menu")]
    blur: Rc<RefCell<Option<TransitionBlur<E>>>>,
    /// The frame and proposal the presentation was last placed at — the
    /// slot an exiting presentation keeps.
    placed: Option<(kurbo::Rect, ProposalSize)>,
}

impl<E: crate::text::TextEngine> AnchoredPresentation<E> {
    /// A new presentation of `entry`: a fresh content instance, built inside
    /// its own animation scope, whose root declaration starts the insertion.
    fn present(
        renderer: &mut HydrolysisRenderer<E>,
        entry: &RegisteredAnchoredOverlay<E>,
        reduce_motion: bool,
    ) -> Self {
        let marker = Rc::new(());
        let mut content = RetainedSubview::held((entry.builder)());
        renderer
            .animation_controller
            .begin_animation_scope(scope_of(&marker));
        content.ensure_built(renderer, &entry.env);
        renderer.animation_controller.end_animation_scope();
        let spec = claim_transition(
            content
                .node()
                .expect("hydrolysis anchored overlay: the content was just built"),
        );
        let run = spec
            .as_ref()
            .map(|spec| TransitionRun::insertion(spec, reduce_motion, renderer.frame_instant));
        #[cfg(feature = "menu")]
        let morph = if spec.is_none() {
            menu_look(&renderer.theme(), &entry.env)
                .get_presentation_motion()
                .cloned()
                .map(|motion| {
                    PresentationMorph::present(motion, renderer.frame_instant, reduce_motion)
                })
        } else {
            None
        };
        Self {
            content,
            marker,
            spec,
            run,
            #[cfg(feature = "menu")]
            morph,
            #[cfg(feature = "menu")]
            blur: Rc::new(RefCell::new(None)),
            placed: None,
        }
    }

    /// The animation-controller scope the content's slots are attributed to.
    fn scope(&self) -> u64 {
        scope_of(&self.marker)
    }
}

/// The animation-controller scope a presentation marker identifies.
#[cfg(feature = "menu")]
pub fn anchored_overlay_scope(marker: &Rc<()>) -> u64 {
    scope_of(marker)
}

/// The animation-controller scope a presentation marker identifies.
fn scope_of(marker: &Rc<()>) -> u64 {
    u64::try_from(Rc::as_ptr(marker).addr())
        .expect("hydrolysis anchored overlay: a marker address fits u64")
}

/// The per-frame state the render pass leaves behind for the input path: each
/// drawn presentation's frame, dismissal mode and binding. An anchor the
/// next frame's flush does not re-register left the tree — its overlay
/// closes.
pub struct PresentedAnchoredOverlay<E: crate::text::TextEngine> {
    /// Identity matching [`RegisteredAnchoredOverlay::marker`].
    pub(crate) marker: Rc<()>,
    /// The overlay's presentations, cleared when its anchor leaves the tree.
    pub(crate) presentations: Rc<RefCell<AnchoredPresentations<E>>>,
    /// The frame the presentation drew into, in hit space.
    pub(crate) frame: kurbo::Rect,
    /// How the overlay closes.
    pub(crate) dismissal: Dismissal,
    /// The binding outside interaction writes `false` to.
    pub(crate) is_presented: Binding<bool>,
    /// Whether this presentation was dismissed: it is leaving — still drawn,
    /// but ignored by hit testing and outside-dismissal until its removal
    /// transition, or the animations its content runs, finish.
    pub(crate) exiting: bool,
}

impl<E: crate::text::TextEngine> SemanticCore<E> {
    /// Outside-interaction dismissal for anchored overlays: a press outside a
    /// presented `OutsideInteraction` overlay writes `false` to its binding.
    /// The press itself continues to its target, exactly like the
    /// context-menu outside dismissal in `handle_pointer_down_with_source`.
    /// `Manual` overlays are never closed here.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(crate) fn dismiss_anchored_overlays_outside(&mut self, point: kurbo::Point) {
        for overlay in &self.popup_menu.presented_anchored_overlays {
            if !overlay.exiting
                && overlay.dismissal == Dismissal::OutsideInteraction
                && !overlay.frame.contains(point)
            {
                overlay.is_presented.set(false);
            }
        }
    }

    /// The drawn frames of every presented anchored overlay, in hit order —
    /// the test harness reads these to assert the placement contract.
    #[cfg(test)]
    pub(crate) fn anchored_overlay_frames(&self) -> Vec<kurbo::Rect> {
        self.popup_menu
            .presented_anchored_overlays
            .iter()
            .map(|overlay| overlay.frame)
            .collect()
    }
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// Draw every anchored overlay whose binding reads `true` this frame,
    /// above all content, at the placement contract's frame, and every
    /// dismissed presentation still leaving. Called after the tree flush: the
    /// registrations it collected during the flush carry the anchors' live
    /// bounds, so the placement follows moves and resizes.
    pub(crate) fn render_anchored_overlays(
        &mut self,
        transform: kurbo::Affine,
        safe_area: &crate::renderer::SafeAreaLayout,
    ) {
        // The overlay pass runs after the tree flush returns, so no
        // `.material_group()` scope the anchors' ancestry opened can
        // reach overlay content: the stack is empty by construction.
        assert!(
            self.material_group_scopes.is_empty(),
            "hydrolysis renderer: the anchored-overlay pass must start \
             with an empty material-scope stack — the tree flush leaves \
             no scope open"
        );
        let registered = core::mem::take(&mut self.popup_menu.anchored_overlays);
        let last_presented = core::mem::take(&mut self.popup_menu.presented_anchored_overlays);
        let mut presented = Vec::with_capacity(last_presented.len());

        // An anchor that drew last frame but registered nothing this frame
        // left the tree: close it through its binding and drop every
        // presentation with it — the contract closes the overlay with the
        // anchor.
        for overlay in &last_presented {
            if !registered
                .iter()
                .any(|entry| Rc::ptr_eq(&entry.marker, &overlay.marker))
            {
                overlay.is_presented.set(false);
                let state = core::mem::take(&mut *overlay.presentations.borrow_mut());
                for presentation in state.current.iter().chain(&state.exiting) {
                    self.animation_controller
                        .drop_animation_scope(presentation.scope());
                }
            }
        }

        let ctx = RenderContext::new(transform, self.window_bounds);
        for entry in registered {
            let mut state = core::mem::take(&mut *entry.presentations.borrow_mut());
            let reduce_motion = ReduceMotion::is_reduced(&entry.env);
            if entry.presented {
                // A new presentation builds fresh content: still-presented
                // keeps its subtree — signals already patch it in place —
                // while a first open or a re-present, including one that
                // overtakes an exit in flight, starts an independent
                // instance.
                if state.current.is_none() {
                    state.current =
                        Some(AnchoredPresentation::present(self, &entry, reduce_motion));
                }
            } else if let Some(mut current) = state.current.take() {
                // Dismissed: a presentation that drew starts leaving — under
                // its removal transition when its content declares one, under
                // the theme's dismiss animation when `MenuPresentationMotion`
                // drives it, or for as long as the animations its content runs.
                if current.placed.is_some() {
                    current.run = current.spec.as_ref().map(|spec| {
                        TransitionRun::removal(spec, reduce_motion, self.frame_instant)
                    });
                    #[cfg(feature = "menu")]
                    let keep_exiting = if current.run.is_none()
                        && let Some(morph) = &mut current.morph
                    {
                        if reduce_motion {
                            false
                        } else {
                            morph.dismiss(self.frame_instant);
                            true
                        }
                    } else {
                        true
                    };
                    #[cfg(not(feature = "menu"))]
                    let keep_exiting = true;
                    if keep_exiting {
                        state.exiting.push(current);
                    } else {
                        self.animation_controller
                            .drop_animation_scope(current.scope());
                        if let Some(node) = current.content.node() {
                            self.retire_subtrees(vec![Rc::clone(&node.core().cell)]);
                        }
                    }
                } else {
                    self.animation_controller
                        .drop_animation_scope(current.scope());
                }
            }
            // Exiting presentations draw beneath the presented one.
            let mut remaining = Vec::with_capacity(state.exiting.len());
            for mut presentation in state.exiting.drain(..) {
                if let Some(frame) =
                    self.render_exiting_presentation(&entry, &mut presentation, ctx, safe_area)
                {
                    presented.push(PresentedAnchoredOverlay {
                        marker: entry.marker.clone(),
                        presentations: Rc::clone(&entry.presentations),
                        frame,
                        dismissal: entry.dismissal,
                        is_presented: entry.is_presented.clone(),
                        exiting: true,
                    });
                    remaining.push(presentation);
                }
            }
            state.exiting = remaining;
            if let Some(current) = &mut state.current {
                let frame = self.render_presented(&entry, current, ctx, safe_area);
                presented.push(PresentedAnchoredOverlay {
                    marker: entry.marker.clone(),
                    presentations: Rc::clone(&entry.presentations),
                    frame,
                    dismissal: entry.dismissal,
                    is_presented: entry.is_presented.clone(),
                    exiting: false,
                });
            }
            *entry.presentations.borrow_mut() = state;
        }
        self.popup_menu.presented_anchored_overlays = presented;
    }

    /// Measures, places and draws the presented presentation — under its
    /// insertion while one runs — and returns the frame it drew into.
    fn render_presented(
        &mut self,
        entry: &RegisteredAnchoredOverlay<E>,
        presentation: &mut AnchoredPresentation<E>,
        ctx: RenderContext,
        safe_area: &crate::renderer::SafeAreaLayout,
    ) -> kurbo::Rect {
        let window = self.window_bounds;
        // Measure at the ideal size with the window as the upper bound:
        // propose at most the window inset by the clamp margins, so long
        // content wraps inside the frame it will be clamped into instead
        // of laying out unbounded and overflowing it.
        let margin = match entry.placement.get_clamp() {
            Clamp::Window { margin } => f64::from(margin) * 2.0,
            Clamp::Off => 0.0,
        };
        let proposal = ProposalSize::new(
            Some(crate::num_cast::f64_as_f32(
                (window.width() - margin).max(0.0),
            )),
            Some(crate::num_cast::f64_as_f32(
                (window.height() - margin).max(0.0),
            )),
        );
        let scope = presentation.scope();
        self.animation_controller.begin_animation_scope(scope);
        let (ideal, _stretch) = presentation
            .content
            .patch_and_measure(self, &entry.env, proposal);
        self.animation_controller.end_animation_scope();
        if let Some(run) = &mut presentation.run
            && !run.advance(self.frame_instant)
        {
            presentation.run = None;
        }
        #[cfg(feature = "menu")]
        let morph_active = presentation
            .morph
            .as_mut()
            .is_some_and(|morph| !morph.advance(self.frame_instant).1);
        #[cfg(not(feature = "menu"))]
        let morph_active = false;
        let overlay_size = waterui_core::layout::Size::new(
            ideal.width.min(crate::num_cast::f64_as_f32(window.width())),
            ideal
                .height
                .min(crate::num_cast::f64_as_f32(window.height())),
        );
        let direction = self.read_signal(&layout_direction(&entry.env));
        let (frame, logical) = if entry.cover_trigger {
            let clamp_margin = match entry.placement.get_clamp() {
                Clamp::Window { margin } => f64::from(margin),
                Clamp::Off => 0.0,
            };
            place_covering_trigger(entry.anchor, window, overlay_size, clamp_margin)
        } else {
            let placement = place_anchored_overlay(
                kurbo_to_rect(entry.anchor),
                kurbo_to_rect(window),
                overlay_size,
                entry.placement,
                direction,
            );
            (
                rect_to_kurbo(placement.frame()),
                logical_edge(placement.edge(), direction),
            )
        };

        // Every placement writes back the logical edge the overlay was
        // placed against — the physical edge resolved under the layout
        // direction, so `Leading`/`Trailing` read back correctly in RTL.
        if entry.placed_edge.snapshot() != logical {
            entry.placed_edge.set(logical);
        }

        // A press inside the overlay belongs to its own content, not the
        // page below — the same occlusion the drawn context-menu panels
        // register, so gesture regions under the frame do not arm either.
        self.register_hit_test_occluder(frame);
        presentation.placed = Some((frame, bounded_proposal(frame)));
        self.record_presentation(entry, presentation, ctx, safe_area, true);
        if presentation.run.is_some() || morph_active {
            self.context_mark_layout();
        }
        frame
    }

    /// Draws a dismissed presentation in the slot it was last placed at,
    /// inert to input, and returns that frame while it is still leaving —
    /// `None` once its removal transition, or the animations its content
    /// runs, finished and it left.
    fn render_exiting_presentation(
        &mut self,
        entry: &RegisteredAnchoredOverlay<E>,
        presentation: &mut AnchoredPresentation<E>,
        ctx: RenderContext,
        safe_area: &crate::renderer::SafeAreaLayout,
    ) -> Option<kurbo::Rect> {
        let (frame, _) = presentation
            .placed
            .expect("hydrolysis anchored overlay: an exiting presentation was placed");
        let scope = presentation.scope();
        if let Some(run) = &mut presentation.run {
            // A removal transition: the presentation already left the
            // semantic tree and paints until its own clock completes.
            if !run.advance(self.frame_instant) {
                self.animation_controller.drop_animation_scope(scope);
                if let Some(node) = presentation.content.node() {
                    self.retire_subtrees(vec![Rc::clone(&node.core().cell)]);
                }
                return None;
            }
            self.record_presentation(entry, presentation, ctx, safe_area, false);
            self.context_mark_layout();
            return Some(frame);
        }
        #[cfg(feature = "menu")]
        if let Some(morph) = &mut presentation.morph {
            // A theme-driven dismiss morph: the presentation stays drawn as an
            // inert, accessibility-suppressed ghost until the dismiss spring
            // or curve settles at `0.0`.
            if morph.advance(self.frame_instant).1 {
                self.animation_controller.drop_animation_scope(scope);
                if let Some(node) = presentation.content.node() {
                    self.retire_subtrees(vec![Rc::clone(&node.core().cell)]);
                }
                return None;
            }
            self.record_presentation(entry, presentation, ctx, safe_area, false);
            self.context_mark_layout();
            return Some(frame);
        }
        // No removal transition: a slot binds only when its node records —
        // an animation the content started in response to the dismissal does
        // not exist in the controller until this frame's record. So the
        // exiting presentation records into its scope, inert to input, and
        // only then decides: a scope with a running slot stays drawn; one
        // with nothing left animating is dropped and the content unmounts.
        self.record_presentation(entry, presentation, ctx, safe_area, false);
        if self
            .animation_controller
            .scope_is_active(scope, self.frame_instant)
        {
            // The engine runs the content's wrapper motions without a host
            // record per frame; their end re-records the content, and this
            // decision runs again then.
            self.animation_controller
                .arm_scope_settle(scope, self.frame_instant);
            return Some(frame);
        }
        self.program().discard_last_scope();
        self.animation_controller.drop_animation_scope(scope);
        // The scope left the program: retire the content's subtree now — the
        // same retire an unplaced sub-view's gets — instead of leaving its
        // layers alive and detached until the next host record.
        if let Some(node) = presentation.content.node() {
            self.retire_subtrees(vec![Rc::clone(&node.core().cell)]);
        }
        None
    }

    /// Records one presentation at its placed frame inside its own keyed
    /// scope and animation scope, under its running transition when it has
    /// one. `hit: false` (a dismissed presentation) keeps it painted while
    /// nothing inside takes input; a presentation leaving under a removal
    /// transition is also out of the accessibility tree.
    fn record_presentation(
        &mut self,
        entry: &RegisteredAnchoredOverlay<E>,
        presentation: &mut AnchoredPresentation<E>,
        ctx: RenderContext,
        safe_area: &crate::renderer::SafeAreaLayout,
        hit: bool,
    ) {
        let (frame, proposal) = presentation
            .placed
            .expect("hydrolysis anchored overlay: a recorded presentation was placed");
        let scope = presentation.scope();
        let key = crate::renderer::mount::ScopeKey {
            role: "anchored",
            item: scope,
        };
        self.open_scope_with(
            key,
            kurbo::Affine::IDENTITY,
            1.0,
            None,
            kurbo::Affine::IDENTITY,
            crate::renderer::ScopeDelta::RECORD_SPACE,
            hit,
        );
        // Record inside the presentation's animation scope so every slot the
        // content binds is attributed to it — that attribution is what the
        // exit without a removal transition waits on.
        self.animation_controller.begin_animation_scope(scope);
        // §7.1: the overlay is chrome of its own placement — content inside
        // inherits the window's boundaries, so it can only touch and release
        // where its frame really sits on one.
        let area = safe_area.with_frame(frame);
        let AnchoredPresentation {
            content,
            run,
            #[cfg(feature = "menu")]
            morph,
            #[cfg(feature = "menu")]
            blur,
            ..
        } = presentation;
        match run {
            Some(run) => {
                let delta = kurbo::Affine::translate((frame.x0, frame.y0));
                let slot = kurbo::Rect::new(0.0, 0.0, frame.width(), frame.height());
                let direction = self.read_signal(&layout_direction(&entry.env));
                let root = Rc::clone(self.root_cell());
                let paint = TransitionPaint::sample(
                    &run.state,
                    waterui_core::layout::Size::new(
                        crate::num_cast::f64_as_f32(frame.width()),
                        crate::num_cast::f64_as_f32(frame.height()),
                    ),
                    direction,
                    hit,
                    &run.runtime,
                    &root,
                );
                let record = |renderer: &mut Self| {
                    flush_transitioned(
                        renderer,
                        ctx.child(delta, slot),
                        delta,
                        &paint,
                        |renderer, inner| {
                            content.place_detached(
                                renderer,
                                inner,
                                &entry.env,
                                proposal,
                                slot,
                                Some(area),
                            );
                        },
                    );
                };
                if hit {
                    record(self);
                } else {
                    self.with_suppressed_accessibility(record);
                }
            }
            None => {
                #[cfg(feature = "menu")]
                if let Some(morph) = morph {
                    let sample = morph.sample_at(entry.anchor, frame, self.frame_instant);
                    self.record_morphed_presentation(
                        entry,
                        content,
                        sample,
                        scope,
                        Rc::clone(blur),
                        ctx,
                        proposal,
                        frame,
                        area,
                        hit,
                    );
                } else {
                    content.place_detached(self, ctx, &entry.env, proposal, frame, Some(area));
                }
                #[cfg(not(feature = "menu"))]
                content.place_detached(self, ctx, &entry.env, proposal, frame, Some(area));
            }
        }
        self.animation_controller.end_animation_scope();
        self.close_scope();
    }

    /// Records a presentation driven by `MenuPresentationMotion`: when the
    /// subtree owns a [`PopupMenuSurface`], hands `sample` to
    /// `apply_popup_menu_surface` so the theme draws the morphing surface and
    /// reveals its rows inside it; otherwise applies the content scale and
    /// opacity morph directly around the placed overlay subtree.
    #[cfg(feature = "menu")]
    #[expect(
        clippy::too_many_arguments,
        reason = "forwards the presentation's layout, sample, scope, blur host, and hit-test context directly"
    )]
    fn record_morphed_presentation(
        &mut self,
        entry: &RegisteredAnchoredOverlay<E>,
        content: &mut RetainedSubview<E>,
        sample: MenuPresentationSample,
        scope: u64,
        blur: Rc<RefCell<Option<TransitionBlur<E>>>>,
        ctx: RenderContext,
        proposal: ProposalSize,
        frame: Rect,
        area: SafeAreaLayout,
        hit: bool,
    ) {
        let uses_menu_surface = content
            .node()
            .is_some_and(RenderNode::has_popup_menu_surface);
        if uses_menu_surface {
            let owner = Rc::clone(self.root_cell());
            let previous =
                self.popup_menu
                    .active_menu_presentation
                    .replace(ActiveMenuPresentation {
                        sample,
                        scope,
                        blur,
                        owner,
                    });
            let record = |renderer: &mut Self| {
                content.place_detached(renderer, ctx, &entry.env, proposal, frame, Some(area));
            };
            if hit {
                record(self);
            } else {
                self.with_suppressed_accessibility(record);
            }
            self.popup_menu.active_menu_presentation = previous;
            return;
        }

        let at_rest = (sample.progress - 1.0).abs() <= 1e-4
            && (sample.vertical_progress - 1.0).abs() <= 1e-4
            && (sample.content_opacity - 1.0).abs() <= 1e-4
            && (sample.content_scale - 1.0).abs() <= 1e-4
            && sample.content_blur <= 0.01;
        if at_rest {
            let record = |renderer: &mut Self| {
                content.place_detached(renderer, ctx, &entry.env, proposal, frame, Some(area));
            };
            if hit {
                record(self);
            } else {
                self.with_suppressed_accessibility(record);
            }
            return;
        }

        let delta = Affine::translate((frame.x0, frame.y0));
        let slot = Rect::new(0.0, 0.0, frame.width(), frame.height());
        let local_surface = sample.surface + Vec2::new(-frame.x0, -frame.y0);
        let local = presentation_content_transform(slot, local_surface, sample.content_scale);
        let child_ctx = ctx.child(delta, slot);
        let paint_space = child_ctx.local() * local;
        let layer = self.program().origin().inverse() * paint_space;
        let opacity = f64_as_f32(sample.content_opacity);
        let key = ScopeKey {
            role: "menu_presentation",
            item: scope,
        };
        let record = |renderer: &mut Self| {
            renderer.open_scope_with(
                key,
                layer,
                opacity,
                None,
                paint_space,
                ScopeDelta {
                    transform: delta * local,
                    hit_alpha: opacity,
                },
                hit,
            );
            content.place_detached(renderer, child_ctx, &entry.env, proposal, slot, Some(area));
            renderer.close_scope();
        };
        if hit {
            record(self);
        } else {
            self.with_suppressed_accessibility(record);
        }
    }
}

/// Places a menu panel overlapping `anchor`: top-aligned with `anchor.y0` and
/// horizontally centred on `anchor`, flipping to bottom-aligned (`anchor.y1`)
/// when the panel overflows the bottom margin and fits above, then clamping
/// inside `window` inset by `margin`.
fn place_covering_trigger(
    anchor: Rect,
    window: Rect,
    overlay: LayoutSize,
    margin: f64,
) -> (Rect, AnchorEdge) {
    let width = f64::from(overlay.width).min(window.width());
    let height = f64::from(overlay.height).min(window.height());
    let min_x = window.x0 + margin;
    let max_x = (window.x1 - margin - width).max(min_x);
    let x = width.mul_add(-0.5, anchor.center().x).clamp(min_x, max_x);

    let min_y = window.y0 + margin;
    let max_y = (window.y1 - margin - height).max(min_y);
    let (y, edge) = if anchor.y0 + height <= window.y1 - margin {
        (anchor.y0.clamp(min_y, max_y), AnchorEdge::Bottom)
    } else if anchor.y1 - height >= min_y {
        ((anchor.y1 - height).clamp(min_y, max_y), AnchorEdge::Top)
    } else {
        (anchor.y0.clamp(min_y, max_y), AnchorEdge::Bottom)
    };
    (Rect::new(x, y, x + width, y + height), edge)
}

const fn kurbo_to_rect(rect: kurbo::Rect) -> waterui_core::layout::Rect {
    waterui_core::layout::Rect::new(
        waterui_core::layout::Point::new(
            crate::num_cast::f64_as_f32(rect.x0),
            crate::num_cast::f64_as_f32(rect.y0),
        ),
        waterui_core::layout::Size::new(
            crate::num_cast::f64_as_f32(rect.width()),
            crate::num_cast::f64_as_f32(rect.height()),
        ),
    )
}

fn rect_to_kurbo(rect: waterui_core::layout::Rect) -> kurbo::Rect {
    kurbo::Rect::from_origin_size(
        kurbo::Point::new(f64::from(rect.x()), f64::from(rect.y())),
        kurbo::Size::new(f64::from(rect.width()), f64::from(rect.height())),
    )
}

impl<E: crate::text::TextEngine> Default for AnchoredPresentations<E> {
    fn default() -> Self {
        Self {
            current: Option::default(),
            exiting: Vec::default(),
        }
    }
}
