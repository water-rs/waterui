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
use waterui::metadata::anchored_overlay::{AnchorPlacement, Clamp, Dismissal};
use waterui_core::Environment;
use waterui_core::layout::layout_direction;

use super::*;

/// What an anchor's flush registered: the anchor frame in hit space plus the
/// handles the post-flush render pass needs. The content slot lives on the
/// effect (`Rc`-shared) so the overlay's built node survives both closes and
/// the anchor's own reconcile.
pub(crate) struct RegisteredAnchoredOverlay {
    /// The anchor's frame in hit space this frame.
    pub(crate) anchor: vello::kurbo::Rect,
    /// The placement contract from the metadata.
    pub(crate) placement: AnchorPlacement,
    /// How the overlay closes.
    pub(crate) dismissal: Dismissal,
    /// Whether the overlay is presented right now.
    pub(crate) presented: bool,
    /// The binding the renderer writes `false` to on outside dismissal, and
    /// the anchor's environment the overlay content is built and flushed in.
    pub(crate) is_presented: Binding<bool>,
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
pub(crate) struct PresentedAnchoredOverlay {
    /// Identity matching [`RegisteredAnchoredOverlay::marker`].
    pub(crate) marker: Rc<()>,
    /// The frame the overlay drew into, in hit space.
    pub(crate) frame: vello::kurbo::Rect,
    /// How the overlay closes.
    pub(crate) dismissal: Dismissal,
    /// The binding outside interaction writes `false` to.
    pub(crate) is_presented: Binding<bool>,
}

impl SemanticCore {
    /// Outside-interaction dismissal for anchored overlays: a press outside a
    /// presented `OutsideInteraction` overlay writes `false` to its binding.
    /// The press itself continues to its target, exactly like the
    /// context-menu outside dismissal in `handle_pointer_down_with_source`.
    /// `Manual` overlays are never closed here.
    pub(crate) fn dismiss_anchored_overlays_outside(&mut self, point: vello::kurbo::Point) {
        for overlay in &self.popup_menu.presented_anchored_overlays {
            if overlay.dismissal == Dismissal::OutsideInteraction && !overlay.frame.contains(point)
            {
                overlay.is_presented.set(false);
            }
        }
    }

    /// The drawn frames of every presented anchored overlay, in hit order —
    /// the test harness reads these to assert the placement contract.
    #[cfg(test)]
    pub(crate) fn anchored_overlay_frames(&self) -> Vec<vello::kurbo::Rect> {
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
    pub(crate) fn render_anchored_overlays(&mut self, transform: vello::kurbo::Affine) {
        let registered = core::mem::take(&mut self.popup_menu.anchored_overlays);
        let last_presented = core::mem::take(&mut self.popup_menu.presented_anchored_overlays);
        let mut presented = Vec::with_capacity(last_presented.len());

        // An anchor that presented last frame but registered nothing this
        // frame left the tree: close it through its binding.
        for overlay in last_presented {
            if !registered
                .iter()
                .any(|entry| Rc::ptr_eq(&entry.marker, &overlay.marker))
            {
                overlay.is_presented.set(false);
            }
        }

        let window = self.window_bounds;
        for entry in registered {
            if !entry.presented {
                continue;
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
                Some((window.width() - margin).max(0.0) as f32),
                Some((window.height() - margin).max(0.0) as f32),
            );
            let (ideal, _stretch) = content.patch_and_measure(self, &entry.env, proposal);
            let overlay_size = waterui_core::layout::Size::new(
                ideal.width.min(window.width() as f32),
                ideal.height.min(window.height() as f32),
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

            // A press inside the overlay belongs to its own content, not the
            // page below — the same swallow the drawn context-menu panels
            // register.
            let depth = self.render_depth;
            let order = self.hit_test.next_hit_test_order();
            let frame = self.hit_test.clip_hit_bounds(frame);
            self.hit_test.pointer_targets.push(PointerTarget {
                bounds: frame,
                captures_drag: false,
                depth,
                order,
                press_slot: None,
                claim_owner: None,
                interaction: None,
                action: Rc::new(RefCell::new(
                    |_: &mut SemanticCore, _: vello::kurbo::Point, _: &Environment| false,
                )),
                keyboard_step: None,
                keyboard_focusable: false,
                modal: false,
            });

            content.flush_in_rect(
                self,
                RenderContext::with_transforms(window, transform, vello::kurbo::Affine::IDENTITY),
                &entry.env,
                bounded_proposal(frame),
                frame,
            );
            *entry.content.borrow_mut() = Some(content);

            let marker = entry.marker.clone();
            presented.push(PresentedAnchoredOverlay {
                marker,
                frame,
                dismissal: entry.dismissal,
                is_presented: entry.is_presented.clone(),
            });
        }
        self.popup_menu.presented_anchored_overlays = presented;
    }
}

fn kurbo_to_rect(rect: vello::kurbo::Rect) -> waterui_core::layout::Rect {
    waterui_core::layout::Rect::new(
        waterui_core::layout::Point::new(rect.x0 as f32, rect.y0 as f32),
        waterui_core::layout::Size::new(rect.width() as f32, rect.height() as f32),
    )
}

fn rect_to_kurbo(rect: waterui_core::layout::Rect) -> vello::kurbo::Rect {
    vello::kurbo::Rect::from_origin_size(
        vello::kurbo::Point::new(f64::from(rect.x()), f64::from(rect.y())),
        vello::kurbo::Size::new(f64::from(rect.width()), f64::from(rect.height())),
    )
}
