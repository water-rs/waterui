//! The `anchored_overlay` metadata: an overlay presented in window space
//! next to the view it wraps, above all other content.
//!
//! Mirrors `WuiAnchoredOverlay`: the system popovers cannot honour the
//! placement contract — neither offers edge alignment nor an exact `gap`
//! once the arrow is suppressed — so the overlay is presented in a
//! borderless surface positioned by [`place_anchored_overlay`]: an
//! `NSPanel` child window on macOS, a passthrough container in the host
//! `UIWindow` on iOS. All geometry the placement call sees is in the
//! window's top-left space; `AppKit`'s bottom-left space is mirrored in and
//! out.
//!
//! `is_presented` drives present/dismiss, `placed_edge` is written on every
//! placement, and `dismissal == OutsideInteraction` closes the overlay on a
//! pointer or touch down that lands outside it — the event itself still
//! reaches its target.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use cocoa_ui::geometry::Rect as KitRect;
use cocoa_ui::{PlatformView, Retained, view};
use waterui::metadata::anchored_overlay::{
    AnchorEdge, AnchorPlacement, AnchoredOverlay, Dismissal,
};
use waterui::reactive::{Binding, Signal};
use waterui_backend_core::overlay::{PhysicalEdge, logical_edge, place_anchored_overlay};
use waterui_core::layout::{
    LayoutDirection, Point, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions,
    layout_direction,
};
use waterui_core::{Computed, Metadata};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, Panel, window_of};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HitTest, HostView, window_of};

/// The leaf's live state: the mounted anchor child, the rendered overlay,
/// the bindings it syncs, and the surface it is presented in.
struct OverlayState {
    /// Proof of the main thread for kit calls inside callbacks.
    mtm: cocoa_ui::MainThreadMarker,
    /// The wrapper's own view — the anchor the overlay is placed against.
    host: Retained<HostView>,
    /// The anchor's content, laid out over the host's full bounds.
    child: Mounted,
    /// The overlay content, moved into the presentation surface while shown.
    overlay: NativeLeaf,
    /// Whether the overlay is presented; platform dismissal writes back.
    is_presented: Binding<bool>,
    /// The logical edge the last placement landed on, written here.
    placed_edge: Binding<AnchorEdge>,
    /// Where the overlay sits relative to the anchor.
    placement: AnchorPlacement,
    /// What besides the binding closes the overlay.
    dismissal: Dismissal,
    /// The environment's layout direction, for `Leading`/`Trailing`.
    direction: Computed<LayoutDirection>,
    /// Bound `true` before the anchor had a window; presented on attach.
    pending: Cell<bool>,
    /// The surface the overlay is currently presented in.
    presentation: RefCell<Option<Presentation>>,
    /// The window-resize and anchor-frame observers, live while the anchor
    /// is in a window: the overlay follows the window's own geometry
    /// changes. `UIKit` re-drives placement through the layout handler and
    /// needs neither.
    #[cfg(target_os = "macos")]
    watchers: RefCell<
        Option<(
            cocoa_ui::notification::NotificationObserver,
            cocoa_ui::notification::NotificationObserver,
        )>,
    >,
    /// The overlay's frame in window space, for the host's hit test (iOS).
    #[cfg(target_os = "ios")]
    overlay_frame: Cell<KitRect>,
}

impl core::fmt::Debug for OverlayState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OverlayState").finish_non_exhaustive()
    }
}

/// The `NSPanel` presentation: the panel plus its outside-click monitor.
#[cfg(target_os = "macos")]
struct Presentation {
    /// The borderless child window holding the overlay while presented.
    panel: Panel,
    /// Observes clicks that land outside the panel; does not consume them.
    _monitor: cocoa_ui::appkit::event::LocalEventMonitor,
}

#[cfg(target_os = "macos")]
impl Drop for Presentation {
    fn drop(&mut self) {
        self.panel.detach();
    }
}

/// The passthrough container covering the host `UIWindow` (iOS).
#[cfg(target_os = "ios")]
struct Presentation {
    /// The window-covering container holding the overlay.
    host: Retained<HostView>,
}

#[cfg(target_os = "ios")]
impl Drop for Presentation {
    fn drop(&mut self) {
        let host: &PlatformView = &self.host;
        view::remove_from_superview(host);
    }
}

/// A `cocoa_ui` rect as the placement contract's top-left-space rect — the
/// layout contract is f32; window geometry always fits.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; window geometry always fits"
)]
const fn to_core(rect: KitRect) -> Rect {
    Rect::new(
        Point::new(rect.origin.x as f32, rect.origin.y as f32),
        Size::new(rect.size.width as f32, rect.size.height as f32),
    )
}

/// A contract rect back in kit space.
fn to_kit(rect: Rect) -> KitRect {
    KitRect::new(
        f64::from(rect.x()),
        f64::from(rect.y()),
        f64::from(rect.width()),
        f64::from(rect.height()),
    )
}

/// `rect` mirrored about `container`'s horizontal midline — the transform
/// between `AppKit`'s bottom-left window space and the contract's top-left
/// space. Self-inverse.
#[cfg(target_os = "macos")]
fn mirror(rect: KitRect, container: KitRect) -> KitRect {
    KitRect::new(
        rect.origin.x,
        container.origin.y + container.size.height - (rect.origin.y + rect.size.height),
        rect.size.width,
        rect.size.height,
    )
}

/// The overlay's ideal size bounded by the surface it presents in —
/// `measureOverlay`: `sizeThatFits` against the container, clamped to it.
fn measure_overlay(state: &OverlayState, container: Size) -> Size {
    let ideal = state
        .overlay
        .layout()
        .measure(ProposalSize::new(
            Some(container.width),
            Some(container.height),
        ))
        .size;
    Size::new(
        ideal.width.min(container.width),
        ideal.height.min(container.height),
    )
}

/// Writes the resolved physical edge back as the logical `AnchorEdge` the
/// binding reports — the `placed_edge` half of every placement.
fn report_edge(state: &OverlayState, edge: PhysicalEdge, direction: LayoutDirection) {
    let logical = logical_edge(edge, direction);
    if state.placed_edge.snapshot() != logical {
        state.placed_edge.set(logical);
    }
}

/// Present or dismiss per the binding once the anchor is on a window, and
/// close the overlay when the anchor leaves the tree —
/// `attachmentChanged`.
fn attachment_changed(state: &Rc<OverlayState>) {
    if view::has_window(&state.host) {
        if state.is_presented.snapshot() || state.pending.get() {
            state.pending.set(false);
            present(state);
        }
        #[cfg(target_os = "macos")]
        observe_window_and_frame(state);
    } else {
        // The anchor left the tree: the overlay closes with it.
        state.pending.set(false);
        if state.is_presented.snapshot() {
            state.is_presented.set(false);
        }
        dismiss(state);
        #[cfg(target_os = "macos")]
        state.watchers.borrow_mut().take();
    }
}

/// `dismissOverlay`: the presentation surface releases the overlay's view
/// back to the leaf.
fn dismiss(state: &Rc<OverlayState>) {
    state.presentation.borrow_mut().take();
    view::remove_from_superview(state.overlay.view());
}

// MARK: - iOS presentation

/// `presentOverlay`, iOS: a full-window passthrough container that forwards
/// every hit that is not on the overlay to the content below — the same
/// touch that dismisses the overlay still reaches its target.
#[cfg(target_os = "ios")]
fn present(state: &Rc<OverlayState>) {
    let Some(window) = window_of(&state.host) else {
        state.pending.set(true);
        return;
    };
    if state.presentation.borrow().is_none() {
        let host = HostView::new(state.mtm, view::bounds(&window));
        let host_view: &PlatformView = &host;
        view::set_autoresizing_flexible_size(host_view);
        host.set_hit_test_handler({
            let state = Rc::downgrade(state);
            move |_, point| {
                let Some(state) = state.upgrade() else {
                    return HitTest::Pass;
                };
                let frame = state.overlay_frame.get();
                let inside = point.x >= frame.origin.x
                    && point.x <= frame.origin.x + frame.size.width
                    && point.y >= frame.origin.y
                    && point.y <= frame.origin.y + frame.size.height;
                if inside {
                    // The overlay's deepest hit wins; a point inside the
                    // frame but on no view passes through, as
                    // `hitTest`'s `nil` did.
                    HitTest::PassIfSelf
                } else {
                    if state.dismissal == Dismissal::OutsideInteraction {
                        state.is_presented.set(false);
                    }
                    HitTest::Pass
                }
            }
        });
        let window_view: &PlatformView = &window;
        view::add_subview(window_view, host_view);
        view::add_subview(host_view, state.overlay.view());
        *state.presentation.borrow_mut() = Some(Presentation { host });
    }
    reposition(state);
}

/// `repositionOverlay`, iOS: place against the window's own bounds; the
/// window's top-left space is already the contract's.
#[cfg(target_os = "ios")]
fn reposition(state: &Rc<OverlayState>) {
    if state.presentation.borrow().is_none() {
        return;
    }
    let Some(window) = window_of(&state.host) else {
        return;
    };
    let window_view: &PlatformView = &window;
    let container = to_core(view::bounds(window_view));
    let size = measure_overlay(state, *container.size());
    let anchor = to_core(view::bounds_in_window(&state.host));
    let direction = state.direction.snapshot();
    let placed = place_anchored_overlay(anchor, container, size, state.placement, direction);
    let frame = to_kit(placed.frame);
    state.overlay_frame.set(frame);
    view::set_frame(state.overlay.view(), frame);
    report_edge(state, placed.edge, direction);
}

// MARK: - macOS presentation

/// `presentOverlay`, macOS: the borderless child `NSPanel` and the
/// outside-click monitor that closes an `OutsideInteraction` overlay.
#[cfg(target_os = "macos")]
fn present(state: &Rc<OverlayState>) {
    let Some(parent) = window_of(&state.host) else {
        state.pending.set(true);
        return;
    };
    if state.presentation.borrow().is_none() {
        let panel = Panel::new(state.mtm);
        panel.attach(&parent);
        panel.set_content(state.overlay.view());
        let monitor = cocoa_ui::appkit::event::on_mouse_down({
            let state = Rc::downgrade(state);
            move |event_window| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                let presentation = state.presentation.borrow();
                let inside = presentation
                    .as_ref()
                    .is_some_and(|shown| shown.panel.is_window(event_window));
                drop(presentation);
                if !inside && state.dismissal == Dismissal::OutsideInteraction {
                    state.is_presented.set(false);
                }
            }
        });
        *state.presentation.borrow_mut() = Some(Presentation {
            panel,
            _monitor: monitor,
        });
    }
    reposition(state);
}

/// `repositionOverlay`, macOS: window space is bottom-left, so the anchor
/// and the placed frame are mirrored about the content area's height on
/// the way in and out; the panel takes a screen-space frame.
#[cfg(target_os = "macos")]
fn reposition(state: &Rc<OverlayState>) {
    let presentation = state.presentation.borrow();
    let Some(parent) = window_of(&state.host) else {
        return;
    };
    let container = cocoa_ui::appkit::content_bounds(&parent);
    let size = measure_overlay(state, *to_core(container).size());
    let anchor = to_core(mirror(view::bounds_in_window(&state.host), container));
    let direction = state.direction.snapshot();
    let placed =
        place_anchored_overlay(anchor, to_core(container), size, state.placement, direction);
    let frame_window = mirror(to_kit(placed.frame), container);
    if let Some(shown) = presentation.as_ref() {
        shown
            .panel
            .set_frame(cocoa_ui::appkit::convert_to_screen(&parent, frame_window));
    }
    report_edge(state, placed.edge, direction);
}

/// `observeWindowAndFrame`: the overlay re-places when the parent window
/// resizes and follows the anchor's own frame changes.
#[cfg(target_os = "macos")]
fn observe_window_and_frame(state: &Rc<OverlayState>) {
    if state.watchers.borrow().is_some() {
        return;
    }
    let Some(parent) = window_of(&state.host) else {
        return;
    };
    let resize = cocoa_ui::notification::observe_object(
        state.mtm,
        &cocoa_ui::appkit::did_resize_notification(),
        parent.as_ref(),
        {
            let state = Rc::downgrade(state);
            move || {
                if let Some(state) = state.upgrade() {
                    reposition(&state);
                }
            }
        },
    );
    view::set_posts_frame_changed(&state.host, true);
    let host = view::retain_base(&state.host);
    let frame = cocoa_ui::notification::observe_object(
        state.mtm,
        &cocoa_ui::appkit::view_frame_did_change_notification(),
        host.as_ref(),
        {
            let state = Rc::downgrade(state);
            move || {
                if let Some(state) = state.upgrade() {
                    reposition(&state);
                }
            }
        },
    );
    *state.watchers.borrow_mut() = Some((resize, frame));
}

/// The wrapper's layout face: transparent, every answer the child's —
/// `WuiAnchoredOverlay` forwards `stretchAxis`, `layoutPriority` and
/// `measure` unchanged.
struct AnchoredSubView {
    state: Rc<OverlayState>,
}

impl core::fmt::Debug for AnchoredSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnchoredSubView").finish_non_exhaustive()
    }
}

impl SubView for AnchoredSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }
}

/// Installs the `anchored_overlay` handler on the dispatcher:
/// `Metadata<AnchoredOverlay>` maps to a transparent container that
/// presents the overlay content in a window-level surface against the
/// anchor's frame.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<AnchoredOverlay>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, KitRect::ZERO);
        let host_view: &PlatformView = &host;
        let mounted = ctx.render(metadata.content).mount(host_view);
        crate::primary_content::forward(&host, mounted.view());
        let overlay = ctx.render(metadata.value.content);

        let state = Rc::new(OverlayState {
            mtm,
            host: host.clone(),
            child: mounted,
            overlay,
            is_presented: metadata.value.is_presented,
            placed_edge: metadata.value.placed_edge,
            placement: metadata.value.placement,
            dismissal: metadata.value.dismissal,
            direction: layout_direction(ctx.env()),
            pending: Cell::new(false),
            presentation: RefCell::new(None),
            #[cfg(target_os = "macos")]
            watchers: RefCell::new(None),
            #[cfg(target_os = "ios")]
            overlay_frame: Cell::new(KitRect::ZERO),
        });

        // `layout`/`layoutSubviews`: the content fills the host and the
        // overlay follows the anchor's new position.
        host.set_layout_handler({
            let child = view::retain_base(state.child.view());
            let state = Rc::clone(&state);
            move |view| {
                view::set_frame(&child, view::bounds(view));
                reposition(&state);
            }
        });
        host.set_window_handler({
            let state = Rc::clone(&state);
            move |_| attachment_changed(&state)
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            AnchoredSubView {
                state: Rc::clone(&state),
            },
        );

        // The objects the watchers fire on are kept first so the guards
        // — kept after — drop before them (reverse insertion order).
        leaf.keep(Rc::clone(&state));
        leaf.watch(&state.is_presented, {
            let state = Rc::downgrade(&state);
            move |ctx| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                if *ctx.value() {
                    present(&state);
                } else {
                    dismiss(&state);
                }
            }
        });

        // An initially-presented overlay before the window exists is
        // replayed on attach, as `presentOverlay`'s pending path did.
        if state.is_presented.snapshot() {
            present(&state);
        }

        leaf
    });
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    /// Mirroring is an involution: a rect mirrored twice is itself.
    #[cfg(target_os = "macos")]
    #[test]
    fn mirror_is_self_inverse() {
        let container = KitRect::new(0.0, 0.0, 800.0, 600.0);
        let rect = KitRect::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!(mirror(mirror(rect, container), container), rect);
    }

    /// The f32/f64 conversion round-trips a rect between kit and contract
    /// space unchanged.
    #[test]
    fn kit_and_core_rects_round_trip() {
        let kit = KitRect::new(1.5, -2.0, 100.25, 64.0);
        let back = to_kit(to_core(kit));
        assert_eq!(back, kit);
    }

    /// A point at `AppKit`'s bottom edge becomes the contract's bottom:
    /// a rect hugging the container's top in top-left space lands at the
    /// bottom of bottom-left space.
    #[cfg(target_os = "macos")]
    #[test]
    fn mirror_flips_vertical_edges() {
        let container = KitRect::new(0.0, 0.0, 800.0, 600.0);
        let top_band = KitRect::new(0.0, 0.0, 800.0, 40.0);
        let mirrored = mirror(top_band, container);
        assert_eq!(mirrored.origin.y, 560.0);
    }
}
