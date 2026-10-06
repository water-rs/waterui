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

use std::cell::RefCell;
use std::rc::Rc;

use nami::Binding;
use waterui::metadata::anchored_overlay::{AnchorEdge, AnchorPlacement, Clamp, Dismissal};
use waterui_core::Environment;
use waterui_core::layout::layout_direction;

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

/// What an anchor's flush registered: the anchor frame in hit space plus the
/// handles the post-flush render pass needs. The content slot lives on the
/// effect (`Rc`-shared) so the overlay's built node survives both closes and
/// the anchor's own reconcile.
pub struct RegisteredAnchoredOverlay {
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
    /// The retained content slot the owning effect keeps across frames.
    pub(crate) content: Rc<RefCell<Option<RetainedSubview>>>,
    /// Identity shared between the effect and its registration, so the render
    /// pass can tell a re-registered anchor from a dropped one.
    pub(crate) marker: Rc<()>,
}

/// The per-frame state the render pass leaves behind for the input path: each
/// presented overlay's drawn frame, dismissal mode and binding. An entry the
/// next frame's flush does not re-register is an anchor that left the tree —
/// its overlay closes.
pub struct PresentedAnchoredOverlay {
    /// Identity matching [`RegisteredAnchoredOverlay::marker`].
    pub(crate) marker: Rc<()>,
    /// The frame the overlay drew into, in hit space.
    pub(crate) frame: kurbo::Rect,
    /// How the overlay closes.
    pub(crate) dismissal: Dismissal,
    /// The binding outside interaction writes `false` to.
    pub(crate) is_presented: Binding<bool>,
    /// Whether `is_presented` already read `false`: the overlay is leaving —
    /// still drawn, but ignored by hit testing and outside-dismissal until
    /// the content's running animations finish.
    pub(crate) exiting: bool,
    /// The animation-controller scope the subtree's slots are attributed to —
    /// the retained content's marker pointer.
    pub(crate) scope: u64,
}

impl SemanticCore {
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

impl HydrolysisRenderer {
    /// Draw every anchored overlay whose binding reads `true` this frame,
    /// above all content, at the placement contract's frame. Called after the
    /// tree flush: the registrations it collected during the flush carry the
    /// anchors' live bounds, so the placement follows moves and resizes.
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(crate) fn render_anchored_overlays(
        &mut self,
        transform: kurbo::Affine,
        safe_area: &crate::renderer::SafeAreaLayout,
    ) {
        // The overlay pass runs after the tree flush returns, so no
        // `.material_group()` scope the anchors' ancestry opened can
        // reach overlay content: the stack is empty by construction.
        assert!(
            self.compositor.material_scopes.is_empty(),
            "hydrolysis renderer: the anchored-overlay pass must start \
             with an empty material-scope stack — the tree flush leaves \
             no scope open"
        );
        let registered = core::mem::take(&mut self.popup_menu.anchored_overlays);
        let last_presented = core::mem::take(&mut self.popup_menu.presented_anchored_overlays);
        let mut presented = Vec::with_capacity(last_presented.len());

        // An anchor that drew last frame but registered nothing this frame
        // left the tree: close it through its binding and drop an exit still
        // in flight with it — the contract closes the overlay with the anchor.
        for overlay in &last_presented {
            if !registered
                .iter()
                .any(|entry| Rc::ptr_eq(&entry.marker, &overlay.marker))
            {
                overlay.is_presented.set(false);
                self.animation_controller
                    .drop_animation_scope(overlay.scope);
            }
        }

        let window = self.window_bounds;
        let now = self.frame_instant;
        for entry in registered {
            let scope = Rc::as_ptr(&entry.marker) as usize as u64;
            // `is_presented` turned false on an overlay that drew last frame:
            // the exit lifecycle. The content stays on screen — ignored by hit
            // testing and outside dismissal — until the animations it started
            // inside its subtree finish, then it leaves at once.
            let exiting = !entry.presented;
            if exiting {
                let drew_last_frame = last_presented
                    .iter()
                    .any(|overlay| Rc::ptr_eq(&overlay.marker, &entry.marker));
                if !drew_last_frame || entry.content.borrow().is_none() {
                    continue;
                }
            }
            let Some(mut content) = entry.content.borrow_mut().take() else {
                continue;
            };
            // Measure at the ideal size with the window as the upper bound:
            // propose at most the window inset by the clamp margins, so long
            // content wraps inside the frame it will be clamped into instead
            // of laying out unbounded and overflowing it.
            let margin = match entry.placement.clamp {
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
            self.animation_controller.begin_animation_scope(scope);
            let (ideal, _stretch) = content.patch_and_measure(self, &entry.env, proposal);
            self.animation_controller.end_animation_scope();
            let overlay_size = waterui_core::layout::Size::new(
                ideal.width.min(crate::num_cast::f64_as_f32(window.width())),
                ideal
                    .height
                    .min(crate::num_cast::f64_as_f32(window.height())),
            );
            let direction = self.read_signal(&layout_direction(&entry.env));
            let placement = waterui_backend_core::overlay::place_anchored_overlay(
                kurbo_to_rect(entry.anchor),
                kurbo_to_rect(window),
                overlay_size,
                entry.placement,
                direction,
            );
            let frame = rect_to_kurbo(placement.frame);

            // Every placement writes back the logical edge the overlay was
            // placed against — the physical edge resolved under the layout
            // direction, so `Leading`/`Trailing` read back correctly in RTL.
            let logical = waterui_backend_core::overlay::logical_edge(placement.edge, direction);
            if entry.placed_edge.snapshot() != logical {
                entry.placed_edge.set(logical);
            }

            // Hit regions clip to the open paint clip stack; the clipped
            // frame feeds the pointer target, the flush and the recorded
            // frame alike.
            let frame = self.hit_test.clip_hit_bounds(frame);

            if !exiting {
                // A press inside the overlay belongs to its own content, not
                // the page below — the same occlusion the drawn context-menu
                // panels register, so gesture regions under the frame do not
                // arm either. An exiting overlay registers none, so its frame
                // stops intercepting input the moment it starts leaving.
                self.register_hit_test_occluder(frame);
            }

            // A slot binds only when its node encodes — an animation the
            // content started in response to the dismissal does not exist in
            // the controller until this frame's flush. So an exiting overlay
            // swaps the window scene for a scratch scene, flushes into it
            // with hit testing suppressed, and only then decides: a scope
            // with a running slot commits the frame and keeps the overlay
            // presented; one with nothing left animating discards it — the
            // overlay draws no frame past its last animation.
            let scratch = exiting.then(|| {
                let hit_test_opacity = self.hit_test.hit_test_opacity;
                self.hit_test.hit_test_opacity = 0.0;
                (
                    hit_test_opacity,
                    core::mem::take(&mut self.scene),
                    core::mem::take(&mut self.compositor.render_layers),
                    core::mem::take(&mut self.compositor.active_scene_layers),
                    self.transient_scene.take(),
                )
            });

            // Flush inside the overlay's animation scope so every slot the
            // content binds is attributed to it — that attribution is what
            // the exit lifecycle waits on.
            self.animation_controller.begin_animation_scope(scope);
            content.flush_in_rect(
                self,
                RenderContext::with_transforms(window, transform, kurbo::Affine::IDENTITY),
                &entry.env,
                bounded_proposal(frame),
                frame,
                // §7.1: the overlay is chrome of its own placement — content
                // inside inherits the window's boundaries, so it can only
                // touch and release where its frame really sits on one.
                Some(safe_area.with_frame(frame)),
            );
            self.animation_controller.end_animation_scope();

            if let Some((
                hit_test_opacity,
                parent_scene,
                parent_render_layers,
                parent_active_layers,
                parent_transient_scene,
            )) = scratch
            {
                self.hit_test.hit_test_opacity = hit_test_opacity;
                let overlay_scene = core::mem::replace(&mut self.scene, parent_scene);
                let overlay_render_layers =
                    core::mem::replace(&mut self.compositor.render_layers, parent_render_layers);
                debug_assert!(
                    self.compositor.active_scene_layers.is_empty(),
                    "hydrolysis anchored overlay exit left an unclosed scene layer"
                );
                self.compositor.active_scene_layers = parent_active_layers;
                let overlay_transient_scene = self.transient_scene.take();
                self.transient_scene = parent_transient_scene;

                if !self.animation_controller.scope_is_active(scope, now) {
                    self.animation_controller.drop_animation_scope(scope);
                    *entry.content.borrow_mut() = Some(content);
                    continue;
                }
                self.scene.append(&overlay_scene, kurbo::Affine::IDENTITY);
                self.compositor.render_layers.extend(overlay_render_layers);
                if let Some(transient) = overlay_transient_scene {
                    match &mut self.transient_scene {
                        Some(parent) => {
                            parent.append(&transient, kurbo::Affine::IDENTITY);
                        }
                        None => self.transient_scene = Some(transient),
                    }
                }
            }
            *entry.content.borrow_mut() = Some(content);

            presented.push(PresentedAnchoredOverlay {
                marker: entry.marker.clone(),
                frame,
                dismissal: entry.dismissal,
                is_presented: entry.is_presented.clone(),
                exiting,
                scope,
            });
        }
        self.popup_menu.presented_anchored_overlays = presented;
    }
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
